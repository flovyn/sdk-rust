//! NapiWorker - Main worker object for NAPI bindings.
//!
//! The NapiWorker handles worker lifecycle, polling for activations,
//! and processing completions.

use std::sync::Arc;

use flovyn_worker_core::client::{
    oauth2, TaskExecutionClient, WorkerLifecycleClient, WorkerType, WorkflowDispatch,
};
use flovyn_worker_core::task::TaskMetadata as CoreTaskMetadata;
use flovyn_worker_core::workflow::WorkflowMetadata as CoreWorkflowMetadata;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::activation::{
    TaskActivationData, TaskCompletion, TaskCompletionStatusType, WorkflowActivationData,
    WorkflowActivationJob, WorkflowCompletionStatus, WorkflowCompletionStatusType,
};
use crate::config::WorkerConfig;
use crate::error::{from_core_error, napi_error, NapiErrorCode};
use crate::types::{ConnectionInfo, LifecycleEvent, RegistrationInfo, WorkerMetrics};

/// The main worker object exposed to Node.js via NAPI.
///
/// NapiWorker manages:
/// - Connection to the Flovyn server
/// - Worker registration
/// - Polling for workflow and task activations
/// - Processing completions
#[napi]
pub struct NapiWorker {
    /// gRPC channel
    channel: Channel,
    /// Worker token
    worker_token: String,
    /// Worker configuration
    config: WorkerConfig,
    /// Server-assigned worker ID after registration
    worker_id: Mutex<Option<Uuid>>,
    /// Tokio runtime for async operations
    #[allow(dead_code)]
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

#[napi]
impl NapiWorker {
    /// Create a new NapiWorker with the given configuration.
    #[napi(constructor)]
    pub fn new(config: WorkerConfig) -> Result<Self> {
        use std::time::{SystemTime, UNIX_EPOCH};

        // Create a Tokio runtime
        let runtime = Arc::new(Runtime::new().map_err(|e| {
            napi_error(
                NapiErrorCode::Other,
                format!("Failed to create runtime: {}", e),
            )
        })?);

        // Get start time in milliseconds
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        // Connect to the server
        let channel: Channel = runtime.block_on(async {
            Channel::from_shared(config.server_url.clone())
                .map_err(|e| {
                    napi_error(
                        NapiErrorCode::InvalidConfiguration,
                        format!("Invalid server URL: {}", e),
                    )
                })?
                .connect()
                .await
                .map_err(|e| napi_error(NapiErrorCode::Grpc, format!("Failed to connect: {}", e)))
        })?;

        // Determine authentication token
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
                    .map_err(|e| {
                        napi_error(
                            NapiErrorCode::Other,
                            format!("OAuth2 token fetch failed: {}", e),
                        )
                    })
            })?
        } else if let Some(token) = &config.worker_token {
            token.clone()
        } else {
            format!("fwt_placeholder-{}", Uuid::new_v4())
        };

        let worker = Self {
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
        };

        // Emit starting event
        worker.emit_lifecycle_event("starting", None);

        Ok(worker)
    }

    /// Register the worker with the Flovyn server.
    #[napi]
    pub async fn register(&self) -> Result<String> {
        // Convert config metadata to core metadata
        let workflows: Vec<CoreWorkflowMetadata> = self
            .config
            .workflow_metadata
            .iter()
            .map(|m| {
                let mut metadata = CoreWorkflowMetadata::new(&m.kind)
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

        let tasks: Vec<CoreTaskMetadata> = self
            .config
            .task_metadata
            .iter()
            .map(|m| CoreTaskMetadata::new(&m.kind))
            .collect();

        let mut lifecycle_client =
            WorkerLifecycleClient::new(self.channel.clone(), &self.worker_token);

        // Determine worker type
        let worker_type = match (workflows.is_empty(), tasks.is_empty()) {
            (false, false) => WorkerType::Unified,
            (false, true) => WorkerType::Workflow,
            (true, false) => WorkerType::Task,
            (true, true) => WorkerType::Unified,
        };

        let worker_name = self
            .config
            .worker_identity
            .clone()
            .unwrap_or_else(|| format!("napi-worker-{}", &self.config.queue));

        let org_id = Uuid::parse_str(&self.config.org_id).unwrap_or_else(|_| Uuid::nil());

        let result = lifecycle_client
            .register_worker(
                &worker_name,
                "0.1.0",
                worker_type,
                org_id,
                None,
                workflows,
                tasks,
                Vec::new(),
            )
            .await
            .map_err(from_core_error)?;

        if !result.success {
            return Err(napi_error(
                NapiErrorCode::Other,
                result
                    .error
                    .unwrap_or_else(|| "Registration failed".to_string()),
            ));
        }

        *self.worker_id.lock() = Some(result.worker_id);

        // Emit registered and ready events
        self.emit_lifecycle_event(
            "registered",
            Some(format!("{{\"worker_id\":\"{}\"}}", result.worker_id)),
        );
        self.emit_lifecycle_event("ready", None);

        Ok(result.worker_id.to_string())
    }

    /// Poll for the next workflow activation.
    /// Returns plain activation data that the TypeScript SDK uses to create its own context.
    #[napi]
    pub async fn poll_workflow_activation(&self) -> Result<Option<WorkflowActivationData>> {
        if self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(napi_error(
                NapiErrorCode::ShuttingDown,
                "Worker is shutting down",
            ));
        }

        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
        }

        let worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or_else(|| napi_error(NapiErrorCode::InvalidState, "Worker not registered"))?
        };

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        let result = dispatch_client
            .poll_workflow(
                &worker_id.to_string(),
                &self.config.org_id,
                &self.config.queue,
                std::time::Duration::from_secs(30),
            )
            .await
            .map_err(from_core_error)?;

        match result {
            Some(workflow_info) => {
                // Get workflow events for replay
                let events = dispatch_client
                    .get_events(workflow_info.id, None)
                    .await
                    .unwrap_or_default();

                // Convert events to JSON strings for TypeScript SDK
                let replay_events: Vec<String> = events
                    .into_iter()
                    .map(|e| {
                        serde_json::json!({
                            "sequenceNumber": e.sequence,
                            "eventType": e.event_type,
                            "data": e.payload,
                            "timestampMs": 0
                        })
                        .to_string()
                    })
                    .collect();

                // Build jobs
                let jobs = vec![WorkflowActivationJob::initialize(
                    serde_json::to_vec(&workflow_info.input).unwrap_or_default(),
                )];

                // Serialize input as JSON string
                let input = serde_json::to_string(&workflow_info.input).unwrap_or_default();

                Ok(Some(WorkflowActivationData {
                    workflow_execution_id: workflow_info.id.to_string(),
                    org_id: self.config.org_id.clone(),
                    workflow_kind: workflow_info.kind.clone(),
                    input,
                    jobs,
                    replay_events,
                    state_entries: vec![],
                    timestamp_ms: workflow_info.workflow_task_time_millis,
                    random_seed: workflow_info.id.as_u128().to_string(),
                    cancellation_requested: false,
                }))
            }
            None => Ok(None),
        }
    }

    /// Complete a workflow activation.
    /// The status.commands field contains the serialized commands from the context.
    #[napi]
    pub async fn complete_workflow_activation(
        &self,
        workflow_execution_id: String,
        status: WorkflowCompletionStatus,
    ) -> Result<()> {
        let _worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or_else(|| napi_error(NapiErrorCode::InvalidState, "Worker not registered"))?
        };

        let workflow_id = Uuid::parse_str(&workflow_execution_id).map_err(|_| {
            napi_error(
                NapiErrorCode::InvalidConfiguration,
                "Invalid workflow execution ID",
            )
        })?;

        // Parse commands from JSON string
        let proto_commands: Vec<flovyn_worker_core::generated::flovyn_v1::WorkflowCommand> =
            if let Some(commands_json) = &status.commands {
                parse_commands_from_json(commands_json)
            } else {
                vec![]
            };

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        match status.status {
            WorkflowCompletionStatusType::Completed => {
                let output: serde_json::Value = status
                    .output
                    .as_ref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Null);
                dispatch_client
                    .complete_workflow(workflow_id, output, proto_commands)
                    .await
                    .map_err(from_core_error)?;
            }
            WorkflowCompletionStatusType::Suspended => {
                dispatch_client
                    .suspend_workflow(workflow_id, proto_commands)
                    .await
                    .map_err(from_core_error)?;
            }
            WorkflowCompletionStatusType::Cancelled => {
                let reason = status.reason.unwrap_or_else(|| "Cancelled".to_string());
                dispatch_client
                    .fail_workflow(workflow_id, &reason, "CANCELLED", proto_commands)
                    .await
                    .map_err(from_core_error)?;
            }
            WorkflowCompletionStatusType::Failed => {
                let error = status.error.unwrap_or_else(|| "Unknown error".to_string());
                dispatch_client
                    .fail_workflow(workflow_id, &error, "ERROR", proto_commands)
                    .await
                    .map_err(from_core_error)?;
            }
        }

        Ok(())
    }

    /// Poll for the next task activation.
    /// Returns plain activation data for the TypeScript SDK.
    #[napi]
    pub async fn poll_task_activation(&self) -> Result<Option<TaskActivationData>> {
        if self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(napi_error(
                NapiErrorCode::ShuttingDown,
                "Worker is shutting down",
            ));
        }

        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
        }

        let worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or_else(|| napi_error(NapiErrorCode::InvalidState, "Worker not registered"))?
        };

        let mut task_client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);

        let result = task_client
            .poll_task(
                &worker_id.to_string(),
                &self.config.org_id,
                &self.config.queue,
                std::time::Duration::from_secs(30),
            )
            .await
            .map_err(from_core_error)?;

        match result {
            Some(task_info) => {
                // Serialize input as JSON string
                let input = serde_json::to_string(&task_info.input).unwrap_or_default();

                Ok(Some(TaskActivationData {
                    task_execution_id: task_info.id.to_string(),
                    task_kind: task_info.task_type.clone(),
                    input,
                    workflow_execution_id: task_info.workflow_execution_id.map(|id| id.to_string()),
                    attempt: task_info.execution_count,
                    max_retries: 3,
                    timeout_ms: None,
                }))
            }
            None => Ok(None),
        }
    }

    /// Complete a task activation.
    #[napi]
    pub async fn complete_task(&self, completion: TaskCompletion) -> Result<()> {
        let _worker_id = {
            let guard = self.worker_id.lock();
            guard.ok_or_else(|| napi_error(NapiErrorCode::InvalidState, "Worker not registered"))?
        };

        let task_execution_id = Uuid::parse_str(&completion.task_execution_id).map_err(|_| {
            napi_error(
                NapiErrorCode::InvalidConfiguration,
                "Invalid task execution ID",
            )
        })?;

        let mut task_client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);

        match completion.status {
            TaskCompletionStatusType::Completed => {
                let output: serde_json::Value = completion
                    .output
                    .as_ref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::Value::Null);
                task_client
                    .complete_task(task_execution_id, output)
                    .await
                    .map_err(from_core_error)?;
            }
            TaskCompletionStatusType::Failed => {
                let error = completion
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string());
                task_client
                    .fail_task(task_execution_id, &error)
                    .await
                    .map_err(from_core_error)?;
            }
            TaskCompletionStatusType::Cancelled => {
                task_client
                    .fail_task(task_execution_id, "Cancelled")
                    .await
                    .map_err(from_core_error)?;
            }
        }

        Ok(())
    }

    /// Initiate graceful shutdown of the worker.
    #[napi]
    pub fn shutdown(&self) {
        self.shutdown_requested
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check if shutdown has been requested.
    #[napi(getter)]
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get the current worker status.
    #[napi(getter)]
    pub fn status(&self) -> String {
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

    /// Pause the worker.
    #[napi]
    pub fn pause(&self, reason: String) -> Result<()> {
        if self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(napi_error(
                NapiErrorCode::InvalidState,
                "Worker is already paused",
            ));
        }

        if self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(napi_error(
                NapiErrorCode::InvalidState,
                "Cannot pause worker that is shutting down",
            ));
        }

        if self.worker_id.lock().is_none() {
            return Err(napi_error(
                NapiErrorCode::InvalidState,
                "Cannot pause worker that is not registered",
            ));
        }

        self.paused
            .store(true, std::sync::atomic::Ordering::Relaxed);
        *self.pause_reason.lock() = Some(reason.clone());

        self.emit_lifecycle_event("paused", Some(format!("{{\"reason\":\"{}\"}}", reason)));

        Ok(())
    }

    /// Resume the worker.
    #[napi]
    pub fn resume(&self) -> Result<()> {
        if !self.paused.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(napi_error(
                NapiErrorCode::InvalidState,
                "Worker is not paused",
            ));
        }

        self.paused
            .store(false, std::sync::atomic::Ordering::Relaxed);
        *self.pause_reason.lock() = None;

        self.emit_lifecycle_event("resumed", None);

        Ok(())
    }

    /// Check if the worker is paused.
    #[napi(getter)]
    pub fn is_paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Check if the worker is running.
    #[napi(getter)]
    pub fn is_running(&self) -> bool {
        let is_registered = self.worker_id.lock().is_some();
        let is_shutdown = self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed);
        let is_paused = self.paused.load(std::sync::atomic::Ordering::Relaxed);

        is_registered && !is_shutdown && !is_paused
    }

    /// Get the pause reason.
    #[napi(getter)]
    pub fn pause_reason(&self) -> Option<String> {
        self.pause_reason.lock().clone()
    }

    /// Get the maximum concurrent workflows setting.
    #[napi(getter)]
    pub fn max_concurrent_workflows(&self) -> u32 {
        self.config.max_concurrent_workflow_tasks.unwrap_or(100)
    }

    /// Get the maximum concurrent tasks setting.
    #[napi(getter)]
    pub fn max_concurrent_tasks(&self) -> u32 {
        self.config.max_concurrent_tasks.unwrap_or(100)
    }

    /// Get the queue name.
    #[napi(getter)]
    pub fn queue(&self) -> String {
        self.config.queue.clone()
    }

    /// Get the org ID.
    #[napi(getter)]
    pub fn org_id(&self) -> String {
        self.config.org_id.clone()
    }

    /// Get the server URL.
    #[napi(getter)]
    pub fn server_url(&self) -> String {
        self.config.server_url.clone()
    }

    /// Get the worker identity.
    #[napi(getter)]
    pub fn worker_identity(&self) -> Option<String> {
        self.config.worker_identity.clone()
    }

    /// Poll for lifecycle events.
    #[napi]
    pub fn poll_lifecycle_events(&self) -> Vec<LifecycleEvent> {
        let mut events = self.lifecycle_events.lock();
        std::mem::take(&mut *events)
    }

    /// Get the count of pending lifecycle events.
    #[napi]
    pub fn pending_lifecycle_event_count(&self) -> u32 {
        self.lifecycle_events.lock().len() as u32
    }

    /// Get the worker uptime in milliseconds.
    #[napi(getter)]
    pub fn uptime_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};

        let started_at = self
            .started_at_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        now - started_at
    }

    /// Get the worker start time in milliseconds since Unix epoch.
    #[napi(getter)]
    pub fn started_at_ms(&self) -> i64 {
        self.started_at_ms
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get the server-assigned worker ID.
    #[napi(getter)]
    pub fn worker_id(&self) -> Option<String> {
        self.worker_id.lock().map(|id| id.to_string())
    }

    /// Get worker metrics.
    #[napi]
    pub fn get_metrics(&self) -> WorkerMetrics {
        WorkerMetrics {
            uptime_ms: self.uptime_ms(),
            status: self.status(),
            worker_id: self.worker_id(),
            workflows_processed: 0,
            tasks_processed: 0,
            active_workflows: 0,
            active_tasks: 0,
        }
    }

    /// Get registration information.
    #[napi]
    pub fn get_registration_info(&self) -> Option<RegistrationInfo> {
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

        Some(RegistrationInfo {
            worker_id: worker_id.to_string(),
            success: true,
            registered_at_ms: self
                .started_at_ms
                .load(std::sync::atomic::Ordering::Relaxed),
            workflow_kinds,
            task_kinds,
            has_conflicts: false,
        })
    }

    /// Get connection information.
    #[napi]
    pub fn get_connection_info(&self) -> ConnectionInfo {
        use std::time::{SystemTime, UNIX_EPOCH};

        let is_running = self.worker_id.lock().is_some();
        let is_shutdown = self
            .shutdown_requested
            .load(std::sync::atomic::Ordering::Relaxed);

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        ConnectionInfo {
            connected: is_running && !is_shutdown,
            last_heartbeat_ms: if is_running { Some(now_ms) } else { None },
            last_poll_ms: if is_running { Some(now_ms) } else { None },
            heartbeat_failures: 0,
            poll_failures: 0,
            reconnect_attempt: None,
        }
    }
}

impl NapiWorker {
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

/// Parse commands from JSON string into proto commands.
fn parse_commands_from_json(
    json: &str,
) -> Vec<flovyn_worker_core::generated::flovyn_v1::WorkflowCommand> {
    use base64::Engine;
    use flovyn_worker_core::generated::flovyn_v1::{self, workflow_command::CommandData};

    let commands: Vec<serde_json::Value> = serde_json::from_str(json).unwrap_or_default();

    commands
        .into_iter()
        .enumerate()
        .filter_map(|(i, cmd)| {
            let cmd_type = cmd.get("type")?.as_str()?;
            let sequence_number = (i + 1) as i32;

            let (command_type, command_data) = match cmd_type {
                "RecordOperation" => {
                    let operation_name = cmd.get("operationName")?.as_str()?.to_string();
                    let result_b64 = cmd.get("result")?.as_str()?;
                    let result = base64::engine::general_purpose::STANDARD
                        .decode(result_b64)
                        .unwrap_or_default();
                    (
                        flovyn_v1::CommandType::RecordOperation as i32,
                        Some(CommandData::RecordOperation(
                            flovyn_v1::RecordOperationCommand {
                                operation_name,
                                result,
                            },
                        )),
                    )
                }
                "SetState" => {
                    let key = cmd.get("key")?.as_str()?.to_string();
                    let value_b64 = cmd.get("value")?.as_str()?;
                    let value = base64::engine::general_purpose::STANDARD
                        .decode(value_b64)
                        .unwrap_or_default();
                    (
                        flovyn_v1::CommandType::SetState as i32,
                        Some(CommandData::SetState(flovyn_v1::SetStateCommand {
                            key,
                            value,
                        })),
                    )
                }
                "ClearState" => {
                    let key = cmd.get("key")?.as_str()?.to_string();
                    (
                        flovyn_v1::CommandType::ClearState as i32,
                        Some(CommandData::ClearState(flovyn_v1::ClearStateCommand {
                            key,
                        })),
                    )
                }
                "ScheduleTask" => {
                    let task_execution_id = cmd.get("taskExecutionId")?.as_str()?.to_string();
                    let kind = cmd.get("kind")?.as_str()?.to_string();
                    let input_b64 = cmd.get("input")?.as_str()?;
                    let input = base64::engine::general_purpose::STANDARD
                        .decode(input_b64)
                        .unwrap_or_default();
                    let priority_seconds = cmd.get("prioritySeconds").and_then(|v| v.as_i64());
                    let max_retries = cmd.get("maxRetries").and_then(|v| v.as_i64());
                    let timeout_ms = cmd.get("timeoutMs").and_then(|v| v.as_i64());
                    let queue = cmd.get("queue").and_then(|v| v.as_str()).map(String::from);
                    let idempotency_key = cmd
                        .get("idempotencyKey")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let idempotency_key_ttl_seconds =
                        cmd.get("idempotencyKeyTtlSeconds").and_then(|v| v.as_i64());

                    (
                        flovyn_v1::CommandType::ScheduleTask as i32,
                        Some(CommandData::ScheduleTask(flovyn_v1::ScheduleTaskCommand {
                            kind,
                            input,
                            task_execution_id,
                            max_retries: max_retries.map(|v| v as i32),
                            timeout_ms,
                            queue,
                            priority_seconds: priority_seconds.map(|v| v as i32),
                            idempotency_key,
                            idempotency_key_ttl_seconds,
                        })),
                    )
                }
                "ScheduleChildWorkflow" => {
                    let name = cmd.get("name")?.as_str()?.to_string();
                    let kind = cmd.get("kind").and_then(|v| v.as_str()).map(String::from);
                    let child_execution_id = cmd.get("childExecutionId")?.as_str()?.to_string();
                    let input_b64 = cmd.get("input")?.as_str()?;
                    let input = base64::engine::general_purpose::STANDARD
                        .decode(input_b64)
                        .unwrap_or_default();
                    let queue = cmd
                        .get("queue")
                        .and_then(|v| v.as_str())
                        .unwrap_or("default")
                        .to_string();
                    let priority_seconds = cmd
                        .get("prioritySeconds")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as i32;

                    (
                        flovyn_v1::CommandType::ScheduleChildWorkflow as i32,
                        Some(CommandData::ScheduleChildWorkflow(
                            flovyn_v1::ScheduleChildWorkflowCommand {
                                child_execution_name: name,
                                workflow_kind: kind,
                                workflow_definition_id: None,
                                child_workflow_execution_id: child_execution_id,
                                input,
                                queue,
                                priority_seconds,
                            },
                        )),
                    )
                }
                "CompleteWorkflow" => {
                    let output_b64 = cmd.get("output")?.as_str()?;
                    let output = base64::engine::general_purpose::STANDARD
                        .decode(output_b64)
                        .unwrap_or_default();
                    (
                        flovyn_v1::CommandType::CompleteWorkflow as i32,
                        Some(CommandData::CompleteWorkflow(
                            flovyn_v1::CompleteWorkflowCommand { output },
                        )),
                    )
                }
                "FailWorkflow" => {
                    let error = cmd.get("error")?.as_str()?.to_string();
                    let stack_trace = cmd
                        .get("stackTrace")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let failure_type = cmd
                        .get("failureType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    (
                        flovyn_v1::CommandType::FailWorkflow as i32,
                        Some(CommandData::FailWorkflow(flovyn_v1::FailWorkflowCommand {
                            error,
                            stack_trace,
                            failure_type,
                        })),
                    )
                }
                "SuspendWorkflow" => {
                    let reason = cmd.get("reason")?.as_str()?.to_string();
                    (
                        flovyn_v1::CommandType::SuspendWorkflow as i32,
                        Some(CommandData::SuspendWorkflow(
                            flovyn_v1::SuspendWorkflowCommand { reason },
                        )),
                    )
                }
                "CancelWorkflow" => {
                    let reason = cmd.get("reason")?.as_str()?.to_string();
                    (
                        flovyn_v1::CommandType::CancelWorkflow as i32,
                        Some(CommandData::CancelWorkflow(
                            flovyn_v1::CancelWorkflowCommand { reason },
                        )),
                    )
                }
                "CreatePromise" => {
                    let promise_id = cmd.get("promiseId")?.as_str()?.to_string();
                    let timeout_ms = cmd.get("timeoutMs").and_then(|v| v.as_i64());
                    let idempotency_key = cmd
                        .get("idempotencyKey")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let idempotency_key_ttl_seconds =
                        cmd.get("idempotencyKeyTtlSeconds").and_then(|v| v.as_i64());
                    (
                        flovyn_v1::CommandType::CreatePromise as i32,
                        Some(CommandData::CreatePromise(
                            flovyn_v1::CreatePromiseCommand {
                                promise_id,
                                timeout_ms,
                                idempotency_key,
                                idempotency_key_ttl_seconds,
                            },
                        )),
                    )
                }
                "ResolvePromise" => {
                    let promise_id = cmd.get("promiseId")?.as_str()?.to_string();
                    let value_b64 = cmd.get("value")?.as_str()?;
                    let value = base64::engine::general_purpose::STANDARD
                        .decode(value_b64)
                        .unwrap_or_default();
                    (
                        flovyn_v1::CommandType::ResolvePromise as i32,
                        Some(CommandData::ResolvePromise(
                            flovyn_v1::ResolvePromiseCommand { promise_id, value },
                        )),
                    )
                }
                "StartTimer" => {
                    let timer_id = cmd.get("timerId")?.as_str()?.to_string();
                    let duration_ms = cmd.get("durationMs")?.as_i64()?;
                    (
                        flovyn_v1::CommandType::StartTimer as i32,
                        Some(CommandData::StartTimer(flovyn_v1::StartTimerCommand {
                            timer_id,
                            duration_ms,
                        })),
                    )
                }
                "CancelTimer" => {
                    let timer_id = cmd.get("timerId")?.as_str()?.to_string();
                    (
                        flovyn_v1::CommandType::CancelTimer as i32,
                        Some(CommandData::CancelTimer(flovyn_v1::CancelTimerCommand {
                            timer_id,
                        })),
                    )
                }
                "RequestCancelChildWorkflow" => {
                    let child_execution_id = cmd.get("childExecutionId")?.as_str()?.to_string();
                    (
                        flovyn_v1::CommandType::RequestCancelChildWorkflow as i32,
                        Some(CommandData::RequestCancelChildWorkflow(
                            flovyn_v1::RequestCancelChildWorkflowCommand {
                                child_execution_name: String::new(),
                                child_workflow_execution_id: child_execution_id,
                                reason: String::new(),
                            },
                        )),
                    )
                }
                _ => return None,
            };

            Some(flovyn_v1::WorkflowCommand {
                sequence_number,
                command_type,
                command_data,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_commands_from_json_empty() {
        let commands = parse_commands_from_json("[]");
        assert!(commands.is_empty());
    }

    #[test]
    fn test_parse_commands_from_json_start_timer() {
        let json = r#"[{"type": "StartTimer", "timerId": "timer-1", "durationMs": 1000}]"#;
        let commands = parse_commands_from_json(json);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].command_type,
            flovyn_worker_core::generated::flovyn_v1::CommandType::StartTimer as i32
        );
    }
}
