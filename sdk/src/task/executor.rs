//! TaskExecutor - Task execution engine with supervision

use crate::error::FlovynError;
use crate::task::context_impl::TaskContextImpl;
use crate::task::definition::RetryConfig;
use crate::task::registry::TaskRegistry;
use crate::task::streaming::StreamEvent;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;
use uuid::Uuid;

/// Configuration for task execution
#[derive(Debug, Clone)]
pub struct TaskExecutorConfig {
    /// Default timeout for task execution (if task doesn't specify one)
    pub default_timeout: Duration,
    /// Default heartbeat interval
    pub default_heartbeat_interval: Duration,
}

impl Default for TaskExecutorConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(300), // 5 minutes
            default_heartbeat_interval: Duration::from_secs(30),
        }
    }
}

/// Result of task execution
#[derive(Debug, Clone)]
pub enum TaskExecutionResult {
    /// Task completed successfully
    Completed { output: Value },
    /// Task failed
    Failed {
        error_message: String,
        error_type: Option<String>,
        is_retryable: bool,
    },
    /// Task was cancelled
    Cancelled,
    /// Task timed out
    TimedOut,
}

/// Callbacks for task execution status
#[derive(Default)]
pub struct TaskExecutorCallbacks {
    /// Called when progress is reported
    #[allow(clippy::type_complexity)]
    pub on_progress: Option<Box<dyn Fn(f64, Option<String>) + Send + Sync>>,
    /// Called when a log message is sent
    pub on_log: Option<Box<dyn Fn(String, String) + Send + Sync>>,
    /// Called for heartbeat
    pub on_heartbeat: Option<Box<dyn Fn() + Send + Sync>>,
    /// Called when a stream event is emitted
    pub on_stream: Option<Box<dyn Fn(StreamEvent) + Send + Sync>>,
}

impl std::fmt::Debug for TaskExecutorCallbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskExecutorCallbacks")
            .field("on_progress", &self.on_progress.is_some())
            .field("on_log", &self.on_log.is_some())
            .field("on_heartbeat", &self.on_heartbeat.is_some())
            .field("on_stream", &self.on_stream.is_some())
            .finish()
    }
}

/// Task executor that handles task execution with supervision
pub struct TaskExecutor {
    /// Task registry
    registry: Arc<TaskRegistry>,
    /// Configuration
    config: TaskExecutorConfig,
}

impl TaskExecutor {
    /// Create a new TaskExecutor
    pub fn new(registry: Arc<TaskRegistry>, config: TaskExecutorConfig) -> Self {
        Self { registry, config }
    }

    /// Execute a task by kind
    pub async fn execute(
        &self,
        task_execution_id: Uuid,
        task_kind: &str,
        input: Value,
        attempt: u32,
    ) -> TaskExecutionResult {
        self.execute_with_callbacks(
            task_execution_id,
            task_kind,
            input,
            attempt,
            TaskExecutorCallbacks::default(),
        )
        .await
    }

    /// Execute a task with callbacks for progress/log/heartbeat
    pub async fn execute_with_callbacks(
        &self,
        task_execution_id: Uuid,
        task_kind: &str,
        input: Value,
        attempt: u32,
        callbacks: TaskExecutorCallbacks,
    ) -> TaskExecutionResult {
        // Look up the task in the registry
        let registered = match self.registry.get(task_kind) {
            Some(r) => r,
            None => {
                return TaskExecutionResult::Failed {
                    error_message: format!("Task kind not found: {}", task_kind),
                    error_type: Some("TaskNotFound".to_string()),
                    is_retryable: false,
                };
            }
        };

        // Get task configuration
        let task_timeout = registered
            .metadata
            .timeout_seconds
            .map(|s| Duration::from_secs(s as u64))
            .unwrap_or(self.config.default_timeout);

        let heartbeat_interval = registered
            .metadata
            .heartbeat_timeout_seconds
            .map(|s| Duration::from_secs(s as u64 / 2))
            .unwrap_or(self.config.default_heartbeat_interval);

        // Create cancellation flag
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = Arc::clone(&cancelled);

        // Create task context with callbacks
        let ctx = Self::create_context(
            task_execution_id,
            attempt,
            callbacks,
            Arc::clone(&cancelled),
        );

        // Create a channel to signal task completion
        let (complete_tx, _complete_rx) = oneshot::channel::<()>();

        // Start heartbeat monitor
        let heartbeat_handle = self.start_heartbeat_monitor(
            Arc::clone(&cancelled),
            heartbeat_interval,
            ctx.cancelled_flag(),
        );

        // Execute task with timeout
        let result = timeout(task_timeout, async {
            registered.execute(Arc::new(ctx), input).await
        })
        .await;

        // Stop heartbeat monitor
        drop(complete_tx);
        heartbeat_handle.abort();

        // Process result
        match result {
            Ok(Ok(output)) => TaskExecutionResult::Completed { output },
            Ok(Err(e)) => {
                // Check if it was a cancellation
                if cancelled_clone.load(Ordering::SeqCst) {
                    return TaskExecutionResult::Cancelled;
                }

                let (error_type, is_retryable) = Self::classify_error(&e);
                TaskExecutionResult::Failed {
                    error_message: e.to_string(),
                    error_type,
                    is_retryable,
                }
            }
            Err(_) => {
                // Timeout
                TaskExecutionResult::TimedOut
            }
        }
    }

    fn create_context(
        task_execution_id: Uuid,
        attempt: u32,
        callbacks: TaskExecutorCallbacks,
        cancelled: Arc<AtomicBool>,
    ) -> TaskContextImpl {
        use crate::task::context::LogLevel;
        use crate::task::context_impl::{
            HeartbeatReporter, LogReporter, ProgressReporter, StreamReporter,
        };

        // Create progress reporter
        let progress_reporter: ProgressReporter = if let Some(on_progress) = callbacks.on_progress {
            let on_progress = Arc::new(on_progress);
            Arc::new(move |progress, msg| {
                let on_progress = Arc::clone(&on_progress);
                Box::pin(async move {
                    on_progress(progress, msg);
                    Ok(())
                })
            })
        } else {
            Arc::new(|_, _| Box::pin(async { Ok(()) }))
        };

        // Create log reporter
        let log_reporter: LogReporter = if let Some(on_log) = callbacks.on_log {
            let on_log = Arc::new(on_log);
            Arc::new(move |level, msg| {
                let on_log = Arc::clone(&on_log);
                let level_str = match level {
                    LogLevel::Debug => "DEBUG",
                    LogLevel::Info => "INFO",
                    LogLevel::Warn => "WARN",
                    LogLevel::Error => "ERROR",
                };
                Box::pin(async move {
                    on_log(level_str.to_string(), msg);
                    Ok(())
                })
            })
        } else {
            Arc::new(|_, _| Box::pin(async { Ok(()) }))
        };

        // Create heartbeat reporter
        let heartbeat_reporter: HeartbeatReporter =
            if let Some(on_heartbeat) = callbacks.on_heartbeat {
                let on_heartbeat = Arc::new(on_heartbeat);
                Arc::new(move || {
                    let on_heartbeat = Arc::clone(&on_heartbeat);
                    Box::pin(async move {
                        on_heartbeat();
                        Ok(())
                    })
                })
            } else {
                Arc::new(|| Box::pin(async { Ok(()) }))
            };

        // Create stream reporter (optional)
        let stream_reporter: Option<StreamReporter> =
            callbacks.on_stream.map(|on_stream| -> StreamReporter {
                let on_stream = Arc::new(on_stream);
                Arc::new(move |event| {
                    let on_stream = Arc::clone(&on_stream);
                    Box::pin(async move {
                        on_stream(event);
                        Ok(())
                    })
                })
            });

        TaskContextImpl::with_all_callbacks(
            task_execution_id,
            attempt,
            progress_reporter,
            log_reporter,
            heartbeat_reporter,
            stream_reporter,
            cancelled,
        )
    }

    fn start_heartbeat_monitor(
        &self,
        cancelled: Arc<AtomicBool>,
        interval: Duration,
        task_cancelled: Arc<AtomicBool>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;

                // Check if task is cancelled or completed
                if cancelled.load(Ordering::SeqCst) || task_cancelled.load(Ordering::SeqCst) {
                    break;
                }

                // Heartbeat logic is handled through callbacks
            }
        })
    }

    fn classify_error(error: &FlovynError) -> (Option<String>, bool) {
        match error {
            FlovynError::TaskCancelled => (Some("CANCELLED".to_string()), false),
            FlovynError::NetworkError(_) => (Some("NETWORK".to_string()), true),
            FlovynError::Grpc(_) => (Some("GRPC".to_string()), true),
            FlovynError::Timeout(_) => (Some("TIMEOUT".to_string()), true),
            FlovynError::InvalidInput(_) => (Some("INVALID_INPUT".to_string()), false),
            FlovynError::InvalidConfiguration(_) => (Some("INVALID_CONFIG".to_string()), false),
            FlovynError::Serialization(_) => (Some("SERIALIZATION".to_string()), false),
            _ => (None, false),
        }
    }

    /// Calculate backoff duration for retry
    pub fn calculate_backoff(config: &RetryConfig, attempt: u32) -> Duration {
        let base_ms = config.initial_backoff.as_millis() as f64;
        let multiplier = config
            .backoff_multiplier
            .powi(attempt.saturating_sub(1) as i32);
        let backoff_ms = base_ms * multiplier;
        let backoff = Duration::from_millis(backoff_ms as u64);
        std::cmp::min(backoff, config.max_backoff)
    }

    /// Check if we should retry based on config and attempt
    pub fn should_retry(config: &RetryConfig, attempt: u32, is_retryable: bool) -> bool {
        is_retryable && attempt < config.max_retries
    }
}

impl std::fmt::Debug for TaskExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskExecutor")
            .field("registry", &self.registry)
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::definition::RetryConfig;
    use crate::task::registry::TaskMetadata;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn test_task_executor_config_default() {
        let config = TaskExecutorConfig::default();
        assert_eq!(config.default_timeout, Duration::from_secs(300));
        assert_eq!(config.default_heartbeat_interval, Duration::from_secs(30));
    }

    #[test]
    fn test_task_execution_result_completed() {
        let result = TaskExecutionResult::Completed {
            output: json!({"status": "ok"}),
        };
        match result {
            TaskExecutionResult::Completed { output } => {
                assert_eq!(output, json!({"status": "ok"}));
            }
            _ => panic!("Expected Completed"),
        }
    }

    #[test]
    fn test_task_execution_result_failed() {
        let result = TaskExecutionResult::Failed {
            error_message: "something went wrong".to_string(),
            error_type: Some("INTERNAL".to_string()),
            is_retryable: true,
        };
        match result {
            TaskExecutionResult::Failed {
                error_message,
                error_type,
                is_retryable,
            } => {
                assert_eq!(error_message, "something went wrong");
                assert_eq!(error_type, Some("INTERNAL".to_string()));
                assert!(is_retryable);
            }
            _ => panic!("Expected Failed"),
        }
    }

    #[test]
    fn test_task_execution_result_cancelled() {
        let result = TaskExecutionResult::Cancelled;
        assert!(matches!(result, TaskExecutionResult::Cancelled));
    }

    #[test]
    fn test_task_execution_result_timed_out() {
        let result = TaskExecutionResult::TimedOut;
        assert!(matches!(result, TaskExecutionResult::TimedOut));
    }

    #[test]
    fn test_calculate_backoff_first_attempt() {
        let config = RetryConfig::default();
        let backoff = TaskExecutor::calculate_backoff(&config, 1);
        assert_eq!(backoff, Duration::from_secs(1));
    }

    #[test]
    fn test_calculate_backoff_second_attempt() {
        let config = RetryConfig::default();
        let backoff = TaskExecutor::calculate_backoff(&config, 2);
        assert_eq!(backoff, Duration::from_secs(2));
    }

    #[test]
    fn test_calculate_backoff_third_attempt() {
        let config = RetryConfig::default();
        let backoff = TaskExecutor::calculate_backoff(&config, 3);
        assert_eq!(backoff, Duration::from_secs(4));
    }

    #[test]
    fn test_calculate_backoff_capped_at_max() {
        let config = RetryConfig {
            max_retries: 10,
            initial_backoff: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(60),
        };
        let backoff = TaskExecutor::calculate_backoff(&config, 10);
        assert_eq!(backoff, Duration::from_secs(60));
    }

    #[test]
    fn test_should_retry_true() {
        let config = RetryConfig::default();
        assert!(TaskExecutor::should_retry(&config, 1, true));
        assert!(TaskExecutor::should_retry(&config, 2, true));
    }

    #[test]
    fn test_should_retry_false_max_reached() {
        let config = RetryConfig::default();
        assert!(!TaskExecutor::should_retry(&config, 3, true)); // max_retries is 3
    }

    #[test]
    fn test_should_retry_false_not_retryable() {
        let config = RetryConfig::default();
        assert!(!TaskExecutor::should_retry(&config, 1, false));
    }

    #[test]
    fn test_classify_error_cancelled() {
        let err = FlovynError::TaskCancelled;
        let (error_type, is_retryable) = TaskExecutor::classify_error(&err);
        assert_eq!(error_type, Some("CANCELLED".to_string()));
        assert!(!is_retryable);
    }

    #[test]
    fn test_classify_error_network() {
        let err = FlovynError::NetworkError("connection failed".to_string());
        let (error_type, is_retryable) = TaskExecutor::classify_error(&err);
        assert_eq!(error_type, Some("NETWORK".to_string()));
        assert!(is_retryable);
    }

    #[test]
    fn test_classify_error_invalid_input() {
        let err = FlovynError::InvalidInput("bad input".to_string());
        let (error_type, is_retryable) = TaskExecutor::classify_error(&err);
        assert_eq!(error_type, Some("INVALID_INPUT".to_string()));
        assert!(!is_retryable);
    }

    #[tokio::test]
    async fn test_execute_task_not_found() {
        let registry = Arc::new(TaskRegistry::new());
        let executor = TaskExecutor::new(registry, TaskExecutorConfig::default());

        let result = executor
            .execute(Uuid::new_v4(), "nonexistent-task", json!({}), 1)
            .await;

        match result {
            TaskExecutionResult::Failed {
                error_message,
                error_type,
                is_retryable,
            } => {
                assert!(error_message.contains("not found"));
                assert_eq!(error_type, Some("TaskNotFound".to_string()));
                assert!(!is_retryable);
            }
            _ => panic!("Expected Failed result"),
        }
    }

    #[tokio::test]
    async fn test_execute_simple_task() {
        let registry = Arc::new(TaskRegistry::new());

        // Register a simple task
        let metadata = TaskMetadata {
            kind: "echo".to_string(),
            name: "Echo Task".to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: Some(10),
            cancellable: true,
            heartbeat_timeout_seconds: None,
        };

        registry
            .register_raw(crate::task::registry::RegisteredTask::new(
                metadata,
                Box::new(|_ctx, input| Box::pin(async move { Ok(json!({"echo": input})) })),
            ))
            .unwrap();

        let executor = TaskExecutor::new(registry, TaskExecutorConfig::default());

        let result = executor
            .execute(Uuid::new_v4(), "echo", json!({"message": "hello"}), 1)
            .await;

        match result {
            TaskExecutionResult::Completed { output } => {
                assert_eq!(output, json!({"echo": {"message": "hello"}}));
            }
            _ => panic!("Expected Completed result"),
        }
    }

    #[tokio::test]
    async fn test_execute_with_progress_callback() {
        let registry = Arc::new(TaskRegistry::new());
        let progress_values = Arc::new(std::sync::Mutex::new(vec![]));
        let progress_clone = Arc::clone(&progress_values);

        // Register a task that reports progress
        let metadata = TaskMetadata {
            kind: "progress-task".to_string(),
            name: "Progress Task".to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: Some(10),
            cancellable: true,
            heartbeat_timeout_seconds: None,
        };

        registry
            .register_raw(crate::task::registry::RegisteredTask::new(
                metadata,
                Box::new(|ctx, _input| {
                    Box::pin(async move {
                        ctx.report_progress(0.5, Some("halfway")).await?;
                        ctx.report_progress(1.0, Some("done")).await?;
                        Ok(json!({"status": "complete"}))
                    })
                }),
            ))
            .unwrap();

        let callbacks = TaskExecutorCallbacks {
            on_progress: Some(Box::new(move |progress, _msg| {
                progress_clone.lock().unwrap().push(progress);
            })),
            on_log: None,
            on_heartbeat: None,
            on_stream: None,
        };

        let executor = TaskExecutor::new(registry, TaskExecutorConfig::default());
        let result = executor
            .execute_with_callbacks(Uuid::new_v4(), "progress-task", json!({}), 1, callbacks)
            .await;

        assert!(matches!(result, TaskExecutionResult::Completed { .. }));

        let values = progress_values.lock().unwrap();
        assert_eq!(*values, vec![0.5, 1.0]);
    }

    #[tokio::test]
    async fn test_execute_task_timeout() {
        let registry = Arc::new(TaskRegistry::new());

        // Register a slow task
        let metadata = TaskMetadata {
            kind: "slow-task".to_string(),
            name: "Slow Task".to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: Some(1), // 1 second timeout
            cancellable: true,
            heartbeat_timeout_seconds: None,
        };

        registry
            .register_raw(crate::task::registry::RegisteredTask::new(
                metadata,
                Box::new(|_ctx, _input| {
                    Box::pin(async move {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        Ok(json!({}))
                    })
                }),
            ))
            .unwrap();

        let executor = TaskExecutor::new(registry, TaskExecutorConfig::default());
        let result = executor
            .execute(Uuid::new_v4(), "slow-task", json!({}), 1)
            .await;

        assert!(matches!(result, TaskExecutionResult::TimedOut));
    }

    #[tokio::test]
    async fn test_execute_task_failure() {
        let registry = Arc::new(TaskRegistry::new());

        // Register a failing task
        let metadata = TaskMetadata {
            kind: "failing-task".to_string(),
            name: "Failing Task".to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: Some(10),
            cancellable: true,
            heartbeat_timeout_seconds: None,
        };

        registry
            .register_raw(crate::task::registry::RegisteredTask::new(
                metadata,
                Box::new(|_ctx, _input| {
                    Box::pin(async move { Err(FlovynError::InvalidInput("bad data".to_string())) })
                }),
            ))
            .unwrap();

        let executor = TaskExecutor::new(registry, TaskExecutorConfig::default());
        let result = executor
            .execute(Uuid::new_v4(), "failing-task", json!({}), 1)
            .await;

        match result {
            TaskExecutionResult::Failed {
                error_type,
                is_retryable,
                ..
            } => {
                assert_eq!(error_type, Some("INVALID_INPUT".to_string()));
                assert!(!is_retryable);
            }
            _ => panic!("Expected Failed result"),
        }
    }
}
