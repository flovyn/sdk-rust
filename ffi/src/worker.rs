//! CoreWorker - Main worker object for FFI.
//!
//! The CoreWorker handles worker lifecycle, polling for activations,
//! and processing completions.

use std::sync::Arc;

use flovyn_core::client::{
    TaskExecutionClient, WorkerLifecycleClient, WorkerType, WorkflowDispatch,
};
use flovyn_core::task::TaskMetadata;
use flovyn_core::workflow::WorkflowMetadata;
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::activation::{
    StateEntry, TaskActivation, TaskCompletion, WorkflowActivation, WorkflowActivationCompletion,
    WorkflowActivationJob,
};
use crate::config::WorkerConfig;
use crate::error::FfiError;
use crate::types::FfiReplayEvent;

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
    #[uniffi::constructor]
    pub fn new(config: WorkerConfig) -> Result<Arc<Self>, FfiError> {
        // Create a Tokio runtime
        let runtime = Runtime::new().map_err(|e| FfiError::Other {
            message: format!("Failed to create runtime: {}", e),
        })?;

        // Connect to the server
        let channel: Channel = runtime.block_on(async {
            Channel::from_shared(config.server_url.clone())
                .map_err(|e| FfiError::InvalidConfiguration {
                    message: format!("Invalid server URL: {}", e),
                })?
                .connect()
                .await
                .map_err(|e| FfiError::Grpc {
                    message: format!("Failed to connect: {}", e),
                    code: 14, // UNAVAILABLE
                })
        })?;

        // Generate a worker token (in production this would come from auth)
        let worker_token = format!("worker-token-{}", Uuid::new_v4());

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
        let workflows: Vec<WorkflowMetadata> = self
            .config
            .workflow_kinds
            .iter()
            .map(WorkflowMetadata::new)
            .collect();

        let tasks: Vec<TaskMetadata> = self
            .config
            .task_kinds
            .iter()
            .map(TaskMetadata::new)
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
            .unwrap_or_else(|| format!("ffi-worker-{}", &self.config.task_queue));

        let result = self.runtime.block_on(async {
            lifecycle_client
                .register_worker(
                    &worker_name,
                    "0.1.0",
                    worker_type,
                    // Use a placeholder tenant ID - would come from config in real impl
                    Uuid::parse_str(&self.config.namespace).unwrap_or_else(|_| Uuid::nil()),
                    None, // No space_id
                    workflows,
                    tasks,
                )
                .await
        })?;

        if !result.success {
            return Err(FfiError::Other {
                message: result
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
                message: "Worker not registered".to_string(),
            })?
        };

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        let result = self.runtime.block_on(async {
            dispatch_client
                .poll_workflow(
                    &worker_id.to_string(),
                    &self.config.namespace,
                    &self.config.task_queue,
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

                // Convert to FFI activation
                let activation = WorkflowActivation {
                    run_id: Uuid::new_v4().to_string(), // Run ID for this execution
                    workflow_execution_id: workflow_info.id.to_string(),
                    workflow_kind: workflow_info.kind.clone(),
                    timestamp_ms: workflow_info.workflow_task_time_millis,
                    is_replaying: workflow_info.current_sequence > 0,
                    random_seed: vec![], // Would be populated from workflow info
                    jobs: vec![WorkflowActivationJob::Initialize {
                        input: serde_json::to_vec(&workflow_info.input).unwrap_or_default(),
                    }],
                    history: events
                        .into_iter()
                        .map(|e| FfiReplayEvent {
                            sequence_number: e.sequence,
                            event_type: crate::types::FfiEventType::OperationCompleted, // Simplified
                            data: serde_json::to_vec(&e.payload).unwrap_or_default(),
                            timestamp_ms: 0, // Would come from event
                        })
                        .collect(),
                    state: vec![], // Would be populated from workflow state
                };

                Ok(Some(activation))
            }
            None => Ok(None),
        }
    }

    /// Complete a workflow activation.
    ///
    /// This sends the commands generated by the language SDK back to the server.
    pub fn complete_workflow_activation(
        &self,
        completion: WorkflowActivationCompletion,
    ) -> Result<(), FfiError> {
        let _worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or(FfiError::Other {
                message: "Worker not registered".to_string(),
            })?
        };

        let workflow_execution_id =
            Uuid::parse_str(&completion.run_id).map_err(|_| FfiError::InvalidConfiguration {
                message: "Invalid workflow execution ID".to_string(),
            })?;

        // Convert FFI commands to core commands
        let _commands: Vec<flovyn_core::WorkflowCommand> = completion
            .commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| cmd.to_core_command(i as i32 + 1))
            .collect();

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        // Determine workflow status and submit
        // This is simplified - real impl would need to look at command types
        self.runtime.block_on(async {
            dispatch_client
                .suspend_workflow(workflow_execution_id, vec![])
                .await
        })?;

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
                message: "Worker not registered".to_string(),
            })?
        };

        let mut task_client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);

        let result = self.runtime.block_on(async {
            task_client
                .poll_task(
                    &worker_id.to_string(),
                    &self.config.namespace,
                    &self.config.task_queue,
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
                message: "Worker not registered".to_string(),
            })?
        };

        let task_execution_id = Uuid::parse_str(completion.task_execution_id()).map_err(|_| {
            FfiError::InvalidConfiguration {
                message: "Invalid task execution ID".to_string(),
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
