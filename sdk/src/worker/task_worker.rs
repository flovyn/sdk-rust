//! TaskExecutorWorker - Polling service for task execution

use crate::client::{TaskExecutionClient, WorkerLifecycleClient, WorkflowDispatch};
use crate::error::{FlovynError, Result};
use crate::task::executor::{
    TaskExecutionResult, TaskExecutor, TaskExecutorCallbacks, TaskExecutorConfig,
};
use crate::task::registry::TaskRegistry;
use crate::telemetry::{task_execute_span, SpanCollector};
use crate::worker::lifecycle::{HookChain, ReconnectionStrategy, StopReason, WorkerInternals};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, Notify};
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Configuration for TaskExecutorWorker
#[derive(Clone)]
pub struct TaskWorkerConfig {
    /// Unique worker identifier
    pub worker_id: String,
    /// Tenant ID
    pub tenant_id: Uuid,
    /// Queue to poll tasks from
    pub queue: String,
    /// Long polling timeout
    pub poll_timeout: Duration,
    /// Interval to wait before polling again when no task is available
    pub no_work_backoff: Duration,
    /// Worker labels for task routing
    pub worker_labels: std::collections::HashMap<String, String>,
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Human-readable worker name for registration
    pub worker_name: Option<String>,
    /// Worker version for registration
    pub worker_version: String,
    /// Space ID (None = tenant-level)
    pub space_id: Option<Uuid>,
    /// Enable automatic worker registration on startup
    pub enable_auto_registration: bool,
    /// Worker token for gRPC authentication
    pub worker_token: String,
    /// Enable telemetry (task.execute spans)
    pub enable_telemetry: bool,
    /// Worker lifecycle hooks
    pub lifecycle_hooks: HookChain,
    /// Reconnection strategy for connection recovery
    pub reconnection_strategy: ReconnectionStrategy,
    /// Pre-registered server worker ID (from unified registration)
    /// If set, skip auto-registration and use this ID
    pub server_worker_id: Option<Uuid>,
}

impl Default for TaskWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: Uuid::new_v4().to_string(),
            tenant_id: Uuid::nil(),
            queue: "default".to_string(),
            poll_timeout: Duration::from_secs(60),
            no_work_backoff: Duration::from_millis(100),
            worker_labels: std::collections::HashMap::new(),
            heartbeat_interval: Duration::from_secs(30),
            worker_name: None,
            worker_version: "1.0.0".to_string(),
            space_id: None,
            enable_auto_registration: true,
            worker_token: String::new(),
            enable_telemetry: false,
            lifecycle_hooks: HookChain::new(),
            reconnection_strategy: ReconnectionStrategy::default(),
            server_worker_id: None,
        }
    }
}

impl std::fmt::Debug for TaskWorkerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskWorkerConfig")
            .field("worker_id", &self.worker_id)
            .field("tenant_id", &self.tenant_id)
            .field("queue", &self.queue)
            .field("poll_timeout", &self.poll_timeout)
            .field("worker_labels", &self.worker_labels)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("worker_name", &self.worker_name)
            .field("worker_version", &self.worker_version)
            .field("space_id", &self.space_id)
            .field("enable_auto_registration", &self.enable_auto_registration)
            .field("worker_token", &"<redacted>")
            .field("enable_telemetry", &self.enable_telemetry)
            .field("lifecycle_hooks_count", &self.lifecycle_hooks.len())
            .field("reconnection_strategy", &self.reconnection_strategy)
            .finish()
    }
}

/// Worker service that polls for tasks and executes them
pub struct TaskExecutorWorker {
    config: TaskWorkerConfig,
    registry: Arc<TaskRegistry>,
    executor: TaskExecutor,
    client: TaskExecutionClient,
    channel: Channel,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    /// Server-assigned worker ID from registration
    server_worker_id: Option<Uuid>,
    /// Notify when worker is ready
    ready_notify: Arc<Notify>,
    /// External shutdown signal receiver
    external_shutdown_rx: Option<watch::Receiver<bool>>,
    /// Span collector for telemetry
    span_collector: SpanCollector,
    /// WorkflowDispatch client for reporting spans
    workflow_dispatch: WorkflowDispatch,
    /// Shared internal state for lifecycle tracking
    internals: Arc<WorkerInternals>,
}

impl TaskExecutorWorker {
    /// Create a new task worker with the given configuration
    pub fn new(config: TaskWorkerConfig, registry: Arc<TaskRegistry>, channel: Channel) -> Self {
        let executor_config = TaskExecutorConfig::default();
        let executor = TaskExecutor::new(registry.clone(), executor_config);
        let client = TaskExecutionClient::new(channel.clone(), &config.worker_token);
        let span_collector = SpanCollector::new(config.enable_telemetry);
        let workflow_dispatch = WorkflowDispatch::new(channel.clone(), &config.worker_token);

        // Create worker internals with lifecycle hooks from config
        let internals = Arc::new(WorkerInternals::new(
            config.worker_id.clone(),
            config.worker_name.clone(),
            config.lifecycle_hooks.clone(),
        ));

        Self {
            config,
            registry,
            executor,
            client,
            channel,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            server_worker_id: None,
            ready_notify: Arc::new(Notify::new()),
            external_shutdown_rx: None,
            span_collector,
            workflow_dispatch,
            internals,
        }
    }

    /// Get the worker internals for status/metrics access.
    pub fn internals(&self) -> Arc<WorkerInternals> {
        self.internals.clone()
    }

    /// Set shared worker internals for lifecycle tracking.
    ///
    /// This replaces the internally created WorkerInternals with a shared one,
    /// allowing multiple workers to share the same status, metrics, and events.
    pub fn with_internals(mut self, internals: Arc<WorkerInternals>) -> Self {
        self.internals = internals;
        self
    }

    /// Set the external shutdown signal receiver
    pub fn with_shutdown_signal(mut self, rx: watch::Receiver<bool>) -> Self {
        self.external_shutdown_rx = Some(rx);
        self
    }

    /// Get the ready notification handle
    pub fn ready_notify(&self) -> Arc<Notify> {
        self.ready_notify.clone()
    }

    /// Get the running flag for external shutdown control
    pub fn running_flag(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    /// Check if the worker is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Run heartbeat loop
    async fn heartbeat_loop(
        channel: Channel,
        worker_token: String,
        server_worker_id: Option<Uuid>,
        heartbeat_interval: Duration,
        running: Arc<AtomicBool>,
        internals: Arc<WorkerInternals>,
    ) {
        if server_worker_id.is_none() {
            debug!("No server worker ID, skipping heartbeat loop");
            return;
        }

        let worker_id = server_worker_id.unwrap();
        let mut client = WorkerLifecycleClient::new(channel, &worker_token);

        while running.load(Ordering::SeqCst) {
            tokio::time::sleep(heartbeat_interval).await;

            if !running.load(Ordering::SeqCst) {
                break;
            }

            match client.send_heartbeat(worker_id).await {
                Ok(()) => {
                    debug!("Heartbeat sent successfully");
                    internals.record_heartbeat_success().await;
                }
                Err(e) => {
                    warn!("Failed to send heartbeat: {}", e);
                    internals.record_heartbeat_failure(e.to_string()).await;
                }
            }
        }
    }

    /// Start the worker
    pub async fn start(&mut self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(FlovynError::Other("Worker already running".to_string()));
        }

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Clone external shutdown receiver if present
        let mut external_shutdown_rx = self.external_shutdown_rx.clone();

        info!(
            worker_id = %self.config.worker_id,
            tenant_id = %self.config.tenant_id,
            queue = %self.config.queue,
            "Starting task worker"
        );

        // Only emit Starting event if we're doing auto-registration
        // (unified registration already set the status to Running)
        if self.config.enable_auto_registration {
            self.internals.record_starting().await;
        }

        // Use the server_worker_id from config (set by unified registration in FlovynClient)
        self.server_worker_id = self.config.server_worker_id;

        // Start heartbeat loop in background
        let heartbeat_running = self.running.clone();
        let heartbeat_channel = self.channel.clone();
        let heartbeat_worker_token = self.config.worker_token.clone();
        let heartbeat_worker_id = self.server_worker_id;
        let heartbeat_interval = self.config.heartbeat_interval;
        let heartbeat_internals = self.internals.clone();

        tokio::spawn(async move {
            Self::heartbeat_loop(
                heartbeat_channel,
                heartbeat_worker_token,
                heartbeat_worker_id,
                heartbeat_interval,
                heartbeat_running,
                heartbeat_internals,
            )
            .await;
        });

        // Signal that worker is ready
        // Use notify_one() instead of notify_waiters() to store a permit
        // that can be consumed even if await_ready() hasn't been called yet
        self.ready_notify.notify_one();

        // Emit Ready event
        self.internals.record_ready(self.server_worker_id).await;

        // Polling loop with reconnection and pause support
        let mut reconnect_attempt: u32 = 0;
        let reconnection_strategy = self.config.reconnection_strategy.clone();
        let mut pause_rx = self.internals.pause_receiver();

        while self.running.load(Ordering::SeqCst) {
            // Check if paused - if so, wait for resume
            if *pause_rx.borrow() {
                debug!("Task worker is paused, waiting for resume signal");
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!("Received shutdown signal while paused");
                        break;
                    }
                    _ = async {
                        if let Some(ref mut rx) = external_shutdown_rx {
                            let _ = rx.changed().await;
                        } else {
                            std::future::pending::<()>().await
                        }
                    } => {
                        debug!("Received external shutdown signal while paused");
                        break;
                    }
                    _ = pause_rx.changed() => {
                        // Pause state changed - loop back to check
                        continue;
                    }
                }
            }

            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!("Received shutdown signal");
                    break;
                }
                _ = async {
                    if let Some(ref mut rx) = external_shutdown_rx {
                        let _ = rx.changed().await;
                    } else {
                        std::future::pending::<()>().await
                    }
                } => {
                    debug!("Received external shutdown signal");
                    break;
                }
                _ = pause_rx.changed() => {
                    // Pause state changed - loop back to check
                    continue;
                }
                result = self.poll_and_execute() => {
                    if let Err(e) = result {
                        // Check if it's a connection error
                        let is_connection_error = e.to_string().contains("UNAVAILABLE")
                            || e.to_string().contains("Connection refused");

                        if is_connection_error {
                            warn!("Server unavailable (attempt {}), will retry: {}", reconnect_attempt + 1, e);

                            // Record disconnected state on first failure
                            if reconnect_attempt == 0 {
                                self.internals.record_disconnected(e.to_string()).await;
                            }

                            // Calculate delay using reconnection strategy
                            match reconnection_strategy.calculate_delay(reconnect_attempt) {
                                Some(delay) => {
                                    reconnect_attempt += 1;
                                    self.internals.record_reconnecting(reconnect_attempt, Some(e.to_string())).await;
                                    debug!("Waiting {:?} before reconnection attempt {}", delay, reconnect_attempt);
                                    tokio::time::sleep(delay).await;
                                }
                                None => {
                                    // Max attempts reached
                                    error!("Max reconnection attempts ({}) reached, stopping worker", reconnect_attempt);
                                    break;
                                }
                            }
                        } else {
                            error!("Polling loop error: {}", e);
                            // Non-connection error, use short delay
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    } else if reconnect_attempt > 0 {
                        // Successfully reconnected
                        info!("Successfully reconnected to server after {} attempts", reconnect_attempt);
                        self.internals.record_reconnected(self.server_worker_id).await;
                        reconnect_attempt = 0;
                    }
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);

        // Emit Stopped event
        self.internals.record_stopped(StopReason::Graceful).await;

        info!("Task worker stopped");
        Ok(())
    }

    /// Stop the worker gracefully
    pub fn stop(&mut self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return; // Already stopped
        }

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }
    }

    /// Poll for a task and execute it
    async fn poll_and_execute(&mut self) -> Result<()> {
        // Poll for task
        let task_info = self
            .client
            .poll_task(
                &self.config.worker_id,
                &self.config.tenant_id.to_string(),
                &self.config.queue,
                self.config.poll_timeout,
            )
            .await?;

        let task_info = match task_info {
            Some(info) => info,
            None => {
                // No task available - wait before next poll to avoid tight loop
                tokio::time::sleep(self.config.no_work_backoff).await;
                return Ok(());
            }
        };

        let task_execution_id = task_info.id;
        let task_type = &task_info.task_type;
        let workflow_id = task_info.workflow_execution_id;

        // Record work received
        self.internals
            .record_work_received(crate::worker::lifecycle::WorkType::Task, task_execution_id)
            .await;

        let start_time = std::time::Instant::now();

        debug!(
            task_execution_id = %task_execution_id,
            task_type = %task_type,
            attempt = task_info.execution_count,
            "Executing task"
        );

        // Create task.execute span for telemetry
        let span = if self.span_collector.is_enabled() {
            let wf_id = workflow_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "standalone".to_string());
            Some(
                task_execute_span(&wf_id, &task_execution_id.to_string(), task_type)
                    .with_attribute("attempt", &task_info.execution_count.to_string()),
            )
        } else {
            None
        };

        // Create callbacks for progress/log reporting via gRPC
        let callbacks = self.create_task_callbacks(task_execution_id, workflow_id);

        // Reset stream sequence for this task execution
        self.client.reset_stream_sequence();

        // Execute the task using TaskExecutor with callbacks
        let result = self
            .executor
            .execute_with_callbacks(
                task_execution_id,
                task_type,
                task_info.input,
                task_info.execution_count,
                callbacks,
            )
            .await;

        // Finish and record the span
        if let Some(span) = span {
            let finished_span = match &result {
                TaskExecutionResult::Completed { .. } => span.finish(),
                TaskExecutionResult::Failed { error_message, .. } => {
                    span.finish_with_error("TaskFailed", error_message)
                }
                TaskExecutionResult::Cancelled => {
                    span.finish_with_error("TaskCancelled", "Task was cancelled")
                }
                TaskExecutionResult::TimedOut => {
                    span.finish_with_error("TaskTimeout", "Task execution timed out")
                }
            };
            self.span_collector.record(finished_span);

            // Flush spans to server
            self.span_collector.flush(&mut self.workflow_dispatch).await;
        }

        // Record work completed/failed
        let duration = start_time.elapsed();
        let is_success = matches!(result, TaskExecutionResult::Completed { .. });

        // Report result to server
        self.report_result(task_execution_id, result).await?;

        // Record work completion metrics
        if is_success {
            self.internals
                .record_work_completed(
                    crate::worker::lifecycle::WorkType::Task,
                    task_execution_id,
                    duration,
                )
                .await;
        } else {
            self.internals
                .record_work_failed(
                    crate::worker::lifecycle::WorkType::Task,
                    task_execution_id,
                    "Task execution failed".to_string(),
                    duration,
                )
                .await;
        }

        Ok(())
    }

    /// Report task execution result to the server
    async fn report_result(
        &mut self,
        task_execution_id: Uuid,
        result: TaskExecutionResult,
    ) -> Result<()> {
        match result {
            TaskExecutionResult::Completed { output } => {
                debug!(
                    task_execution_id = %task_execution_id,
                    "Task completed successfully"
                );
                self.client.complete_task(task_execution_id, output).await?;
            }
            TaskExecutionResult::Failed {
                error_message,
                error_type,
                is_retryable,
            } => {
                debug!(
                    task_execution_id = %task_execution_id,
                    error = %error_message,
                    error_type = ?error_type,
                    is_retryable = is_retryable,
                    "Task failed"
                );
                self.client
                    .fail_task(task_execution_id, &error_message)
                    .await?;
            }
            TaskExecutionResult::Cancelled => {
                debug!(
                    task_execution_id = %task_execution_id,
                    "Task was cancelled"
                );
                self.client
                    .fail_task(task_execution_id, "Task cancelled")
                    .await?;
            }
            TaskExecutionResult::TimedOut => {
                debug!(
                    task_execution_id = %task_execution_id,
                    "Task timed out"
                );
                self.client
                    .fail_task(task_execution_id, "Task execution timed out")
                    .await?;
            }
        }

        Ok(())
    }

    /// Get the task registry
    pub fn registry(&self) -> &TaskRegistry {
        &self.registry
    }

    /// Create callbacks for reporting progress and logs to the server via gRPC.
    ///
    /// Uses a single background task with a channel to batch and forward events,
    /// rather than spawning a task per callback call.
    fn create_task_callbacks(
        &self,
        task_execution_id: Uuid,
        _workflow_execution_id: Option<Uuid>,
    ) -> TaskExecutorCallbacks {
        // Create a channel for sending events to a single background forwarder task
        let (tx, mut rx) = mpsc::channel::<TaskEvent>(100);

        // Clone channel and token for the forwarder task
        let channel = self.channel.clone();
        let worker_token = self.config.worker_token.clone();

        // Spawn a single background task to forward events to the server
        tokio::spawn(async move {
            let mut client = TaskExecutionClient::new(channel, &worker_token);
            debug!(task_id = %task_execution_id, "Progress forwarder task started");

            while let Some(event) = rx.recv().await {
                match event {
                    TaskEvent::Progress { progress, message } => {
                        debug!(
                            task_id = %task_execution_id,
                            progress = %progress,
                            message = ?message,
                            "Forwarding progress to server via gRPC"
                        );
                        match client
                            .report_progress(task_execution_id, progress, message.as_deref())
                            .await
                        {
                            Ok(_) => {
                                debug!(
                                    task_id = %task_execution_id,
                                    progress = %progress,
                                    "Progress reported successfully"
                                );
                            }
                            Err(e) => {
                                debug!(error = %e, "Failed to report progress (non-critical)");
                            }
                        }
                    }
                    TaskEvent::Log { level, message } => {
                        if let Err(e) = client
                            .log_message(task_execution_id, &level, &message)
                            .await
                        {
                            debug!(error = %e, "Failed to send log message (non-critical)");
                        }
                    }
                }
            }
            debug!(task_id = %task_execution_id, "Progress forwarder task ended");
        });

        // Create progress callback that sends to channel
        let progress_tx = tx.clone();
        let on_progress: Box<dyn Fn(f64, Option<String>) + Send + Sync> =
            Box::new(move |progress, message| {
                match progress_tx.try_send(TaskEvent::Progress {
                    progress,
                    message: message.clone(),
                }) {
                    Ok(_) => {
                        tracing::debug!(
                            progress = %progress,
                            message = ?message,
                            "Progress event sent to channel"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            progress = %progress,
                            "Failed to send progress to channel"
                        );
                    }
                }
            });

        // Create log callback that sends to channel
        let log_tx = tx;
        let on_log: Box<dyn Fn(String, String) + Send + Sync> = Box::new(move |level, message| {
            let _ = log_tx.try_send(TaskEvent::Log { level, message });
        });

        TaskExecutorCallbacks {
            on_progress: Some(on_progress),
            on_log: Some(on_log),
            on_heartbeat: None, // Heartbeats are handled by the separate heartbeat loop
            on_stream: None,    // Stream events can be added if needed
        }
    }
}

/// Events sent from task callbacks to the forwarder task
enum TaskEvent {
    Progress {
        progress: f64,
        message: Option<String>,
    },
    Log {
        level: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_worker_config_default() {
        let config = TaskWorkerConfig::default();
        assert_eq!(config.queue, "default");
        assert_eq!(config.poll_timeout, Duration::from_secs(60));
        assert!(config.worker_labels.is_empty());
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.worker_version, "1.0.0");
        assert!(config.enable_auto_registration);
        assert!(config.worker_name.is_none());
        assert!(config.space_id.is_none());
        assert!(!config.enable_telemetry);
    }

    #[test]
    fn test_task_worker_config_custom() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("gpu".to_string(), "true".to_string());

        let config = TaskWorkerConfig {
            worker_id: "task-worker-1".to_string(),
            tenant_id: Uuid::new_v4(),
            queue: "gpu-tasks".to_string(),
            poll_timeout: Duration::from_secs(30),
            no_work_backoff: Duration::from_millis(100),
            worker_labels: labels,
            heartbeat_interval: Duration::from_secs(15),
            worker_name: Some("GPU Task Worker".to_string()),
            worker_version: "2.0.0".to_string(),
            space_id: Some(Uuid::new_v4()),
            enable_auto_registration: false,
            worker_token: "test-token".to_string(),
            enable_telemetry: true,
            lifecycle_hooks: HookChain::new(),
            reconnection_strategy: ReconnectionStrategy::fixed(Duration::from_secs(5)),
            server_worker_id: None,
        };

        assert_eq!(config.worker_id, "task-worker-1");
        assert_eq!(config.queue, "gpu-tasks");
        assert_eq!(config.worker_labels.get("gpu"), Some(&"true".to_string()));
        assert_eq!(config.worker_name, Some("GPU Task Worker".to_string()));
        assert_eq!(config.worker_version, "2.0.0");
        assert!(!config.enable_auto_registration);
        assert!(config.enable_telemetry);
    }
}
