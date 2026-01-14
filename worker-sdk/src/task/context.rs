//! TaskContext trait definition

use crate::error::Result;
use crate::task::streaming::StreamEvent;
use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

/// Log level for task logging
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Context for task execution providing progress reporting, logging, and streaming.
#[async_trait]
pub trait TaskContext: Send + Sync {
    /// Get the unique ID of this task execution
    fn task_execution_id(&self) -> Uuid;

    /// Get the current attempt number (1-indexed)
    fn attempt(&self) -> u32;

    /// Report progress (0.0 to 1.0) with optional message.
    ///
    /// This is persisted progress that the server tracks.
    async fn report_progress(&self, progress: f64, message: Option<&str>) -> Result<()>;

    /// Log a message at the specified level
    async fn log(&self, level: LogLevel, message: &str) -> Result<()>;

    /// Check if the task has been cancelled
    fn is_cancelled(&self) -> bool;

    /// Check for cancellation and return error if cancelled
    async fn check_cancellation(&self) -> Result<()>;

    // === Streaming Methods ===

    /// Stream an event to connected clients.
    ///
    /// Events are ephemeral (not persisted) and delivered via SSE.
    /// This method is fire-and-forget; delivery failures are logged but not returned.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Stream LLM tokens
    /// for token in llm.stream_completion(prompt).await? {
    ///     ctx.stream(StreamEvent::token(&token)).await?;
    /// }
    ///
    /// // Stream progress
    /// ctx.stream(StreamEvent::progress(0.5, Some("Halfway done"))).await?;
    /// ```
    async fn stream(&self, event: StreamEvent) -> Result<()>;

    /// Stream a token (convenience method).
    ///
    /// Equivalent to `stream(StreamEvent::Token { text })`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// ctx.stream_token("Hello, ").await?;
    /// ctx.stream_token("world!").await?;
    /// ```
    async fn stream_token(&self, text: &str) -> Result<()> {
        self.stream(StreamEvent::token(text)).await
    }

    /// Stream progress (convenience method).
    ///
    /// Equivalent to `stream(StreamEvent::Progress { progress, details })`.
    /// Progress values are clamped to 0.0-1.0 during serialization.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// ctx.stream_progress(0.5, Some("Processing...")).await?;
    /// ctx.stream_progress(1.0, None).await?;
    /// ```
    async fn stream_progress(&self, progress: f64, details: Option<&str>) -> Result<()> {
        self.stream(StreamEvent::progress(progress, details)).await
    }

    /// Stream data (convenience method).
    ///
    /// Equivalent to `stream(StreamEvent::Data { data })`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// ctx.stream_data_value(serde_json::json!({"count": 42})).await?;
    /// ```
    async fn stream_data_value(&self, data: Value) -> Result<()> {
        self.stream(StreamEvent::data_value(data)).await
    }

    /// Stream an error notification (convenience method).
    ///
    /// Use to notify clients of recoverable errors during execution.
    /// For fatal errors, let the task fail normally.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// ctx.stream_error("Rate limit exceeded, retrying...", Some("RATE_LIMIT")).await?;
    /// ```
    async fn stream_error(&self, message: &str, code: Option<&str>) -> Result<()> {
        self.stream(StreamEvent::error(message, code)).await
    }
}
