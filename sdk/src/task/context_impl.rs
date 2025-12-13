//! TaskContextImpl - Concrete implementation of TaskContext

use crate::error::{FlovynError, Result};
use crate::task::context::{LogLevel, TaskContext};
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Type alias for progress reporter callback
pub type ProgressReporter = Arc<
    dyn Fn(f64, Option<String>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync,
>;

/// Type alias for log reporter callback
pub type LogReporter =
    Arc<dyn Fn(LogLevel, String) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;

/// Type alias for heartbeat reporter callback
pub type HeartbeatReporter =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync>;

/// Concrete implementation of TaskContext
pub struct TaskContextImpl {
    /// Unique ID for this task execution
    task_execution_id: Uuid,
    /// Current attempt number (1-indexed)
    attempt: u32,
    /// Callback for progress reporting
    progress_reporter: Option<ProgressReporter>,
    /// Callback for log reporting
    log_reporter: Option<LogReporter>,
    /// Callback for heartbeat reporting
    heartbeat_reporter: Option<HeartbeatReporter>,
    /// Cancellation flag
    cancelled: Arc<AtomicBool>,
}

impl TaskContextImpl {
    /// Create a new TaskContextImpl
    pub fn new(task_execution_id: Uuid, attempt: u32) -> Self {
        Self {
            task_execution_id,
            attempt,
            progress_reporter: None,
            log_reporter: None,
            heartbeat_reporter: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a new TaskContextImpl with all callbacks
    pub fn with_callbacks(
        task_execution_id: Uuid,
        attempt: u32,
        progress_reporter: ProgressReporter,
        log_reporter: LogReporter,
        heartbeat_reporter: HeartbeatReporter,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            task_execution_id,
            attempt,
            progress_reporter: Some(progress_reporter),
            log_reporter: Some(log_reporter),
            heartbeat_reporter: Some(heartbeat_reporter),
            cancelled,
        }
    }

    /// Set the progress reporter callback
    pub fn set_progress_reporter(&mut self, reporter: ProgressReporter) {
        self.progress_reporter = Some(reporter);
    }

    /// Set the log reporter callback
    pub fn set_log_reporter(&mut self, reporter: LogReporter) {
        self.log_reporter = Some(reporter);
    }

    /// Set the heartbeat reporter callback
    pub fn set_heartbeat_reporter(&mut self, reporter: HeartbeatReporter) {
        self.heartbeat_reporter = Some(reporter);
    }

    /// Set the cancellation flag
    pub fn set_cancelled(&mut self, cancelled: Arc<AtomicBool>) {
        self.cancelled = cancelled;
    }

    /// Mark the task as cancelled
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Get the cancellation flag for external monitoring
    pub fn cancelled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    /// Send a heartbeat
    pub async fn heartbeat(&self) -> Result<()> {
        if let Some(ref reporter) = self.heartbeat_reporter {
            reporter().await
        } else {
            Ok(())
        }
    }
}

impl std::fmt::Debug for TaskContextImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskContextImpl")
            .field("task_execution_id", &self.task_execution_id)
            .field("attempt", &self.attempt)
            .field("cancelled", &self.cancelled.load(Ordering::SeqCst))
            .finish()
    }
}

#[async_trait]
impl TaskContext for TaskContextImpl {
    fn task_execution_id(&self) -> Uuid {
        self.task_execution_id
    }

    fn attempt(&self) -> u32 {
        self.attempt
    }

    async fn report_progress(&self, progress: f64, message: Option<&str>) -> Result<()> {
        // Validate progress range
        if !(0.0..=1.0).contains(&progress) {
            return Err(FlovynError::InvalidInput(
                "Progress must be between 0.0 and 1.0".to_string(),
            ));
        }

        if let Some(ref reporter) = self.progress_reporter {
            match reporter(progress, message.map(|s| s.to_string())).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Log warning but don't fail the task
                    tracing::warn!("Failed to report progress: {}", e);
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }

    async fn log(&self, level: LogLevel, message: &str) -> Result<()> {
        if let Some(ref reporter) = self.log_reporter {
            match reporter(level, message.to_string()).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Log warning but don't fail the task
                    tracing::warn!("Failed to send log: {}", e);
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    async fn check_cancellation(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(FlovynError::TaskCancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::sync::Mutex;

    #[test]
    fn test_task_context_impl_new() {
        let id = Uuid::new_v4();
        let ctx = TaskContextImpl::new(id, 1);
        assert_eq!(ctx.task_execution_id(), id);
        assert_eq!(ctx.attempt(), 1);
        assert!(!ctx.is_cancelled());
    }

    #[test]
    fn test_task_context_impl_debug() {
        let id = Uuid::new_v4();
        let ctx = TaskContextImpl::new(id, 2);
        let debug_str = format!("{:?}", ctx);
        assert!(debug_str.contains("TaskContextImpl"));
        assert!(debug_str.contains("attempt: 2"));
    }

    #[test]
    fn test_task_context_cancel() {
        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        assert!(!ctx.is_cancelled());
        ctx.cancel();
        assert!(ctx.is_cancelled());
    }

    #[test]
    fn test_task_context_cancelled_flag() {
        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        let flag = ctx.cancelled_flag();
        assert!(!flag.load(Ordering::SeqCst));
        ctx.cancel();
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_report_progress_valid() {
        let progress_values: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(vec![]));
        let progress_clone = Arc::clone(&progress_values);

        let reporter: ProgressReporter = Arc::new(move |progress, _msg| {
            let values = Arc::clone(&progress_clone);
            Box::pin(async move {
                values.lock().unwrap().push(progress);
                Ok(())
            })
        });

        let mut ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        ctx.set_progress_reporter(reporter);

        ctx.report_progress(0.0, None).await.unwrap();
        ctx.report_progress(0.5, Some("halfway")).await.unwrap();
        ctx.report_progress(1.0, Some("done")).await.unwrap();

        let values = progress_values.lock().unwrap();
        assert_eq!(*values, vec![0.0, 0.5, 1.0]);
    }

    #[tokio::test]
    async fn test_report_progress_invalid_too_low() {
        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        let result = ctx.report_progress(-0.1, None).await;
        assert!(result.is_err());
        match result {
            Err(FlovynError::InvalidInput(msg)) => {
                assert!(msg.contains("between 0.0 and 1.0"));
            }
            _ => panic!("Expected InvalidInput error"),
        }
    }

    #[tokio::test]
    async fn test_report_progress_invalid_too_high() {
        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        let result = ctx.report_progress(1.1, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_log() {
        let logs: Arc<Mutex<Vec<(LogLevel, String)>>> = Arc::new(Mutex::new(vec![]));
        let logs_clone = Arc::clone(&logs);

        let reporter: LogReporter = Arc::new(move |level, msg| {
            let logs = Arc::clone(&logs_clone);
            Box::pin(async move {
                logs.lock().unwrap().push((level, msg));
                Ok(())
            })
        });

        let mut ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        ctx.set_log_reporter(reporter);

        ctx.log(LogLevel::Info, "test message").await.unwrap();
        ctx.log(LogLevel::Error, "error message").await.unwrap();

        let logs = logs.lock().unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0], (LogLevel::Info, "test message".to_string()));
        assert_eq!(logs[1], (LogLevel::Error, "error message".to_string()));
    }

    #[tokio::test]
    async fn test_check_cancellation_not_cancelled() {
        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        let result = ctx.check_cancellation().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_check_cancellation_cancelled() {
        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        ctx.cancel();
        let result = ctx.check_cancellation().await;
        assert!(result.is_err());
        assert!(matches!(result, Err(FlovynError::TaskCancelled)));
    }

    #[tokio::test]
    async fn test_heartbeat() {
        let heartbeat_count = Arc::new(AtomicU32::new(0));
        let count_clone = Arc::clone(&heartbeat_count);

        let reporter: HeartbeatReporter = Arc::new(move || {
            let count = Arc::clone(&count_clone);
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });

        let mut ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        ctx.set_heartbeat_reporter(reporter);

        ctx.heartbeat().await.unwrap();
        ctx.heartbeat().await.unwrap();

        assert_eq!(heartbeat_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_with_callbacks() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let progress_reporter: ProgressReporter = Arc::new(|_, _| Box::pin(async { Ok(()) }));
        let log_reporter: LogReporter = Arc::new(|_, _| Box::pin(async { Ok(()) }));
        let heartbeat_reporter: HeartbeatReporter = Arc::new(|| Box::pin(async { Ok(()) }));

        let id = Uuid::new_v4();
        let ctx = TaskContextImpl::with_callbacks(
            id,
            3,
            progress_reporter,
            log_reporter,
            heartbeat_reporter,
            cancelled,
        );

        assert_eq!(ctx.task_execution_id(), id);
        assert_eq!(ctx.attempt(), 3);
        assert!(!ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_progress_reporter_error_is_swallowed() {
        let reporter: ProgressReporter = Arc::new(|_, _| {
            Box::pin(async { Err(FlovynError::NetworkError("connection failed".to_string())) })
        });

        let mut ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        ctx.set_progress_reporter(reporter);

        // Error should be swallowed and Ok returned
        let result = ctx.report_progress(0.5, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_log_reporter_error_is_swallowed() {
        let reporter: LogReporter = Arc::new(|_, _| {
            Box::pin(async { Err(FlovynError::NetworkError("connection failed".to_string())) })
        });

        let mut ctx = TaskContextImpl::new(Uuid::new_v4(), 1);
        ctx.set_log_reporter(reporter);

        // Error should be swallowed and Ok returned
        let result = ctx.log(LogLevel::Info, "test").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_no_reporter_returns_ok() {
        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);

        // Without reporters, operations should return Ok
        assert!(ctx.report_progress(0.5, None).await.is_ok());
        assert!(ctx.log(LogLevel::Info, "test").await.is_ok());
        assert!(ctx.heartbeat().await.is_ok());
    }
}
