//! CoreWorker - Main worker object for FFI.
//!
//! The CoreWorker handles worker lifecycle, polling for activations,
//! and processing completions.

use std::sync::Arc;

use flovyn_sdk_core::client::{
    oauth2, TaskExecutionClient, WorkerLifecycleClient, WorkerType, WorkflowDispatch,
};
use flovyn_sdk_core::task::TaskMetadata;
use flovyn_sdk_core::workflow::WorkflowMetadata;
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::activation::{
    StateEntry, TaskActivation, TaskCompletion, WorkflowActivation, WorkflowActivationJob,
    WorkflowCompletionStatus,
};
use crate::config::WorkerConfig;
use crate::context::FfiWorkflowContext;
use crate::error::FfiError;
use crate::types::{FfiEventType, FfiReplayEvent};

/// The main worker object exposed to foreign languages via FFI.
///
/// CoreWorker manages:
/// - Connection to the Flovyn server
/// - Worker registration
/// - Polling for workflow and task activations
/// - Processing completions
#[derive(uniffi::Object)]
pub struct CoreWorker {
    /// gRPC channel
    channel: Channel,
    /// Worker token
    worker_token: String,
    /// Worker configuration
    config: WorkerConfig,
    /// Server-assigned worker ID after registration
    worker_id: Mutex<Option<Uuid>>,
    /// Tokio runtime for async operations
    runtime: Runtime,
    /// Shutdown flag
    shutdown_requested: std::sync::atomic::AtomicBool,
}

#[uniffi::export]
impl CoreWorker {
    /// Create a new CoreWorker with the given configuration.
    ///
    /// This establishes a connection to the Flovyn server but does not
    /// register the worker yet. Call `register()` to register.
    ///
    /// Authentication priority:
    /// 1. OAuth2 credentials (if provided, fetches JWT token)
    /// 2. Worker token (if provided)
    /// 3. Placeholder token (for testing)
    #[uniffi::constructor]
    pub fn new(config: WorkerConfig) -> Result<Arc<Self>, FfiError> {
        // Create a Tokio runtime
        let runtime = Runtime::new().map_err(|e| FfiError::Other {
            msg: format!("Failed to create runtime: {}", e),
        })?;

        // Connect to the server
        let channel: Channel = runtime.block_on(async {
            Channel::from_shared(config.server_url.clone())
                .map_err(|e| FfiError::InvalidConfiguration {
                    msg: format!("Invalid server URL: {}", e),
                })?
                .connect()
                .await
                .map_err(|e| FfiError::Grpc {
                    msg: format!("Failed to connect: {}", e),
                    code: 14, // UNAVAILABLE
                })
        })?;

        // Determine authentication token
        // Priority: OAuth2 credentials > worker_token > placeholder
        let worker_token = if let Some(oauth2_creds) = &config.oauth2_credentials {
            // Fetch token using OAuth2 client credentials flow
            let core_creds = oauth2::OAuth2Credentials::new(
                &oauth2_creds.client_id,
                &oauth2_creds.client_secret,
                &oauth2_creds.token_endpoint,
            );
            let core_creds = if let Some(scopes) = &oauth2_creds.scopes {
                core_creds.with_scopes(scopes.split_whitespace().map(String::from).collect())
            } else {
                core_creds
            };

            runtime.block_on(async {
                oauth2::fetch_access_token(&core_creds)
                    .await
                    .map(|r| r.access_token)
                    .map_err(|e| FfiError::Other {
                        msg: format!("OAuth2 token fetch failed: {}", e),
                    })
            })?
        } else if let Some(token) = &config.worker_token {
            token.clone()
        } else {
            format!("fwt_placeholder-{}", Uuid::new_v4())
        };

        Ok(Arc::new(Self {
            channel,
            worker_token,
            config,
            worker_id: Mutex::new(None),
            runtime,
            shutdown_requested: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    /// Register the worker with the Flovyn server.
    ///
    /// This registers all workflow and task kinds specified in the configuration.
    /// Returns the server-assigned worker ID.
    pub fn register(&self) -> Result<String, FfiError> {
        // Convert FFI metadata to core WorkflowMetadata with schemas
        let workflows: Vec<WorkflowMetadata> = self
            .config
            .workflow_metadata
            .iter()
            .map(|m| {
                let mut metadata = WorkflowMetadata::new(&m.kind)
                    .with_name(&m.name)
                    .with_cancellable(m.cancellable)
                    .with_tags(m.tags.clone());

                if let Some(desc) = &m.description {
                    metadata = metadata.with_description(desc);
                }
                if let Some(version) = &m.version {
                    metadata = metadata.with_version(version);
                }
                if let Some(timeout) = m.timeout_seconds {
                    metadata = metadata.with_timeout_seconds(timeout as u64);
                }
                // Parse and set schemas from JSON strings
                if let Some(schema_str) = &m.input_schema {
                    if let Ok(schema) = serde_json::from_str(schema_str) {
                        metadata = metadata.with_input_schema(schema);
                    }
                }
                if let Some(schema_str) = &m.output_schema {
                    if let Ok(schema) = serde_json::from_str(schema_str) {
                        metadata = metadata.with_output_schema(schema);
                    }
                }
                metadata
            })
            .collect();

        // Convert FFI metadata to core TaskMetadata
        let tasks: Vec<TaskMetadata> = self
            .config
            .task_metadata
            .iter()
            .map(|m| TaskMetadata::new(&m.kind))
            .collect();

        let mut lifecycle_client =
            WorkerLifecycleClient::new(self.channel.clone(), &self.worker_token);

        // Determine worker type based on registered capabilities
        let worker_type = match (workflows.is_empty(), tasks.is_empty()) {
            (false, false) => WorkerType::Unified,
            (false, true) => WorkerType::Workflow,
            (true, false) => WorkerType::Task,
            (true, true) => WorkerType::Unified, // Empty worker, register as unified
        };

        let worker_name = self
            .config
            .worker_identity
            .clone()
            .unwrap_or_else(|| format!("ffi-worker-{}", &self.config.queue));

        // Get tenant ID from config
        let tenant_id = Uuid::parse_str(&self.config.tenant_id).unwrap_or_else(|_| Uuid::nil());

        let result = self.runtime.block_on(async {
            lifecycle_client
                .register_worker(
                    &worker_name,
                    "0.1.0",
                    worker_type,
                    tenant_id,
                    None, // No space_id
                    workflows,
                    tasks,
                )
                .await
        })?;

        if !result.success {
            return Err(FfiError::Other {
                msg: result
                    .error
                    .unwrap_or_else(|| "Registration failed".to_string()),
            });
        }

        *self.worker_id.lock() = Some(result.worker_id);
        Ok(result.worker_id.to_string())
    }

    /// Poll for the next workflow activation.
    ///
    /// This blocks until work is available or the worker is shut down.
    /// Returns `None` if no work is available within the timeout.
    ///
    /// The returned activation includes a replay-aware context that handles:
    /// - Determinism validation during replay
    /// - Cached result return for replayed operations
    /// - Command generation for new operations
    pub fn poll_workflow_activation(&self) -> Result<Option<WorkflowActivation>, FfiError> {
        if self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(FfiError::ShuttingDown);
        }

        let worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or(FfiError::Other {
                msg: "Worker not registered".to_string(),
            })?
        };

        let tenant_id = Uuid::parse_str(&self.config.tenant_id).unwrap_or_else(|_| Uuid::nil());

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        let result = self.runtime.block_on(async {
            dispatch_client
                .poll_workflow(
                    &worker_id.to_string(),
                    &self.config.tenant_id,
                    &self.config.queue,
                    std::time::Duration::from_secs(30),
                )
                .await
        })?;

        match result {
            Some(workflow_info) => {
                // Get workflow events for replay
                let events = self.runtime.block_on(async {
                    dispatch_client
                        .get_events(workflow_info.id, None)
                        .await
                        .unwrap_or_default()
                });

                // Convert events to FFI format
                let ffi_events: Vec<FfiReplayEvent> = events
                    .into_iter()
                    .map(|e| FfiReplayEvent {
                        sequence_number: e.sequence,
                        event_type: parse_event_type(&e.event_type),
                        data: serde_json::to_vec(&e.payload).unwrap_or_default(),
                        timestamp_ms: 0, // Timestamp not available in WorkflowEvent
                    })
                    .collect();

                // Get workflow state
                let state_entries: Vec<(String, Vec<u8>)> = vec![]; // TODO: Fetch from workflow state

                // Determine if cancellation was requested
                let cancellation_requested = false; // TODO: Check from workflow info or events

                // Create the replay-aware context
                let context = FfiWorkflowContext::new(
                    workflow_info.id,
                    tenant_id,
                    workflow_info.workflow_task_time_millis,
                    workflow_info.id.as_u128() as u64, // Use workflow ID as random seed
                    ffi_events,
                    state_entries,
                    cancellation_requested,
                );

                // Build jobs (signals, queries, cancellation)
                let jobs = vec![WorkflowActivationJob::Initialize {
                    input: serde_json::to_vec(&workflow_info.input).unwrap_or_default(),
                }];

                let activation = WorkflowActivation {
                    context,
                    workflow_kind: workflow_info.kind.clone(),
                    input: serde_json::to_vec(&workflow_info.input).unwrap_or_default(),
                    jobs,
                };

                Ok(Some(activation))
            }
            None => Ok(None),
        }
    }

    /// Complete a workflow activation.
    ///
    /// This extracts commands from the context and sends them along with
    /// the completion status to the server.
    ///
    /// # Arguments
    ///
    /// * `context` - The workflow context (commands are extracted from this)
    /// * `status` - The completion status (Completed, Suspended, Cancelled, or Failed)
    pub fn complete_workflow_activation(
        &self,
        context: &FfiWorkflowContext,
        status: WorkflowCompletionStatus,
    ) -> Result<(), FfiError> {
        let _worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or(FfiError::Other {
                msg: "Worker not registered".to_string(),
            })?
        };

        let workflow_execution_id =
            Uuid::parse_str(&context.workflow_execution_id()).map_err(|_| {
                FfiError::InvalidConfiguration {
                    msg: "Invalid workflow execution ID".to_string(),
                }
            })?;

        // Extract commands from the context
        let commands = context.take_commands();

        // Convert FFI commands to proto commands for gRPC submission
        let proto_commands: Vec<flovyn_sdk_core::generated::flovyn_v1::WorkflowCommand> = commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| cmd.to_proto_command(i as i32 + 1))
            .collect();

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        // Submit based on status
        match status {
            WorkflowCompletionStatus::Completed { output } => {
                // Add CompleteWorkflow command
                let output_value: serde_json::Value =
                    serde_json::from_slice(&output).unwrap_or(serde_json::Value::Null);
                self.runtime.block_on(async {
                    dispatch_client
                        .complete_workflow(workflow_execution_id, output_value, proto_commands)
                        .await
                })?;
            }
            WorkflowCompletionStatus::Suspended => {
                self.runtime.block_on(async {
                    dispatch_client
                        .suspend_workflow(workflow_execution_id, proto_commands)
                        .await
                })?;
            }
            WorkflowCompletionStatus::Cancelled { reason } => {
                // Use fail_workflow with "CANCELLED" failure type
                self.runtime.block_on(async {
                    dispatch_client
                        .fail_workflow(workflow_execution_id, &reason, "CANCELLED", proto_commands)
                        .await
                })?;
            }
            WorkflowCompletionStatus::Failed { error } => {
                self.runtime.block_on(async {
                    dispatch_client
                        .fail_workflow(workflow_execution_id, &error, "ERROR", proto_commands)
                        .await
                })?;
            }
        }

        Ok(())
    }

    /// Poll for the next task activation.
    ///
    /// This blocks until work is available or the worker is shut down.
    pub fn poll_task_activation(&self) -> Result<Option<TaskActivation>, FfiError> {
        if self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(FfiError::ShuttingDown);
        }

        let worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or(FfiError::Other {
                msg: "Worker not registered".to_string(),
            })?
        };

        let mut task_client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);

        let result = self.runtime.block_on(async {
            task_client
                .poll_task(
                    &worker_id.to_string(),
                    &self.config.tenant_id,
                    &self.config.queue,
                    std::time::Duration::from_secs(30),
                )
                .await
        })?;

        match result {
            Some(task_info) => {
                let activation = TaskActivation {
                    task_execution_id: task_info.id.to_string(),
                    task_kind: task_info.task_type.clone(),
                    input: serde_json::to_vec(&task_info.input).unwrap_or_default(),
                    workflow_execution_id: task_info.workflow_execution_id.map(|id| id.to_string()),
                    attempt: task_info.execution_count,
                    max_retries: 3,   // Default, would come from task metadata
                    timeout_ms: None, // Not available in TaskExecutionInfo
                };
                Ok(Some(activation))
            }
            None => Ok(None),
        }
    }

    /// Complete a task activation.
    ///
    /// This sends the task result back to the server.
    pub fn complete_task(&self, completion: TaskCompletion) -> Result<(), FfiError> {
        let _worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or(FfiError::Other {
                msg: "Worker not registered".to_string(),
            })?
        };

        let task_execution_id = Uuid::parse_str(completion.task_execution_id()).map_err(|_| {
            FfiError::InvalidConfiguration {
                msg: "Invalid task execution ID".to_string(),
            }
        })?;

        let mut task_client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);

        match completion {
            TaskCompletion::Completed { output, .. } => {
                let output_value: serde_json::Value =
                    serde_json::from_slice(&output).unwrap_or(serde_json::Value::Null);
                self.runtime.block_on(async {
                    task_client
                        .complete_task(task_execution_id, output_value)
                        .await
                })?;
            }
            TaskCompletion::Failed { error, .. } => {
                self.runtime
                    .block_on(async { task_client.fail_task(task_execution_id, &error).await })?;
            }
            TaskCompletion::Cancelled { .. } => {
                // Mark as cancelled - simplified
                self.runtime.block_on(async {
                    task_client.fail_task(task_execution_id, "Cancelled").await
                })?;
            }
        }

        Ok(())
    }

    /// Initiate graceful shutdown of the worker.
    ///
    /// This signals that the worker should stop polling for new work
    /// and finish any in-flight tasks.
    pub fn initiate_shutdown(&self) {
        self.shutdown_requested
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check if shutdown has been requested.
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get the current worker status as a string.
    pub fn get_status(&self) -> String {
        let worker_id = self.worker_id.lock();
        if self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            "shutting_down".to_string()
        } else if worker_id.is_some() {
            "running".to_string()
        } else {
            "initializing".to_string()
        }
    }
}

/// Helper struct for workflow state entries
impl StateEntry {
    /// Create a new state entry
    pub fn new(key: String, value: Vec<u8>) -> Self {
        Self { key, value }
    }
}

/// Parse event type string to FfiEventType.
fn parse_event_type(event_type: &str) -> FfiEventType {
    match event_type {
        "WORKFLOW_STARTED" => FfiEventType::WorkflowStarted,
        "WORKFLOW_COMPLETED" => FfiEventType::WorkflowCompleted,
        "WORKFLOW_EXECUTION_FAILED" => FfiEventType::WorkflowExecutionFailed,
        "WORKFLOW_SUSPENDED" => FfiEventType::WorkflowSuspended,
        "CANCELLATION_REQUESTED" => FfiEventType::CancellationRequested,
        "OPERATION_COMPLETED" => FfiEventType::OperationCompleted,
        "STATE_SET" => FfiEventType::StateSet,
        "STATE_CLEARED" => FfiEventType::StateCleared,
        "TASK_SCHEDULED" => FfiEventType::TaskScheduled,
        "TASK_COMPLETED" => FfiEventType::TaskCompleted,
        "TASK_FAILED" => FfiEventType::TaskFailed,
        "TASK_CANCELLED" => FfiEventType::TaskCancelled,
        "PROMISE_CREATED" => FfiEventType::PromiseCreated,
        "PROMISE_RESOLVED" => FfiEventType::PromiseResolved,
        "PROMISE_REJECTED" => FfiEventType::PromiseRejected,
        "PROMISE_TIMEOUT" => FfiEventType::PromiseTimeout,
        "CHILD_WORKFLOW_INITIATED" => FfiEventType::ChildWorkflowInitiated,
        "CHILD_WORKFLOW_STARTED" => FfiEventType::ChildWorkflowStarted,
        "CHILD_WORKFLOW_COMPLETED" => FfiEventType::ChildWorkflowCompleted,
        "CHILD_WORKFLOW_FAILED" => FfiEventType::ChildWorkflowFailed,
        "CHILD_WORKFLOW_CANCELLED" => FfiEventType::ChildWorkflowCancelled,
        "TIMER_STARTED" => FfiEventType::TimerStarted,
        "TIMER_FIRED" => FfiEventType::TimerFired,
        "TIMER_CANCELLED" => FfiEventType::TimerCancelled,
        // Default to OperationCompleted for unknown types
        _ => FfiEventType::OperationCompleted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_entry_new() {
        let entry = StateEntry::new("key".to_string(), b"value".to_vec());
        assert_eq!(entry.key, "key");
        assert_eq!(entry.value, b"value".to_vec());
    }
}
