//! CoreWorker - Main worker object for FFI.
//!
//! The CoreWorker handles worker lifecycle, polling for activations,
//! and processing completions.

use std::sync::Arc;

use flovyn_worker_core::client::{
    oauth2, TaskExecutionClient, WorkerLifecycleClient, WorkerType, WorkflowDispatch,
};
use flovyn_worker_core::task::TaskMetadata;
use flovyn_worker_core::workflow::WorkflowMetadata;
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::activation::{
    StateEntry, TaskActivation, TaskCompletion, WorkflowActivation, WorkflowActivationJob,
    WorkflowCompletionStatus,
};
use crate::config::WorkerConfig;
use crate::context::{FfiTaskContext, FfiWorkflowContext};
use crate::error::FfiError;
use crate::types::{FfiEventType, FfiReplayEvent};

/// Lifecycle event for worker status changes.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LifecycleEvent {
    /// Event name (e.g., "starting", "registered", "ready", "paused", "resumed", "stopped")
    pub event_name: String,
    /// Timestamp in milliseconds since Unix epoch
    pub timestamp_ms: i64,
    /// Optional additional data as JSON string
    pub data: Option<String>,
}

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
    /// Tokio runtime for async operations (Arc for sharing with task contexts)
    runtime: Arc<Runtime>,
    /// Shutdown flag
    shutdown_requested: std::sync::atomic::AtomicBool,
    /// Worker start time in milliseconds since Unix epoch
    started_at_ms: std::sync::atomic::AtomicI64,
    /// Paused flag
    paused: std::sync::atomic::AtomicBool,
    /// Pause reason (when paused)
    pause_reason: Mutex<Option<String>>,
    /// Lifecycle events queue for polling
    lifecycle_events: Mutex<Vec<LifecycleEvent>>,
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
        use std::time::{SystemTime, UNIX_EPOCH};

        // Create a Tokio runtime
        let runtime = Arc::new(Runtime::new().map_err(|e| FfiError::Other {
            msg: format!("Failed to create runtime: {}", e),
        })?);

        // Get start time in milliseconds
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

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

        let worker = Arc::new(Self {
            channel,
            worker_token,
            config,
            worker_id: Mutex::new(None),
            runtime,
            shutdown_requested: std::sync::atomic::AtomicBool::new(false),
            started_at_ms: std::sync::atomic::AtomicI64::new(started_at_ms),
            paused: std::sync::atomic::AtomicBool::new(false),
            pause_reason: Mutex::new(None),
            lifecycle_events: Mutex::new(Vec::new()),
        });

        // Emit starting event
        worker.emit_lifecycle_event("starting", None);

        Ok(worker)
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

        // Get org ID from config
        let org_id = Uuid::parse_str(&self.config.org_id).unwrap_or_else(|_| Uuid::nil());

        let result = self.runtime.block_on(async {
            lifecycle_client
                .register_worker(
                    &worker_name,
                    "0.1.0",
                    worker_type,
                    org_id,
                    None, // No team_id
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

        // Emit registered and ready events
        self.emit_lifecycle_event("registered", Some(format!("{{\"worker_id\":\"{}\"}}", result.worker_id)));
        self.emit_lifecycle_event("ready", None);

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

        // Don't poll when paused
        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
        }

        let worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or(FfiError::Other {
                msg: "Worker not registered".to_string(),
            })?
        };

        let org_id = Uuid::parse_str(&self.config.org_id).unwrap_or_else(|_| Uuid::nil());

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        let result = self.runtime.block_on(async {
            dispatch_client
                .poll_workflow(
                    &worker_id.to_string(),
                    &self.config.org_id,
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
                    org_id,
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
        let proto_commands: Vec<flovyn_worker_core::generated::flovyn_v1::WorkflowCommand> =
            commands
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

        // Don't poll when paused
        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
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
                    &self.config.org_id,
                    &self.config.queue,
                    std::time::Duration::from_secs(30),
                )
                .await
        })?;

        match result {
            Some(task_info) => {
                // Create task context for streaming and lifecycle
                let task_context = FfiTaskContext::new(
                    task_info.id,
                    task_info.workflow_execution_id,
                    task_info.execution_count,
                    self.channel.clone(),
                    self.worker_token.clone(),
                    self.runtime.clone(),
                );

                let activation = TaskActivation {
                    context: task_context,
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
        } else if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            "paused".to_string()
        } else if worker_id.is_some() {
            "running".to_string()
        } else {
            "initializing".to_string()
        }
    }

    // =========================================================================
    // Pause/Resume APIs
    // =========================================================================

    /// Pause the worker.
    ///
    /// When paused, the worker will not poll for new work but will continue
    /// processing any in-flight work.
    ///
    /// Returns an error if the worker is not in Running state.
    pub fn pause(&self, reason: String) -> Result<(), FfiError> {
        // Check if already paused
        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(FfiError::InvalidState {
                msg: "Worker is already paused".to_string(),
            });
        }

        // Check if shutdown requested
        if self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(FfiError::InvalidState {
                msg: "Cannot pause worker that is shutting down".to_string(),
            });
        }

        // Check if registered
        if self.worker_id.lock().is_none() {
            return Err(FfiError::InvalidState {
                msg: "Cannot pause worker that is not registered".to_string(),
            });
        }

        // Set paused state
        self.paused.store(true, std::sync::atomic::Ordering::Relaxed);
        *self.pause_reason.lock() = Some(reason.clone());

        // Emit event
        self.emit_lifecycle_event("paused", Some(format!("{{\"reason\":\"{}\"}}", reason)));

        Ok(())
    }

    /// Resume the worker.
    ///
    /// Returns an error if the worker is not in Paused state.
    pub fn resume(&self) -> Result<(), FfiError> {
        // Check if not paused
        if !self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(FfiError::InvalidState {
                msg: "Worker is not paused".to_string(),
            });
        }

        // Clear paused state
        self.paused.store(false, std::sync::atomic::Ordering::Relaxed);
        *self.pause_reason.lock() = None;

        // Emit event
        self.emit_lifecycle_event("resumed", None);

        Ok(())
    }

    /// Check if the worker is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Check if the worker is running (not paused and not shutting down).
    pub fn is_running(&self) -> bool {
        let is_registered = self.worker_id.lock().is_some();
        let is_shutdown = self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed);
        let is_paused = self.paused.load(std::sync::atomic::Ordering::Relaxed);

        is_registered && !is_shutdown && !is_paused
    }

    /// Get the pause reason (if paused).
    pub fn get_pause_reason(&self) -> Option<String> {
        self.pause_reason.lock().clone()
    }

    // =========================================================================
    // Config Accessor APIs
    // =========================================================================

    /// Get the maximum concurrent workflows setting.
    pub fn get_max_concurrent_workflows(&self) -> u32 {
        self.config.max_concurrent_workflow_tasks.unwrap_or(100)
    }

    /// Get the maximum concurrent tasks setting.
    pub fn get_max_concurrent_tasks(&self) -> u32 {
        self.config.max_concurrent_tasks.unwrap_or(100)
    }

    /// Get the queue name.
    pub fn get_queue(&self) -> String {
        self.config.queue.clone()
    }

    /// Get the org ID.
    pub fn get_org_id(&self) -> String {
        self.config.org_id.clone()
    }

    /// Get the server URL.
    pub fn get_server_url(&self) -> String {
        self.config.server_url.clone()
    }

    /// Get the worker identity.
    pub fn get_worker_identity(&self) -> Option<String> {
        self.config.worker_identity.clone()
    }

    // =========================================================================
    // Lifecycle Events APIs
    // =========================================================================

    /// Poll for lifecycle events.
    ///
    /// Returns all events that have occurred since the last poll.
    /// Events are cleared after being returned.
    pub fn poll_lifecycle_events(&self) -> Vec<LifecycleEvent> {
        let mut events = self.lifecycle_events.lock();
        std::mem::take(&mut *events)
    }

    /// Get the count of pending lifecycle events.
    pub fn pending_lifecycle_event_count(&self) -> u32 {
        self.lifecycle_events.lock().len() as u32
    }

    // =========================================================================
    // Lifecycle APIs
    // =========================================================================

    /// Get the worker uptime in milliseconds.
    ///
    /// Returns the number of milliseconds since the worker was created.
    pub fn get_uptime_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};

        let started_at = self.started_at_ms.load(std::sync::atomic::Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        now - started_at
    }

    /// Get the worker start time in milliseconds since Unix epoch.
    pub fn get_started_at_ms(&self) -> i64 {
        self.started_at_ms.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get the server-assigned worker ID (if registered).
    pub fn get_worker_id(&self) -> Option<String> {
        self.worker_id.lock().map(|id| id.to_string())
    }

    /// Get worker metrics.
    ///
    /// Returns a record containing various worker metrics.
    pub fn get_metrics(&self) -> WorkerMetrics {
        let uptime_ms = self.get_uptime_ms();
        let status = self.get_status();
        let worker_id = self.get_worker_id();

        WorkerMetrics {
            uptime_ms,
            status,
            worker_id,
            // TODO: Add more metrics as they become available
            workflows_processed: 0,
            tasks_processed: 0,
            active_workflows: 0,
            active_tasks: 0,
        }
    }

    /// Get registration information.
    ///
    /// Returns information about the worker's registration with the server.
    pub fn get_registration_info(&self) -> Option<crate::types::FfiRegistrationInfo> {
        let worker_id = self.worker_id.lock();
        let worker_id = worker_id.as_ref()?;

        let workflow_kinds: Vec<String> = self
            .config
            .workflow_metadata
            .iter()
            .map(|m| m.kind.clone())
            .collect();
        let task_kinds: Vec<String> = self
            .config
            .task_metadata
            .iter()
            .map(|m| m.kind.clone())
            .collect();

        Some(crate::types::FfiRegistrationInfo {
            worker_id: worker_id.to_string(),
            success: true,
            registered_at_ms: self.started_at_ms.load(std::sync::atomic::Ordering::Relaxed),
            workflow_kinds,
            task_kinds,
            has_conflicts: false,
        })
    }

    /// Get connection information.
    ///
    /// Returns information about the worker's connection to the server.
    pub fn get_connection_info(&self) -> crate::types::FfiConnectionInfo {
        use std::time::{SystemTime, UNIX_EPOCH};

        let is_running = self.worker_id.lock().is_some();
        let is_shutdown = self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed);

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        crate::types::FfiConnectionInfo {
            connected: is_running && !is_shutdown,
            last_heartbeat_ms: if is_running { Some(now_ms) } else { None },
            last_poll_ms: if is_running { Some(now_ms) } else { None },
            heartbeat_failures: 0,
            poll_failures: 0,
            reconnect_attempt: None,
        }
    }
}

/// Private implementation for internal helpers.
impl CoreWorker {
    /// Internal helper to emit a lifecycle event.
    fn emit_lifecycle_event(&self, event_name: &str, data: Option<String>) {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let event = LifecycleEvent {
            event_name: event_name.to_string(),
            timestamp_ms,
            data,
        };

        self.lifecycle_events.lock().push(event);
    }
}

/// Worker metrics for FFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WorkerMetrics {
    /// Uptime in milliseconds.
    pub uptime_ms: i64,
    /// Current worker status.
    pub status: String,
    /// Server-assigned worker ID (if registered).
    pub worker_id: Option<String>,
    /// Total workflows processed.
    pub workflows_processed: u64,
    /// Total tasks processed.
    pub tasks_processed: u64,
    /// Currently active workflows.
    pub active_workflows: u32,
    /// Currently active tasks.
    pub active_tasks: u32,
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
