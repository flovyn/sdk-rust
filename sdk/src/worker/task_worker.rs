//! TaskExecutorWorker - Polling service for task execution

use crate::client::worker_lifecycle::{WorkerLifecycleClient, WorkerType};
use crate::client::TaskExecutionClient;
use crate::error::{FlovynError, Result};
use crate::task::executor::{TaskExecutionResult, TaskExecutor, TaskExecutorConfig};
use crate::task::registry::TaskRegistry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Configuration for TaskExecutorWorker
#[derive(Debug, Clone)]
pub struct TaskWorkerConfig {
    /// Unique worker identifier
    pub worker_id: String,
    /// Tenant ID
    pub tenant_id: Uuid,
    /// Queue to poll tasks from
    pub queue: String,
    /// Long polling timeout
    pub poll_timeout: Duration,
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
}

impl Default for TaskWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: Uuid::new_v4().to_string(),
            tenant_id: Uuid::nil(),
            queue: "default".to_string(),
            poll_timeout: Duration::from_secs(60),
            worker_labels: std::collections::HashMap::new(),
            heartbeat_interval: Duration::from_secs(30),
            worker_name: None,
            worker_version: "1.0.0".to_string(),
            space_id: None,
            enable_auto_registration: true,
            worker_token: String::new(),
        }
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
}

impl TaskExecutorWorker {
    /// Create a new task worker with the given configuration
    pub fn new(config: TaskWorkerConfig, registry: Arc<TaskRegistry>, channel: Channel) -> Self {
        let executor_config = TaskExecutorConfig::default();
        let executor = TaskExecutor::new(registry.clone(), executor_config);
        let client = TaskExecutionClient::new(channel.clone(), &config.worker_token);

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
        }
    }

    /// Get the ready notification handle
    pub fn ready_notify(&self) -> Arc<Notify> {
        self.ready_notify.clone()
    }

    /// Check if the worker is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Register worker with the server
    async fn register_with_server(&mut self) -> Result<()> {
        if !self.config.enable_auto_registration {
            debug!("Auto-registration disabled, skipping task worker registration");
            return Ok(());
        }

        let worker_name = self
            .config
            .worker_name
            .clone()
            .unwrap_or_else(|| self.config.worker_id.clone());

        let tasks = self.registry.get_all_metadata();

        info!(
            worker_name = %worker_name,
            task_count = tasks.len(),
            "Registering task worker with server"
        );

        let mut lifecycle_client = WorkerLifecycleClient::new(self.channel.clone(), &self.config.worker_token);

        let result = lifecycle_client
            .register_worker(
                &worker_name,
                &self.config.worker_version,
                WorkerType::Task,
                self.config.tenant_id,
                self.config.space_id,
                vec![], // No workflows for task worker
                tasks,
            )
            .await?;

        if result.success {
            self.server_worker_id = Some(result.worker_id);
            info!(
                server_worker_id = %result.worker_id,
                "Task worker registered successfully"
            );
        } else {
            warn!(
                error = ?result.error,
                task_conflicts = result.task_conflicts.len(),
                "Task worker registration failed or had conflicts"
            );
        }

        Ok(())
    }

    /// Run heartbeat loop
    async fn heartbeat_loop(
        channel: Channel,
        worker_token: String,
        server_worker_id: Option<Uuid>,
        heartbeat_interval: Duration,
        running: Arc<AtomicBool>,
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

            if let Err(e) = client.send_heartbeat(worker_id).await {
                warn!("Failed to send heartbeat: {}", e);
            } else {
                debug!("Heartbeat sent successfully");
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

        info!(
            worker_id = %self.config.worker_id,
            tenant_id = %self.config.tenant_id,
            queue = %self.config.queue,
            "Starting task worker"
        );

        // Register with server
        if let Err(e) = self.register_with_server().await {
            warn!("Failed to register task worker: {}", e);
            // Continue anyway - registration is best-effort
        }

        // Start heartbeat loop in background
        let heartbeat_running = self.running.clone();
        let heartbeat_channel = self.channel.clone();
        let heartbeat_worker_token = self.config.worker_token.clone();
        let heartbeat_worker_id = self.server_worker_id;
        let heartbeat_interval = self.config.heartbeat_interval;

        tokio::spawn(async move {
            Self::heartbeat_loop(
                heartbeat_channel,
                heartbeat_worker_token,
                heartbeat_worker_id,
                heartbeat_interval,
                heartbeat_running,
            )
            .await;
        });

        // Signal that worker is ready
        self.ready_notify.notify_waiters();

        // Polling loop
        while self.running.load(Ordering::SeqCst) {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!("Received shutdown signal");
                    break;
                }
                result = self.poll_and_execute() => {
                    if let Err(e) = result {
                        // Check if it's a connection error
                        let is_connection_error = e.to_string().contains("UNAVAILABLE")
                            || e.to_string().contains("Connection refused");

                        if is_connection_error {
                            warn!("Server unavailable, will retry: {}", e);
                        } else {
                            error!("Polling loop error: {}", e);
                        }

                        // Wait before retrying
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);
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
            None => return Ok(()), // No task available
        };

        let task_execution_id = task_info.id;
        let task_type = &task_info.task_type;

        debug!(
            task_execution_id = %task_execution_id,
            task_type = %task_type,
            attempt = task_info.execution_count,
            "Executing task"
        );

        // Execute the task using TaskExecutor
        let result = self
            .executor
            .execute(
                task_execution_id,
                task_type,
                task_info.input,
                task_info.execution_count,
            )
            .await;

        // Report result to server
        self.report_result(task_execution_id, result).await?;

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
            worker_labels: labels,
            heartbeat_interval: Duration::from_secs(15),
            worker_name: Some("GPU Task Worker".to_string()),
            worker_version: "2.0.0".to_string(),
            space_id: Some(Uuid::new_v4()),
            enable_auto_registration: false,
        };

        assert_eq!(config.worker_id, "task-worker-1");
        assert_eq!(config.queue, "gpu-tasks");
        assert_eq!(config.worker_labels.get("gpu"), Some(&"true".to_string()));
        assert_eq!(config.worker_name, Some("GPU Task Worker".to_string()));
        assert_eq!(config.worker_version, "2.0.0");
        assert!(!config.enable_auto_registration);
    }
}
