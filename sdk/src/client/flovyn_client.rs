//! FlovynClient - Main entry point for the Flovyn SDK
//!
//! The FlovynClient provides a high-level API for interacting with the Flovyn server,
//! including starting workflows, managing workers, and querying workflow state.

use crate::client::builder::FlovynClientBuilder;
use crate::client::hook::WorkflowHook;
use crate::client::workflow_query::WorkflowQueryClient;
use crate::client::{TaskExecutionClient, WorkflowDispatch};
use crate::config::FlovynClientConfig;
use crate::error::{FlovynError, Result};
use crate::generated::flovyn_v1;
use crate::task::registry::TaskRegistry;
use crate::worker::registry::WorkflowRegistry;
use crate::worker::task_worker::{TaskExecutorWorker, TaskWorkerConfig};
use crate::worker::workflow_worker::{WorkflowExecutorWorker, WorkflowWorkerConfig};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Notify};
use tonic::transport::Channel;
use tracing::{debug, error, info};
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
    pub task_queue: String,
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
            task_queue: "default".to_string(),
            ..Default::default()
        }
    }

    /// Set labels
    pub fn with_labels(mut self, labels: std::collections::HashMap<String, String>) -> Self {
        self.labels = labels;
        self
    }

    /// Set task queue
    pub fn with_task_queue(mut self, queue: impl Into<String>) -> Self {
        self.task_queue = queue.into();
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
    pub(crate) task_queue: String,
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
    pub fn task_queue(&self) -> &str {
        &self.task_queue
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
            task_queue = %self.task_queue,
            "Starting FlovynClient"
        );

        // Create shutdown channel for immediate shutdown signaling
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Start workflow worker if there are registered workflows
        let (workflow_handle, workflow_ready, workflow_running) =
            if self.workflow_registry.has_registrations() {
                let config = WorkflowWorkerConfig {
                    worker_id: self.worker_id.clone(),
                    tenant_id: self.tenant_id,
                    task_queue: self.task_queue.clone(),
                    poll_timeout: self.poll_timeout,
                    max_concurrent: self.config.workflow_config.max_concurrent,
                    heartbeat_interval: self.heartbeat_interval,
                    worker_name: self.worker_name.clone(),
                    worker_version: self.worker_version.clone(),
                    space_id: self.space_id,
                    enable_auto_registration: self.enable_auto_registration,
                    enable_notifications: true,
                    worker_token: self.worker_token.clone(),
                    enable_telemetry: self.enable_telemetry,
                };

                let mut worker = WorkflowExecutorWorker::new(
                    config,
                    self.workflow_registry.clone(),
                    self.channel.clone(),
                )
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
                worker_id: format!("{}-task", self.worker_id),
                tenant_id: self.tenant_id,
                queue: self.task_queue.clone(),
                poll_timeout: self.poll_timeout,
                worker_labels: self.config.worker_labels.clone(),
                heartbeat_interval: self.heartbeat_interval,
                worker_name: self.worker_name.clone().map(|n| format!("{}-task", n)),
                worker_version: self.worker_version.clone(),
                space_id: self.space_id,
                enable_auto_registration: self.enable_auto_registration,
                worker_token: self.worker_token.clone(),
                enable_telemetry: self.enable_telemetry,
            };

            let worker =
                TaskExecutorWorker::new(config, self.task_registry.clone(), self.channel.clone())
                    .with_shutdown_signal(shutdown_rx.clone());

            let ready_notify = worker.ready_notify();
            let running_flag = worker.running_flag();
            let mut worker = worker;

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

        Ok(WorkerHandle {
            shutdown_tx,
            workflow_running,
            task_running,
            workflow_handle,
            task_handle,
            workflow_ready,
            task_ready,
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
                Some(&options.task_queue),
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
        let promise_id = format!("{}:{}", workflow_execution_id, promise_name);
        let value_bytes = serde_json::to_vec(&value)?;

        let request = flovyn_v1::ResolvePromiseRequest {
            promise_id,
            value: value_bytes,
        };

        let interceptor = crate::client::auth::WorkerTokenInterceptor::new(&self.worker_token);
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
        let promise_id = format!("{}:{}", workflow_execution_id, promise_name);

        let request = flovyn_v1::RejectPromiseRequest {
            promise_id,
            error: error.to_string(),
        };

        let interceptor = crate::client::auth::WorkerTokenInterceptor::new(&self.worker_token);
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
        client.get_events(workflow_execution_id, None).await
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
        client
            .query(workflow_execution_id, query_name, params)
            .await
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
        client
            .query_typed(workflow_execution_id, query_name, params)
            .await
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
        client.get_state(task_execution_id, key).await
    }

    /// Get all task state keys
    ///
    /// Retrieves all state keys for a task execution.
    pub async fn get_task_state_keys(&self, task_execution_id: Uuid) -> Result<Vec<String>> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        client.get_state_keys(task_execution_id).await
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
        client.set_state(task_execution_id, key, value).await
    }

    /// Clear task state value
    ///
    /// Clears a state value for a task execution.
    pub async fn clear_task_state(&self, task_execution_id: Uuid, key: &str) -> Result<()> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        client.clear_state(task_execution_id, key).await
    }

    /// Clear all task state
    ///
    /// Clears all state values for a task execution.
    pub async fn clear_all_task_state(&self, task_execution_id: Uuid) -> Result<()> {
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        client.clear_all_state(task_execution_id).await
    }

    /// Get the workflow registry
    pub fn workflow_registry(&self) -> &WorkflowRegistry {
        &self.workflow_registry
    }

    /// Get the task registry
    pub fn task_registry(&self) -> &TaskRegistry {
        &self.task_registry
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_workflow_options_default() {
        let options = StartWorkflowOptions::new();
        assert_eq!(options.task_queue, "default");
        assert_eq!(options.priority_seconds, 0);
        assert!(options.idempotency_key.is_none());
    }

    #[test]
    fn test_start_workflow_options_builder() {
        let options = StartWorkflowOptions::new()
            .with_task_queue("gpu-workers")
            .with_priority(10)
            .with_idempotency_key("order-123");

        assert_eq!(options.task_queue, "gpu-workers");
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
            .with_task_queue("ml-workers")
            .with_priority(5)
            .with_workflow_version("1.2.3")
            .with_idempotency_key("order-456")
            .with_idempotency_key_ttl(Duration::from_secs(7200));

        assert_eq!(options.task_queue, "ml-workers");
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
