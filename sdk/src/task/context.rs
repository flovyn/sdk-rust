//! TaskContext trait definition

use crate::error::Result;
use async_trait::async_trait;
use uuid::Uuid;

/// Log level for task logging
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Context for task execution providing progress reporting and logging
#[async_trait]
pub trait TaskContext: Send + Sync {
    /// Get the unique ID of this task execution
    fn task_execution_id(&self) -> Uuid;

    /// Get the current attempt number (1-indexed)
    fn attempt(&self) -> u32;

    /// Report progress (0.0 to 1.0) with optional message
    async fn report_progress(&self, progress: f64, message: Option<&str>) -> Result<()>;

    /// Log a message at the specified level
    async fn log(&self, level: LogLevel, message: &str) -> Result<()>;

    /// Check if the task has been cancelled
    fn is_cancelled(&self) -> bool;

    /// Check for cancellation and return error if cancelled
    async fn check_cancellation(&self) -> Result<()>;
}
