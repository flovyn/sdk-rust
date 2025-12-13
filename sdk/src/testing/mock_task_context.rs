//! Mock task context for unit testing tasks in isolation.

use crate::error::{FlovynError, Result};
use crate::task::context::{LogLevel, TaskContext};
use async_trait::async_trait;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Mock implementation of TaskContext for testing tasks in isolation.
///
/// This mock allows you to:
/// - Track progress reports
/// - Capture log messages
/// - Simulate cancellation
///
/// # Example
///
/// ```ignore
/// use flovyn_sdk::testing::MockTaskContext;
///
/// let ctx = MockTaskContext::builder()
///     .task_execution_id(Uuid::new_v4())
///     .attempt(1)
///     .build();
///
/// // Execute your task with the mock context
/// my_task(&ctx).await?;
///
/// // Verify progress was reported
/// assert!(ctx.last_progress() > 0.0);
/// ```
pub struct MockTaskContext {
    inner: Arc<MockTaskContextInner>,
}

struct MockTaskContextInner {
    task_execution_id: Uuid,
    attempt: AtomicU32,
    cancelled: AtomicBool,
    progress_reports: RwLock<Vec<ProgressReport>>,
    log_messages: RwLock<Vec<LogMessage>>,
}

impl Clone for MockTaskContext {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// A recorded progress report
#[derive(Debug, Clone)]
pub struct ProgressReport {
    pub progress: f64,
    pub message: Option<String>,
}

/// A recorded log message
#[derive(Debug, Clone)]
pub struct LogMessage {
    pub level: LogLevel,
    pub message: String,
}

impl MockTaskContext {
    /// Create a new builder for MockTaskContext.
    pub fn builder() -> MockTaskContextBuilder {
        MockTaskContextBuilder::default()
    }

    /// Create a simple mock context with default values.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Get all progress reports.
    pub fn progress_reports(&self) -> Vec<ProgressReport> {
        self.inner.progress_reports.read().clone()
    }

    /// Get the last reported progress value.
    pub fn last_progress(&self) -> Option<f64> {
        self.inner
            .progress_reports
            .read()
            .last()
            .map(|r| r.progress)
    }

    /// Get all log messages.
    pub fn log_messages(&self) -> Vec<LogMessage> {
        self.inner.log_messages.read().clone()
    }

    /// Get log messages at a specific level.
    pub fn log_messages_at_level(&self, level: LogLevel) -> Vec<LogMessage> {
        self.inner
            .log_messages
            .read()
            .iter()
            .filter(|m| m.level == level)
            .cloned()
            .collect()
    }

    /// Check if a message was logged at any level.
    pub fn was_logged(&self, message: &str) -> bool {
        self.inner
            .log_messages
            .read()
            .iter()
            .any(|m| m.message.contains(message))
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
    }

    /// Set the attempt number.
    pub fn set_attempt(&self, attempt: u32) {
        self.inner.attempt.store(attempt, Ordering::SeqCst);
    }

    /// Clear all recorded progress and logs.
    pub fn clear_recordings(&self) {
        self.inner.progress_reports.write().clear();
        self.inner.log_messages.write().clear();
    }
}

impl Default for MockTaskContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for MockTaskContext.
#[derive(Default)]
pub struct MockTaskContextBuilder {
    task_execution_id: Option<Uuid>,
    attempt: Option<u32>,
    cancelled: bool,
}

impl MockTaskContextBuilder {
    /// Set the task execution ID.
    pub fn task_execution_id(mut self, id: Uuid) -> Self {
        self.task_execution_id = Some(id);
        self
    }

    /// Set the attempt number.
    pub fn attempt(mut self, attempt: u32) -> Self {
        self.attempt = Some(attempt);
        self
    }

    /// Set the task as cancelled.
    pub fn cancelled(mut self) -> Self {
        self.cancelled = true;
        self
    }

    /// Build the MockTaskContext.
    pub fn build(self) -> MockTaskContext {
        MockTaskContext {
            inner: Arc::new(MockTaskContextInner {
                task_execution_id: self.task_execution_id.unwrap_or_else(Uuid::new_v4),
                attempt: AtomicU32::new(self.attempt.unwrap_or(1)),
                cancelled: AtomicBool::new(self.cancelled),
                progress_reports: RwLock::new(Vec::new()),
                log_messages: RwLock::new(Vec::new()),
            }),
        }
    }
}

#[async_trait]
impl TaskContext for MockTaskContext {
    fn task_execution_id(&self) -> Uuid {
        self.inner.task_execution_id
    }

    fn attempt(&self) -> u32 {
        self.inner.attempt.load(Ordering::SeqCst)
    }

    async fn report_progress(&self, progress: f64, message: Option<&str>) -> Result<()> {
        if !(0.0..=1.0).contains(&progress) {
            return Err(FlovynError::InvalidInput(format!(
                "Progress must be between 0.0 and 1.0, got {}",
                progress
            )));
        }

        self.inner.progress_reports.write().push(ProgressReport {
            progress,
            message: message.map(|s| s.to_string()),
        });
        Ok(())
    }

    async fn log(&self, level: LogLevel, message: &str) -> Result<()> {
        self.inner.log_messages.write().push(LogMessage {
            level,
            message: message.to_string(),
        });
        Ok(())
    }

    fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
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

    #[test]
    fn test_mock_task_context_new() {
        let ctx = MockTaskContext::new();
        assert!(!ctx.task_execution_id().is_nil());
        assert_eq!(ctx.attempt(), 1);
    }

    #[test]
    fn test_mock_task_context_builder() {
        let id = Uuid::new_v4();
        let ctx = MockTaskContext::builder()
            .task_execution_id(id)
            .attempt(3)
            .build();

        assert_eq!(ctx.task_execution_id(), id);
        assert_eq!(ctx.attempt(), 3);
    }

    #[test]
    fn test_mock_task_context_builder_cancelled() {
        let ctx = MockTaskContext::builder().cancelled().build();
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_mock_task_context_progress() {
        let ctx = MockTaskContext::new();

        ctx.report_progress(0.0, None).await.unwrap();
        ctx.report_progress(0.5, Some("Halfway")).await.unwrap();
        ctx.report_progress(1.0, Some("Done")).await.unwrap();

        let reports = ctx.progress_reports();
        assert_eq!(reports.len(), 3);
        assert_eq!(reports[0].progress, 0.0);
        assert_eq!(reports[1].progress, 0.5);
        assert_eq!(reports[1].message, Some("Halfway".to_string()));
        assert_eq!(reports[2].progress, 1.0);

        assert_eq!(ctx.last_progress(), Some(1.0));
    }

    #[tokio::test]
    async fn test_mock_task_context_progress_invalid() {
        let ctx = MockTaskContext::new();

        assert!(ctx.report_progress(-0.1, None).await.is_err());
        assert!(ctx.report_progress(1.1, None).await.is_err());
    }

    #[tokio::test]
    async fn test_mock_task_context_logging() {
        let ctx = MockTaskContext::new();

        ctx.log(LogLevel::Info, "Starting task").await.unwrap();
        ctx.log(LogLevel::Debug, "Debug info").await.unwrap();
        ctx.log(LogLevel::Warn, "Warning message").await.unwrap();
        ctx.log(LogLevel::Error, "Error occurred").await.unwrap();

        let logs = ctx.log_messages();
        assert_eq!(logs.len(), 4);

        let info_logs = ctx.log_messages_at_level(LogLevel::Info);
        assert_eq!(info_logs.len(), 1);
        assert_eq!(info_logs[0].message, "Starting task");

        assert!(ctx.was_logged("Starting"));
        assert!(ctx.was_logged("Warning"));
        assert!(!ctx.was_logged("Not logged"));
    }

    #[test]
    fn test_mock_task_context_cancellation() {
        let ctx = MockTaskContext::new();
        assert!(!ctx.is_cancelled());

        ctx.cancel();
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_mock_task_context_check_cancellation() {
        let ctx = MockTaskContext::new();
        assert!(ctx.check_cancellation().await.is_ok());

        ctx.cancel();
        assert!(ctx.check_cancellation().await.is_err());
    }

    #[test]
    fn test_mock_task_context_set_attempt() {
        let ctx = MockTaskContext::new();
        assert_eq!(ctx.attempt(), 1);

        ctx.set_attempt(5);
        assert_eq!(ctx.attempt(), 5);
    }

    #[tokio::test]
    async fn test_mock_task_context_clear_recordings() {
        let ctx = MockTaskContext::new();

        ctx.report_progress(0.5, None).await.unwrap();
        ctx.log(LogLevel::Info, "Test").await.unwrap();

        assert!(!ctx.progress_reports().is_empty());
        assert!(!ctx.log_messages().is_empty());

        ctx.clear_recordings();

        assert!(ctx.progress_reports().is_empty());
        assert!(ctx.log_messages().is_empty());
    }

    #[test]
    fn test_mock_task_context_clone() {
        let ctx1 = MockTaskContext::new();
        let ctx2 = ctx1.clone();

        ctx1.set_attempt(5);
        assert_eq!(ctx2.attempt(), 5); // Same underlying state
    }

    #[test]
    fn test_mock_task_context_default() {
        let ctx = MockTaskContext::default();
        assert_eq!(ctx.attempt(), 1);
    }
}
