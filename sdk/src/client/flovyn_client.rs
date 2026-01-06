//! FlovynClient - Main entry point for the Flovyn SDK
//!
//! The FlovynClient provides a high-level API for interacting with the Flovyn server,
//! including starting workflows, managing workers, and querying workflow state.

use crate::client::builder::FlovynClientBuilder;
use crate::client::hook::WorkflowHook;
use crate::client::{
    AuthInterceptor, TaskExecutionClient, WorkerLifecycleClient, WorkerType, WorkflowDispatch,
    WorkflowQueryClient,
};
use crate::config::FlovynClientConfig;
use crate::error::{FlovynError, Result};
use crate::task::registry::TaskRegistry;
use crate::worker::lifecycle::{
    ConnectionInfo, HookChain, ReconnectionStrategy, RegistrationInfo, TaskConflict,
    WorkerControlError, WorkerInternals, WorkerLifecycleEvent, WorkerMetrics, WorkerStatus,
    WorkflowConflict,
};
use crate::worker::registry::WorkflowRegistry;
use crate::worker::task_worker::{TaskExecutorWorker, TaskWorkerConfig};
use crate::worker::workflow_worker::{WorkflowExecutorWorker, WorkflowWorkerConfig};
use flovyn_sdk_core::generated::flovyn_v1;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::{watch, Notify};
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Result of starting a workflow
#[derive(Debug, Clone)]
pub struct StartWorkflowResult {
    /// The workflow execution ID
    pub workflow_execution_id: Uuid,
    /// Whether an idempotency key was used
    pub idempotency_key_used: bool,
    /// Whether a new execution was created (false means existing returned)
    pub idempotency_key_new: bool,
}

/// Options for starting a workflow
#[derive(Debug, Clone, Default)]
pub struct StartWorkflowOptions {
    /// Labels for the workflow
    pub labels: std::collections::HashMap<String, String>,
    /// Task queue to use
    pub queue: String,
    /// Priority in seconds (higher = lower priority)
    pub priority_seconds: i32,
    /// Idempotency key for deduplication
    pub idempotency_key: Option<String>,
    /// TTL for the idempotency key
    pub idempotency_key_ttl: Option<Duration>,
    /// Specific workflow version to use
    pub workflow_version: Option<String>,
}

impl StartWorkflowOptions {
    /// Create new options with default task queue
    pub fn new() -> Self {
        Self {
            queue: "default".to_string(),
            ..Default::default()
        }
    }

    /// Set labels
    pub fn with_labels(mut self, labels: std::collections::HashMap<String, String>) -> Self {
        self.labels = labels;
        self
    }

    /// Set task queue
    pub fn with_queue(mut self, queue: impl Into<String>) -> Self {
        self.queue = queue.into();
        self
    }

    /// Set priority
    pub fn with_priority(mut self, seconds: i32) -> Self {
        self.priority_seconds = seconds;
        self
    }

    /// Set idempotency key
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Set idempotency key TTL
    pub fn with_idempotency_key_ttl(mut self, ttl: Duration) -> Self {
        self.idempotency_key_ttl = Some(ttl);
        self
    }

    /// Set workflow version
    pub fn with_workflow_version(mut self, version: impl Into<String>) -> Self {
        self.workflow_version = Some(version.into());
        self
    }
}

/// Main client for interacting with the Flovyn server
pub struct FlovynClient {
    pub(crate) tenant_id: Uuid,
    pub(crate) worker_id: String,
    pub(crate) queue: String,
    pub(crate) poll_timeout: Duration,
    pub(crate) config: FlovynClientConfig,
    pub(crate) channel: Channel,
    pub(crate) space_id: Option<Uuid>,
    pub(crate) worker_name: Option<String>,
    pub(crate) worker_version: String,
    pub(crate) enable_auto_registration: bool,
    pub(crate) heartbeat_interval: Duration,
    pub(crate) workflow_registry: Arc<WorkflowRegistry>,
    pub(crate) task_registry: Arc<TaskRegistry>,
    pub(crate) workflow_hook: Option<Arc<dyn WorkflowHook>>,
    pub(crate) running: AtomicBool,
    /// Worker token for gRPC authentication
    pub(crate) worker_token: String,
    /// Enable telemetry (span reporting to server)
    pub(crate) enable_telemetry: bool,
    /// Worker lifecycle hooks
    pub(crate) lifecycle_hooks: HookChain,
    /// Reconnection strategy
    pub(crate) reconnection_strategy: ReconnectionStrategy,
    /// Stored worker internals from the last start() call
    pub(crate) worker_internals: parking_lot::RwLock<Option<Arc<WorkerInternals>>>,
}

impl FlovynClient {
    /// Create a new builder for FlovynClient
    pub fn builder() -> FlovynClientBuilder {
        FlovynClientBuilder::new()
    }

    /// Get the tenant ID
    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    /// Get the worker ID
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// Get the task queue
    pub fn queue(&self) -> &str {
        &self.queue
    }

    /// Check if a workflow is registered
    pub fn has_workflow(&self, kind: &str) -> bool {
        self.workflow_registry.has(kind)
    }

    /// Check if a task is registered
    pub fn has_task(&self, kind: &str) -> bool {
        self.task_registry.has(kind)
    }

    /// Check if the client is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get the lifecycle hooks
    pub fn lifecycle_hooks(&self) -> &HookChain {
        &self.lifecycle_hooks
    }

    /// Get the reconnection strategy
    pub fn reconnection_strategy(&self) -> &ReconnectionStrategy {
        &self.reconnection_strategy
    }

    /// Check if there is an active running worker.
    ///
    /// Returns true if `start()` has been called and the worker is still running
    /// (not stopped or shutting down).
    pub fn has_running_worker(&self) -> bool {
        self.worker_internals
            .read()
            .as_ref()
            .map(|internals| matches!(internals.status(), WorkerStatus::Running { .. }))
            .unwrap_or(false)
    }

    /// Get the worker internals for advanced monitoring.
    ///
    /// Returns `None` if `start()` has not been called yet.
    /// The internals provide access to status, metrics, registration info, and events.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(internals) = client.worker_internals() {
    ///     println!("Worker status: {:?}", internals.status());
    ///     println!("Uptime: {:?}", internals.uptime());
    /// }
    /// ```
    pub fn worker_internals(&self) -> Option<Arc<WorkerInternals>> {
        self.worker_internals.read().clone()
    }

    /// Get the configured heartbeat interval.
    pub fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    /// Get the configured poll timeout.
    pub fn poll_timeout(&self) -> Duration {
        self.poll_timeout
    }

    /// Get the maximum number of concurrent workflows.
    pub fn max_concurrent_workflows(&self) -> usize {
        self.config.workflow_config.max_concurrent
    }

    /// Get the maximum number of concurrent tasks.
    pub fn max_concurrent_tasks(&self) -> usize {
        self.config.task_config.max_concurrent
    }

    /// Start the workflow and task workers
    ///
    /// This spawns background tasks to poll for and execute workflows and tasks.
    pub async fn start(&self) -> Result<WorkerHandle> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(FlovynError::Other("Client already running".to_string()));
        }

        info!(
            worker_id = %self.worker_id,
            tenant_id = %self.tenant_id,
            queue = %self.queue,
            "Starting FlovynClient"
        );

        // Create shared worker internals for lifecycle tracking
        let internals = Arc::new(WorkerInternals::new(
            self.worker_id.clone(),
            self.worker_name.clone(),
            self.lifecycle_hooks.clone(),
        ));

        // Create shutdown channel for immediate shutdown signaling
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Perform unified registration for both workflows and tasks
        let server_worker_id = self.register_unified(&internals).await?;

        // Start workflow worker if there are registered workflows
        let (workflow_handle, workflow_ready, workflow_running) =
            if self.workflow_registry.has_registrations() {
                let config = WorkflowWorkerConfig {
                    worker_id: self.worker_id.clone(),
                    tenant_id: self.tenant_id,
                    queue: self.queue.clone(),
                    poll_timeout: self.poll_timeout,
                    no_work_backoff: Duration::from_millis(100),
                    max_concurrent: self.config.workflow_config.max_concurrent,
                    heartbeat_interval: self.heartbeat_interval,
                    worker_name: self.worker_name.clone(),
                    worker_version: self.worker_version.clone(),
                    space_id: self.space_id,
                    enable_auto_registration: false, // Unified registration already done
                    enable_notifications: true,
                    worker_token: self.worker_token.clone(),
                    enable_telemetry: self.enable_telemetry,
                    lifecycle_hooks: self.lifecycle_hooks.clone(),
                    reconnection_strategy: self.reconnection_strategy.clone(),
                    server_worker_id,
                };

                let mut worker = WorkflowExecutorWorker::new(
                    config,
                    self.workflow_registry.clone(),
                    self.channel.clone(),
                )
                .with_internals(internals.clone())
                .with_shutdown_signal(shutdown_rx.clone());

                // Apply hook if present
                if let Some(ref hook) = self.workflow_hook {
                    worker = worker.with_hook(hook.clone());
                }

                let ready_notify = worker.ready_notify();
                let running_flag = worker.running_flag();

                let handle = tokio::spawn(async move {
                    if let Err(e) = worker.start().await {
                        error!("Workflow worker error: {}", e);
                    }
                });

                (Some(handle), Some(ready_notify), Some(running_flag))
            } else {
                debug!("No workflows registered, skipping workflow worker");
                (None, None, None)
            };

        // Start task worker if there are registered tasks
        let (task_handle, task_ready, task_running) = if self.task_registry.has_registrations() {
            let config = TaskWorkerConfig {
                worker_id: self.worker_id.clone(),
                tenant_id: self.tenant_id,
                queue: self.queue.clone(),
                poll_timeout: self.poll_timeout,
                no_work_backoff: Duration::from_millis(100),
                worker_labels: self.config.worker_labels.clone(),
                heartbeat_interval: self.heartbeat_interval,
                worker_name: self.worker_name.clone(),
                worker_version: self.worker_version.clone(),
                space_id: self.space_id,
                enable_auto_registration: false, // Unified registration already done
                worker_token: self.worker_token.clone(),
                enable_telemetry: self.enable_telemetry,
                lifecycle_hooks: self.lifecycle_hooks.clone(),
                reconnection_strategy: self.reconnection_strategy.clone(),
                server_worker_id,
            };

            let mut worker =
                TaskExecutorWorker::new(config, self.task_registry.clone(), self.channel.clone())
                    .with_internals(internals.clone())
                    .with_shutdown_signal(shutdown_rx.clone());

            let ready_notify = worker.ready_notify();
            let running_flag = worker.running_flag();

            let handle = tokio::spawn(async move {
                if let Err(e) = worker.start().await {
                    error!("Task worker error: {}", e);
                }
            });

            (Some(handle), Some(ready_notify), Some(running_flag))
        } else {
            debug!("No tasks registered, skipping task worker");
            (None, None, None)
        };

        // Store the internals for later access
        *self.worker_internals.write() = Some(internals.clone());

        Ok(WorkerHandle {
            shutdown_tx,
            workflow_running,
            task_running,
            workflow_handle,
            task_handle,
            workflow_ready,
            task_ready,
            internals,
        })
    }

    /// Start a workflow
    pub async fn start_workflow(&self, workflow_kind: &str, input: Value) -> Result<Uuid> {
        self.start_workflow_with_options(workflow_kind, input, StartWorkflowOptions::new())
            .await
            .map(|r| r.workflow_execution_id)
    }

    /// Start a workflow with options
    pub async fn start_workflow_with_options(
        &self,
        workflow_kind: &str,
        input: Value,
        options: StartWorkflowOptions,
    ) -> Result<StartWorkflowResult> {
        let mut client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);

        let result = client
            .start_workflow(
                &self.tenant_id.to_string(),
                workflow_kind,
                input,
                Some(&options.queue),
                options.workflow_version.as_deref(),
                options.idempotency_key.as_deref(),
            )
            .await?;

        Ok(StartWorkflowResult {
            workflow_execution_id: result.workflow_execution_id,
            idempotency_key_used: result.idempotency_key_used,
            idempotency_key_new: result.idempotency_key_new,
        })
    }

    /// Resolve a promise for a workflow
    pub async fn resolve_promise(
        &self,
        workflow_execution_id: Uuid,
        promise_name: &str,
        value: Value,
    ) -> Result<()> {
        // Get the promise UUID from workflow events
        let promise_id = self
            .get_promise_id(workflow_execution_id, promise_name)
            .await?;
        let value_bytes = serde_json::to_vec(&value)?;

        let request = flovyn_v1::ResolvePromiseRequest {
            promise_id,
            value: value_bytes,
        };

        let interceptor = AuthInterceptor::worker_token(&self.worker_token);
        let mut client =
            flovyn_v1::workflow_dispatch_client::WorkflowDispatchClient::with_interceptor(
                self.channel.clone(),
                interceptor,
            );

        client
            .resolve_promise(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Reject a promise for a workflow
    pub async fn reject_promise(
        &self,
        workflow_execution_id: Uuid,
        promise_name: &str,
        error: &str,
    ) -> Result<()> {
        // Get the promise UUID from workflow events
        let promise_id = self
            .get_promise_id(workflow_execution_id, promise_name)
            .await?;

        let request = flovyn_v1::RejectPromiseRequest {
            promise_id,
            error: error.to_string(),
        };

        let interceptor = AuthInterceptor::worker_token(&self.worker_token);
        let mut client =
            flovyn_v1::workflow_dispatch_client::WorkflowDispatchClient::with_interceptor(
                self.channel.clone(),
                interceptor,
            );

        client
            .reject_promise(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Get workflow events
    pub async fn get_workflow_events(
        &self,
        workflow_execution_id: Uuid,
    ) -> Result<Vec<crate::client::WorkflowEvent>> {
        let mut client = WorkflowDispatch::new(self.channel.clone(), &self.worker_token);
        Ok(client.get_events(workflow_execution_id, None).await?)
    }

    /// Get promise UUID from workflow events
    async fn get_promise_id(
        &self,
        workflow_execution_id: Uuid,
        promise_name: &str,
    ) -> Result<String> {
        let events = self.get_workflow_events(workflow_execution_id).await?;

        for event in events {
            if event.event_type == "PROMISE_CREATED" {
                // Check if this is the promise we're looking for
                if let Some(name) = event.payload.get("promiseName").and_then(|v| v.as_str()) {
                    if name == promise_name {
                        // Extract the promise UUID
                        if let Some(id) = event.payload.get("promiseId").and_then(|v| v.as_str()) {
                            return Ok(id.to_string());
                        }
                    }
                }
            }
        }

        Err(FlovynError::PromiseNotFound(format!(
            "Promise '{}' not found in workflow {}",
            promise_name, workflow_execution_id
        )))
    }

    /// Query workflow state
    ///
    /// Executes a query against a running workflow to retrieve its current state.
    pub async fn query(
        &self,
        workflow_execution_id: Uuid,
        query_name: &str,
        params: Value,
    ) -> Result<Value> {
        let mut client = WorkflowQueryClient::new(self.channel.clone(), &self.worker_token);
        Ok(client
            .query(workflow_execution_id, query_name, params)
            .await?)
    }

    /// Query workflow state with typed result
    ///
    /// Executes a query against a running workflow and deserializes the result.
    pub async fn query_typed<T: DeserializeOwned>(
        &self,
        workflow_execution_id: Uuid,
        query_name: &str,
        params: Value,
    ) -> Result<T> {
        let mut client = WorkflowQueryClient::new(self.channel.clone(), &self.worker_token);
        Ok(client
            .query_typed(workflow_execution_id, query_name, params)
            .await?)
    }

    /// Start a workflow and wait for completion
    ///
    /// This method starts a workflow and polls for its completion within the timeout.
    /// Returns the workflow output on success, or error if it fails or times out.
    pub async fn start_workflow_and_wait(
        &self,
        workflow_kind: &str,
        input: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.start_workflow_and_wait_with_options(
            workflow_kind,
            input,
            StartWorkflowOptions::new(),
            timeout,
        )
        .await
    }

    /// Start a workflow with options and wait for completion
    pub async fn start_workflow_and_wait_with_options(
        &self,
        workflow_kind: &str,
        input: Value,
        options: StartWorkflowOptions,
        timeout: Duration,
    ) -> Result<Value> {
        let result = self
            .start_workflow_with_options(workflow_kind, input, options)
            .await?;

        let workflow_execution_id = result.workflow_execution_id;
        let start_time = std::time::Instant::now();
        let poll_interval = Duration::from_millis(500);

        loop {
            // Check timeout
            if start_time.elapsed() > timeout {
                return Err(FlovynError::Timeout(format!(
                    "Workflow {} did not complete within {:?}",
                    workflow_execution_id, timeout
                )));
            }

            // Get events and check for completion
            let events = self.get_workflow_events(workflow_execution_id).await?;

            for event in &events {
                match event.event_type.as_str() {
                    "WORKFLOW_COMPLETED" => {
                        // Parse output from event payload
                        if let Some(output) = event.payload.get("output") {
                            return Ok(output.clone());
                        }
                        return Ok(Value::Null);
                    }
                    "WORKFLOW_FAILED" => {
                        let error = event
                            .payload
                            .get("error")
                            .and_then(|v: &Value| v.as_str())
                            .unwrap_or("Unknown error");
                        return Err(FlovynError::WorkflowFailed(error.to_string()));
                    }
                    "WORKFLOW_CANCELLED" => {
                        let reason = event
                            .payload
                            .get("reason")
                            .and_then(|v: &Value| v.as_str())
                            .unwrap_or("Cancelled");
                        return Err(FlovynError::WorkflowCancelled(reason.to_string()));
                    }
                    _ => {}
                }
            }

            // Wait before polling again
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Get task state value
    ///
    /// Retrieves a state value for a task execution.
    pub async fn get_task_state(
        &self,
        task_execution_id: Uuid,
        key: &str,
    ) -> Result<Option<Value>> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        Ok(client.get_state(task_execution_id, key).await?)
    }

    /// Get all task state keys
    ///
    /// Retrieves all state keys for a task execution.
    pub async fn get_task_state_keys(&self, task_execution_id: Uuid) -> Result<Vec<String>> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        Ok(client.get_state_keys(task_execution_id).await?)
    }

    /// Set task state value
    ///
    /// Sets a state value for a task execution.
    pub async fn set_task_state(
        &self,
        task_execution_id: Uuid,
        key: &str,
        value: Value,
    ) -> Result<()> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        Ok(client.set_state(task_execution_id, key, value).await?)
    }

    /// Clear task state value
    ///
    /// Clears a state value for a task execution.
    pub async fn clear_task_state(&self, task_execution_id: Uuid, key: &str) -> Result<()> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        Ok(client.clear_state(task_execution_id, key).await?)
    }

    /// Clear all task state
    ///
    /// Clears all state values for a task execution.
    pub async fn clear_all_task_state(&self, task_execution_id: Uuid) -> Result<()> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        Ok(client.clear_all_state(task_execution_id).await?)
    }

    /// Get the workflow registry
    pub fn workflow_registry(&self) -> &WorkflowRegistry {
        &self.workflow_registry
    }

    /// Get the task registry
    pub fn task_registry(&self) -> &TaskRegistry {
        &self.task_registry
    }

    /// Perform unified registration for both workflows and tasks
    ///
    /// This sends a single registration request with all workflow and task capabilities,
    /// using UNIFIED worker type. Returns the server-assigned worker ID.
    async fn register_unified(&self, internals: &Arc<WorkerInternals>) -> Result<Option<Uuid>> {
        if !self.enable_auto_registration {
            debug!("Auto-registration disabled, skipping unified registration");
            return Ok(None);
        }

        // Skip if no capabilities to register
        if !self.workflow_registry.has_registrations() && !self.task_registry.has_registrations() {
            debug!("No workflows or tasks registered, skipping registration");
            return Ok(None);
        }

        let worker_name = self
            .worker_name
            .clone()
            .unwrap_or_else(|| self.worker_id.clone());

        // Collect all workflow metadata
        let workflows = self.workflow_registry.get_all_metadata();
        let workflow_kinds: Vec<String> = workflows.iter().map(|m| m.kind.clone()).collect();

        // Collect all task metadata
        let tasks = self.task_registry.get_all_metadata();
        let task_kinds: Vec<String> = tasks.iter().map(|m| m.kind.clone()).collect();

        info!(
            worker_name = %worker_name,
            workflow_count = workflows.len(),
            task_count = tasks.len(),
            "Registering worker with unified capabilities"
        );

        let mut lifecycle_client =
            WorkerLifecycleClient::new(self.channel.clone(), &self.worker_token);

        let result = lifecycle_client
            .register_worker(
                &worker_name,
                &self.worker_version,
                WorkerType::Unified,
                self.tenant_id,
                self.space_id,
                workflows.into_iter().map(Into::into).collect(),
                tasks.into_iter().map(Into::into).collect(),
            )
            .await?;

        // Convert to RegistrationInfo and update internals
        let registration_info = RegistrationInfo {
            worker_id: result.worker_id,
            success: result.success,
            registered_at: std::time::SystemTime::now(),
            workflow_kinds,
            task_kinds,
            workflow_conflicts: result
                .workflow_conflicts
                .iter()
                .map(|c| WorkflowConflict {
                    kind: c.kind.clone(),
                    reason: c.reason.clone(),
                    existing_worker_id: c.existing_worker_id.clone(),
                })
                .collect(),
            task_conflicts: result
                .task_conflicts
                .iter()
                .map(|c| TaskConflict {
                    kind: c.kind.clone(),
                    reason: c.reason.clone(),
                    existing_worker_id: c.existing_worker_id.clone(),
                })
                .collect(),
        };

        let server_worker_id = if result.success {
            info!(
                server_worker_id = %result.worker_id,
                "Worker registered successfully with unified capabilities"
            );
            Some(result.worker_id)
        } else {
            warn!(
                error = ?result.error,
                workflow_conflicts = result.workflow_conflicts.len(),
                task_conflicts = result.task_conflicts.len(),
                "Worker registration failed or had conflicts"
            );
            None
        };

        // Update internals and emit events
        internals.set_registration_info(registration_info).await;

        Ok(server_worker_id)
    }
}

/// Handle for managing running workers
pub struct WorkerHandle {
    shutdown_tx: watch::Sender<bool>,
    workflow_running: Option<Arc<AtomicBool>>,
    task_running: Option<Arc<AtomicBool>>,
    workflow_handle: Option<tokio::task::JoinHandle<()>>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
    workflow_ready: Option<Arc<Notify>>,
    task_ready: Option<Arc<Notify>>,
    /// Shared internal state for lifecycle tracking
    internals: Arc<WorkerInternals>,
}

impl WorkerHandle {
    /// Wait for all workers to be ready and polling
    ///
    /// This method blocks until all active workers have started their
    /// polling loops. Useful for ensuring workers are ready before
    /// starting workflows.
    pub async fn await_ready(&self) {
        if let Some(ref notify) = self.workflow_ready {
            notify.notified().await;
        }
        if let Some(ref notify) = self.task_ready {
            notify.notified().await;
        }
    }

    /// Wait for workflow worker to be ready
    pub async fn await_workflow_ready(&self) {
        if let Some(ref notify) = self.workflow_ready {
            notify.notified().await;
        }
    }

    /// Wait for task worker to be ready
    pub async fn await_task_ready(&self) {
        if let Some(ref notify) = self.task_ready {
            notify.notified().await;
        }
    }

    /// Stop the workers
    ///
    /// If `immediate` is true, workers will be interrupted immediately even if
    /// they are in the middle of a long-polling operation.
    /// If `immediate` is false, workers will finish their current poll before stopping.
    pub async fn stop_with_option(self, immediate: bool) {
        if immediate {
            // Send immediate shutdown signal via watch channel
            let _ = self.shutdown_tx.send(true);
        }

        // Set running flags for any code that checks them
        if let Some(ref running) = self.workflow_running {
            running.store(false, Ordering::SeqCst);
        }
        if let Some(ref running) = self.task_running {
            running.store(false, Ordering::SeqCst);
        }

        // Wait for handles to complete
        if let Some(handle) = self.workflow_handle {
            let _ = handle.await;
        }
        if let Some(handle) = self.task_handle {
            let _ = handle.await;
        }

        info!("Workers stopped");
    }

    /// Stop the workers immediately
    ///
    /// This interrupts any in-progress polling operations for fast shutdown.
    pub async fn stop(self) {
        self.stop_with_option(true).await;
    }

    /// Stop the workers gracefully
    ///
    /// This waits for the current polling operation to complete before stopping.
    /// May take up to the poll timeout duration.
    pub async fn stop_graceful(self) {
        self.stop_with_option(false).await;
    }

    /// Abort the workers immediately
    pub fn abort(self) {
        // Send immediate shutdown signal via watch channel
        let _ = self.shutdown_tx.send(true);

        // Also set running flags
        if let Some(ref running) = self.workflow_running {
            running.store(false, Ordering::SeqCst);
        }
        if let Some(ref running) = self.task_running {
            running.store(false, Ordering::SeqCst);
        }

        // Abort task handles
        if let Some(handle) = self.workflow_handle {
            handle.abort();
        }
        if let Some(handle) = self.task_handle {
            handle.abort();
        }
    }

    // ============ Status Methods ============

    /// Get the current worker status.
    pub fn status(&self) -> WorkerStatus {
        self.internals.status()
    }

    /// Check if the worker is currently running.
    pub fn is_running(&self) -> bool {
        matches!(self.internals.status(), WorkerStatus::Running { .. })
    }

    /// Check if the worker is connected to the server.
    pub fn is_connected(&self) -> bool {
        self.internals.connection_info().connected
    }

    /// Check if the worker is shutting down.
    pub fn is_shutting_down(&self) -> bool {
        matches!(self.internals.status(), WorkerStatus::ShuttingDown { .. })
    }

    /// Check if the worker is paused.
    pub fn is_paused(&self) -> bool {
        matches!(self.internals.status(), WorkerStatus::Paused { .. })
    }

    /// Check if the worker is reconnecting.
    pub fn is_reconnecting(&self) -> bool {
        matches!(self.internals.status(), WorkerStatus::Reconnecting { .. })
    }

    // ============ Registration Methods ============

    /// Get the registration information.
    pub fn registration(&self) -> Option<RegistrationInfo> {
        self.internals.registration_info()
    }

    /// Get the server-assigned worker ID.
    pub fn server_worker_id(&self) -> Option<Uuid> {
        self.internals.registration_info().map(|r| r.worker_id)
    }

    /// Check if there are any registration conflicts.
    pub fn has_conflicts(&self) -> bool {
        self.internals
            .registration_info()
            .map(|r| r.has_conflicts())
            .unwrap_or(false)
    }

    /// Get workflow registration conflicts.
    pub fn workflow_conflicts(&self) -> Vec<WorkflowConflict> {
        self.internals
            .registration_info()
            .map(|r| r.workflow_conflicts)
            .unwrap_or_default()
    }

    /// Get task registration conflicts.
    pub fn task_conflicts(&self) -> Vec<TaskConflict> {
        self.internals
            .registration_info()
            .map(|r| r.task_conflicts)
            .unwrap_or_default()
    }

    // ============ Connection Methods ============

    /// Get the connection information.
    pub fn connection(&self) -> ConnectionInfo {
        self.internals.connection_info()
    }

    /// Get the time since the last successful heartbeat.
    pub fn time_since_heartbeat(&self) -> Option<Duration> {
        self.internals.connection_info().last_heartbeat.map(|t| {
            std::time::SystemTime::now()
                .duration_since(t)
                .unwrap_or(Duration::ZERO)
        })
    }

    /// Get the time since the last successful poll.
    pub fn time_since_poll(&self) -> Option<Duration> {
        self.internals.connection_info().last_poll.map(|t| {
            std::time::SystemTime::now()
                .duration_since(t)
                .unwrap_or(Duration::ZERO)
        })
    }

    // ============ Metrics Methods ============

    /// Get the worker metrics.
    pub fn metrics(&self) -> WorkerMetrics {
        self.internals.metrics()
    }

    /// Get the worker uptime.
    pub fn uptime(&self) -> Duration {
        self.internals.uptime()
    }

    /// Get the number of workflows currently in progress.
    pub fn workflows_in_progress(&self) -> usize {
        self.internals.metrics().workflows_in_progress
    }

    /// Get the number of tasks currently in progress.
    pub fn tasks_in_progress(&self) -> usize {
        self.internals.metrics().tasks_in_progress
    }

    /// Get the total number of workflows executed.
    pub fn workflows_executed(&self) -> u64 {
        self.internals.metrics().workflows_executed
    }

    /// Get the total number of tasks executed.
    pub fn tasks_executed(&self) -> u64 {
        self.internals.metrics().tasks_executed
    }

    // ============ Event Subscription ============

    /// Subscribe to worker lifecycle events.
    ///
    /// Returns a broadcast receiver that will receive events as they occur.
    /// Events include status changes, heartbeats, work received/completed, etc.
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerLifecycleEvent> {
        self.internals.subscribe()
    }

    /// Subscribe to filtered worker lifecycle events.
    ///
    /// Returns a stream that only yields events matching the provided filter.
    /// This is useful for listening to specific event types without processing
    /// all events.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut events = handle.subscribe_filtered(|e| {
    ///     matches!(e, WorkerLifecycleEvent::WorkCompleted { .. })
    /// });
    ///
    /// while let Some(event) = events.next().await {
    ///     println!("Work completed: {:?}", event);
    /// }
    /// ```
    pub fn subscribe_filtered<F>(
        &self,
        filter: F,
    ) -> impl futures::Stream<Item = WorkerLifecycleEvent>
    where
        F: Fn(&WorkerLifecycleEvent) -> bool + Send + 'static,
    {
        use tokio_stream::wrappers::BroadcastStream;
        use tokio_stream::StreamExt;

        let rx = self.internals.subscribe();
        BroadcastStream::new(rx)
            .filter_map(|result| result.ok())
            .filter(move |event| filter(event))
    }

    // ============ Control Methods ============

    /// Pause the worker.
    ///
    /// When paused, workers will stop polling for new work but will complete
    /// any work currently in progress. This is useful for graceful maintenance
    /// or load shedding.
    ///
    /// Returns `Ok(())` if the worker was successfully paused, or an error if:
    /// - The worker is not in a Running state
    /// - A lifecycle hook rejected the pause request
    pub async fn pause(&self, reason: &str) -> std::result::Result<(), WorkerControlError> {
        self.internals.pause(reason).await
    }

    /// Resume the worker from a paused state.
    ///
    /// The worker will resume polling for new work.
    ///
    /// Returns `Ok(())` if the worker was successfully resumed, or an error if:
    /// - The worker is not in a Paused state
    /// - A lifecycle hook rejected the resume request
    pub async fn resume(&self) -> std::result::Result<(), WorkerControlError> {
        self.internals.resume().await
    }

    // ============ Internal Access ============

    /// Get the worker internals for advanced usage.
    ///
    /// This provides direct access to the shared state for custom monitoring
    /// or integration with external systems.
    pub fn internals(&self) -> Arc<WorkerInternals> {
        self.internals.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_workflow_options_default() {
        let options = StartWorkflowOptions::new();
        assert_eq!(options.queue, "default");
        assert_eq!(options.priority_seconds, 0);
        assert!(options.idempotency_key.is_none());
    }

    #[test]
    fn test_start_workflow_options_builder() {
        let options = StartWorkflowOptions::new()
            .with_queue("gpu-workers")
            .with_priority(10)
            .with_idempotency_key("order-123");

        assert_eq!(options.queue, "gpu-workers");
        assert_eq!(options.priority_seconds, 10);
        assert_eq!(options.idempotency_key, Some("order-123".to_string()));
    }

    #[test]
    fn test_start_workflow_options_with_version() {
        let options = StartWorkflowOptions::new().with_workflow_version("2.0.0");

        assert_eq!(options.workflow_version, Some("2.0.0".to_string()));
    }

    #[test]
    fn test_start_workflow_result() {
        let result = StartWorkflowResult {
            workflow_execution_id: Uuid::new_v4(),
            idempotency_key_used: true,
            idempotency_key_new: false,
        };

        assert!(result.idempotency_key_used);
        assert!(!result.idempotency_key_new);
    }

    #[test]
    fn test_start_workflow_options_with_labels() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("environment".to_string(), "production".to_string());
        labels.insert("priority".to_string(), "high".to_string());

        let options = StartWorkflowOptions::new().with_labels(labels.clone());

        assert_eq!(options.labels.len(), 2);
        assert_eq!(
            options.labels.get("environment"),
            Some(&"production".to_string())
        );
    }

    #[test]
    fn test_start_workflow_options_with_idempotency_ttl() {
        let options = StartWorkflowOptions::new()
            .with_idempotency_key("key-123")
            .with_idempotency_key_ttl(Duration::from_secs(3600));

        assert_eq!(options.idempotency_key, Some("key-123".to_string()));
        assert_eq!(options.idempotency_key_ttl, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn test_start_workflow_options_full_builder() {
        let options = StartWorkflowOptions::new()
            .with_queue("ml-workers")
            .with_priority(5)
            .with_workflow_version("1.2.3")
            .with_idempotency_key("order-456")
            .with_idempotency_key_ttl(Duration::from_secs(7200));

        assert_eq!(options.queue, "ml-workers");
        assert_eq!(options.priority_seconds, 5);
        assert_eq!(options.workflow_version, Some("1.2.3".to_string()));
        assert_eq!(options.idempotency_key, Some("order-456".to_string()));
        assert_eq!(options.idempotency_key_ttl, Some(Duration::from_secs(7200)));
    }

    #[test]
    fn test_timeout_error_format() {
        let err = FlovynError::Timeout("Operation timed out".to_string());
        assert_eq!(err.to_string(), "Timeout: Operation timed out");
    }

    #[test]
    fn test_workflow_failed_error_format() {
        let err = FlovynError::WorkflowFailed("Validation failed".to_string());
        assert_eq!(err.to_string(), "Workflow failed: Validation failed");
    }

    #[test]
    fn test_workflow_cancelled_error_format() {
        let err = FlovynError::WorkflowCancelled("User requested cancellation".to_string());
        assert_eq!(
            err.to_string(),
            "Workflow cancelled: User requested cancellation"
        );
    }
}
