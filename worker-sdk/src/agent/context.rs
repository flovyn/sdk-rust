//! AgentContext trait definition
//!
//! Provides the execution context for agent operations including:
//! - Conversation entry management
//! - Checkpoint-based state persistence
//! - Task scheduling
//! - Signal handling
//! - Real-time streaming

use crate::agent::future::AgentTaskFutureRaw;
use crate::error::Result;
use crate::task::streaming::StreamEvent;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

/// Entry type for agent conversation entries.
/// These correspond to the server's AgentEntryType values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    /// A conversation message (user prompt, assistant response, tool result)
    Message,
    /// Metadata-only record of an LLM API call (for observability and cost attribution)
    LlmCall,
    /// Operator-inserted context (future)
    Injection,
}

impl EntryType {
    /// Convert to string representation (lowercase to match server)
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryType::Message => "message",
            EntryType::LlmCall => "llm_call",
            EntryType::Injection => "injection",
        }
    }
}

impl std::fmt::Display for EntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Role of an entry in the conversation.
/// These correspond to the server's AgentEntryRole values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryRole {
    /// System prompt or instruction
    System,
    /// User input
    User,
    /// Assistant (model) response
    Assistant,
    /// Tool execution result
    ToolResult,
}

impl EntryRole {
    /// Convert to string representation (matches server)
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryRole::System => "system",
            EntryRole::User => "user",
            EntryRole::Assistant => "assistant",
            EntryRole::ToolResult => "tool_result",
        }
    }
}

impl std::fmt::Display for EntryRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Options for scheduling an agent task
#[derive(Debug, Clone, Default)]
pub struct ScheduleAgentTaskOptions {
    /// Task queue override
    pub queue: Option<String>,
    /// Task timeout override
    pub timeout: Option<Duration>,
    /// Maximum retry attempts
    pub max_retries: Option<u32>,
    /// Idempotency key for deduplication
    pub idempotency_key: Option<String>,
}

impl ScheduleAgentTaskOptions {
    /// Create options with an idempotency key
    pub fn with_key(key: impl Into<String>) -> Self {
        Self {
            idempotency_key: Some(key.into()),
            ..Default::default()
        }
    }

    /// Set the timeout for the task
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the maximum retries for the task
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Set the queue for the task
    pub fn queue(mut self, queue: impl Into<String>) -> Self {
        self.queue = Some(queue.into());
        self
    }
}

/// A loaded conversation message from entries
#[derive(Debug, Clone)]
pub struct LoadedMessage {
    /// Entry ID
    pub entry_id: Uuid,
    /// Entry type
    pub entry_type: EntryType,
    /// Entry role
    pub role: EntryRole,
    /// Entry content
    pub content: Value,
    /// Token usage for this entry (if applicable)
    pub token_usage: Option<TokenUsage>,
}

/// Token usage statistics
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Input tokens consumed
    pub input_tokens: i64,
    /// Output tokens generated
    pub output_tokens: i64,
    /// Model used (optional)
    pub model: Option<String>,
}

/// Signal received by the agent
#[derive(Debug, Clone)]
pub struct AgentSignalValue {
    /// Signal name
    pub name: String,
    /// Signal value
    pub value: Value,
}

/// Context for agent execution providing entry management, checkpointing, task scheduling,
/// signal handling, and streaming capabilities.
///
/// Unlike WorkflowContext which uses event-sourced replay, AgentContext uses
/// checkpoint-based recovery. On resume, the agent loads its state from the
/// latest checkpoint and reconstructs messages from the entry tree.
#[async_trait]
pub trait AgentContext: Send + Sync {
    // =========================================================================
    // Identity
    // =========================================================================

    /// Get the unique ID of this agent execution
    fn agent_execution_id(&self) -> Uuid;

    /// Get the org ID for this agent
    fn org_id(&self) -> Uuid;

    /// Get the raw agent input as JSON Value
    fn input_raw(&self) -> &Value;

    // =========================================================================
    // Conversation Entries
    // =========================================================================

    /// Append a message entry to the conversation.
    ///
    /// # Arguments
    /// * `role` - The role of this message (System, User, Assistant, Tool)
    /// * `content` - The message content as JSON
    ///
    /// # Returns
    /// The ID of the created entry
    ///
    /// # Example
    /// ```ignore
    /// ctx.append_entry(EntryRole::User, &json!({"text": "Hello!"})).await?;
    /// ctx.append_entry(EntryRole::Assistant, &json!({"text": "Hi there!"})).await?;
    /// ```
    async fn append_entry(&self, role: EntryRole, content: &Value) -> Result<Uuid>;

    /// Append a tool call entry to the conversation.
    ///
    /// # Arguments
    /// * `tool_name` - Name of the tool being called
    /// * `tool_input` - Input arguments for the tool
    ///
    /// # Returns
    /// The ID of the created entry
    async fn append_tool_call(&self, tool_name: &str, tool_input: &Value) -> Result<Uuid>;

    /// Append a tool result entry to the conversation.
    ///
    /// # Arguments
    /// * `tool_name` - Name of the tool that was called
    /// * `tool_output` - Output from the tool execution
    ///
    /// # Returns
    /// The ID of the created entry
    async fn append_tool_result(&self, tool_name: &str, tool_output: &Value) -> Result<Uuid>;

    /// Append a tool result entry with tool call ID for matching with tool calls.
    ///
    /// This variant includes the tool_call_id which allows the UI to correctly
    /// match tool results with their corresponding tool calls.
    ///
    /// # Arguments
    /// * `tool_call_id` - ID of the tool call this result corresponds to
    /// * `tool_name` - Name of the tool that was called
    /// * `tool_output` - Output from the tool execution
    ///
    /// # Returns
    /// The ID of the created entry
    async fn append_tool_result_with_id(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        tool_output: &Value,
    ) -> Result<Uuid>;

    /// Load all conversation messages for this agent.
    ///
    /// Messages are reconstructed from the entry tree, walking from root
    /// to the leaf entry referenced by the current checkpoint.
    ///
    /// This method returns cached messages loaded during agent construction.
    /// Call `reload_messages()` to fetch fresh data from the server.
    fn load_messages(&self) -> &[LoadedMessage];

    /// Reload messages from the server.
    ///
    /// Use this if you need to refresh messages after external changes.
    async fn reload_messages(&self) -> Result<Vec<LoadedMessage>>;

    // =========================================================================
    // Checkpointing
    // =========================================================================

    /// Save a checkpoint with the current state.
    ///
    /// Checkpoints are lightweight snapshots that enable recovery. On resume,
    /// the agent's state is restored from the latest checkpoint.
    ///
    /// The checkpoint automatically captures:
    /// - Current sequence number
    /// - Leaf entry ID (for message reconstruction)
    /// - Custom state provided by the caller
    ///
    /// # Arguments
    /// * `state` - Application-specific state to persist
    ///
    /// # Example
    /// ```ignore
    /// ctx.checkpoint(&json!({
    ///     "turn": 5,
    ///     "tool_calls_remaining": 3
    /// })).await?;
    /// ```
    async fn checkpoint(&self, state: &Value) -> Result<()>;

    /// Get the current checkpoint state.
    ///
    /// Returns the state from the most recent checkpoint, or `None` if
    /// no checkpoint has been created yet.
    fn state(&self) -> Option<&Value>;

    /// Get the current checkpoint sequence number.
    ///
    /// Returns the sequence number of the most recent checkpoint, or 0
    /// if no checkpoint has been created yet.
    fn checkpoint_sequence(&self) -> i32;

    // =========================================================================
    // Task Scheduling
    // =========================================================================

    /// Schedule a task and wait for its result.
    ///
    /// The task input is streamed to the server in chunks to support large inputs.
    /// Idempotency keys are auto-generated as `{agent_execution_id}:{checkpoint_seq}:{index}`
    /// to ensure tasks are deduplicated on retry.
    ///
    /// # Arguments
    /// * `task_kind` - The kind of task to schedule (must be registered with a worker)
    /// * `input` - Task input as JSON Value
    ///
    /// # Returns
    /// The task output as JSON Value
    ///
    /// # Example
    /// ```ignore
    /// let result = ctx.schedule_task_raw("analyze-code", json!({
    ///     "code": source_code,
    ///     "language": "rust"
    /// })).await?;
    /// ```
    async fn schedule_task_raw(&self, task_kind: &str, input: Value) -> Result<Value>;

    /// Schedule a task with custom options.
    async fn schedule_task_with_options_raw(
        &self,
        task_kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> Result<Value>;

    // =========================================================================
    // Lazy Task Scheduling (Preferred API)
    // =========================================================================

    /// Schedule a task and return a future (lazy, no immediate RPC).
    ///
    /// This is the preferred API for parallel task execution. The task is
    /// recorded locally with a deterministic ID but not submitted until
    /// `join_all()` or `select_ok()` is called.
    ///
    /// # Arguments
    /// * `kind` - The kind of task to schedule (must be registered with a worker)
    /// * `input` - Task input as JSON Value
    ///
    /// # Returns
    /// An `AgentTaskFutureRaw` that can be collected with `join_all()` or `select_ok()`.
    ///
    /// # Example
    /// ```ignore
    /// let f1 = ctx.schedule_task_lazy("task-a", json!({"x": 1}));
    /// let f2 = ctx.schedule_task_lazy("task-b", json!({"y": 2}));
    /// let results = ctx.join_all(vec![f1, f2]).await?;
    /// ```
    fn schedule_task_lazy(&self, kind: &str, input: Value) -> AgentTaskFutureRaw;

    /// Schedule a task with options and return a future (lazy, no immediate RPC).
    ///
    /// Similar to `schedule_task_lazy` but allows specifying task options.
    fn schedule_task_lazy_with_options(
        &self,
        kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> AgentTaskFutureRaw;

    /// Wait for all task futures to complete.
    ///
    /// Commits pending task futures as a batch, then waits for all tasks
    /// to reach a terminal state. Returns results in the same order as
    /// the input futures.
    ///
    /// # Arguments
    /// * `futures` - Task futures from `schedule_task_lazy()`
    ///
    /// # Returns
    /// A vector of task outputs in the same order as the input futures.
    ///
    /// # Errors
    /// Returns an error if any task fails or is cancelled.
    ///
    /// # Example
    /// ```ignore
    /// let f1 = ctx.schedule_task_lazy("analyze", json!({"data": "a"}));
    /// let f2 = ctx.schedule_task_lazy("analyze", json!({"data": "b"}));
    /// let results = ctx.join_all(vec![f1, f2]).await?;
    /// // results[0] is from f1, results[1] is from f2
    /// ```
    async fn join_all(&self, futures: Vec<AgentTaskFutureRaw>) -> Result<Vec<Value>>;

    /// Wait for the first task to complete successfully, return remaining futures.
    ///
    /// Commits pending task futures as a batch, then waits for the first
    /// task to complete successfully. Failed tasks are skipped; only a
    /// success triggers return.
    ///
    /// # Arguments
    /// * `futures` - Task futures from `schedule_task_lazy()`
    ///
    /// # Returns
    /// A tuple of (successful result, remaining unfinished futures).
    ///
    /// # Errors
    /// Returns an error if all tasks fail.
    ///
    /// # Example
    /// ```ignore
    /// let f1 = ctx.schedule_task_lazy("provider-a", input.clone());
    /// let f2 = ctx.schedule_task_lazy("provider-b", input.clone());
    /// let (result, remaining) = ctx.select_ok(vec![f1, f2]).await?;
    /// // result is from whichever succeeded first
    /// // remaining contains futures that haven't completed yet
    /// ```
    async fn select_ok(
        &self,
        futures: Vec<AgentTaskFutureRaw>,
    ) -> Result<(Value, Vec<AgentTaskFutureRaw>)>;

    // =========================================================================
    // Signals (User Interaction)
    // =========================================================================

    /// Wait for a signal with the given name.
    ///
    /// This suspends the agent until a signal with the specified name is received.
    /// The agent's status transitions to WAITING, and on signal receipt,
    /// transitions back to PENDING for re-execution.
    ///
    /// # Arguments
    /// * `signal_name` - Name of the signal to wait for (e.g., "userMessage")
    ///
    /// # Returns
    /// The signal value
    ///
    /// # Example
    /// ```ignore
    /// // Wait for user input
    /// let signal = ctx.wait_for_signal_raw("userMessage").await?;
    /// let user_message: String = serde_json::from_value(signal)?;
    /// ```
    async fn wait_for_signal_raw(&self, signal_name: &str) -> Result<Value>;

    /// Check if a signal with the given name is pending.
    ///
    /// This does not consume the signal.
    async fn has_signal(&self, signal_name: &str) -> Result<bool>;

    /// Drain all pending signals with the given name.
    ///
    /// This consumes all signals with the specified name currently in the queue.
    ///
    /// # Returns
    /// Vector of signal values
    async fn drain_signals_raw(&self, signal_name: &str) -> Result<Vec<Value>>;

    // =========================================================================
    // Streaming
    // =========================================================================

    /// Stream an event to connected clients.
    ///
    /// Events are ephemeral (not persisted) and delivered via SSE.
    /// This method is fire-and-forget; delivery failures are logged but not returned.
    async fn stream(&self, event: StreamEvent) -> Result<()>;

    /// Stream a token (convenience method).
    ///
    /// # Example
    /// ```ignore
    /// for token in llm.stream_completion(prompt).await? {
    ///     ctx.stream_token(&token).await?;
    /// }
    /// ```
    async fn stream_token(&self, text: &str) -> Result<()> {
        self.stream(StreamEvent::token(text)).await
    }

    /// Stream progress (convenience method).
    async fn stream_progress(&self, progress: f64, details: Option<&str>) -> Result<()> {
        self.stream(StreamEvent::progress(progress, details)).await
    }

    /// Stream data (convenience method).
    async fn stream_data_value(&self, data: Value) -> Result<()> {
        self.stream(StreamEvent::data_value(data)).await
    }

    /// Stream an error notification (convenience method).
    async fn stream_error(&self, message: &str, code: Option<&str>) -> Result<()> {
        self.stream(StreamEvent::error(message, code)).await
    }

    // =========================================================================
    // Cancellation
    // =========================================================================

    /// Check if cancellation has been requested
    fn is_cancellation_requested(&self) -> bool;

    /// Check for cancellation and return error if cancelled
    async fn check_cancellation(&self) -> Result<()>;
}

/// Extension trait for typed agent context operations.
pub trait AgentContextExt: AgentContext {
    /// Get the agent input as the specified type
    fn input<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.input_raw().clone())
            .map_err(crate::error::FlovynError::Serialization)
    }

    /// Schedule a typed task and wait for its result.
    fn schedule_task<I: Serialize + Send, O: DeserializeOwned>(
        &self,
        task_kind: &str,
        input: I,
    ) -> impl std::future::Future<Output = Result<O>> + Send
    where
        Self: Sync,
    {
        async move {
            let input_value =
                serde_json::to_value(input).map_err(crate::error::FlovynError::Serialization)?;
            let output_value = self.schedule_task_raw(task_kind, input_value).await?;
            serde_json::from_value(output_value).map_err(crate::error::FlovynError::Serialization)
        }
    }

    /// Schedule a typed task with options.
    fn schedule_task_with_options<I: Serialize + Send, O: DeserializeOwned>(
        &self,
        task_kind: &str,
        input: I,
        options: ScheduleAgentTaskOptions,
    ) -> impl std::future::Future<Output = Result<O>> + Send
    where
        Self: Sync,
    {
        async move {
            let input_value =
                serde_json::to_value(input).map_err(crate::error::FlovynError::Serialization)?;
            let output_value = self
                .schedule_task_with_options_raw(task_kind, input_value, options)
                .await?;
            serde_json::from_value(output_value).map_err(crate::error::FlovynError::Serialization)
        }
    }

    /// Wait for a typed signal.
    fn wait_for_signal<T: DeserializeOwned>(
        &self,
        signal_name: &str,
    ) -> impl std::future::Future<Output = Result<T>> + Send
    where
        Self: Sync,
    {
        async move {
            let value = self.wait_for_signal_raw(signal_name).await?;
            serde_json::from_value(value).map_err(crate::error::FlovynError::Serialization)
        }
    }

    /// Drain all pending signals of a specific type.
    fn drain_signals<T: DeserializeOwned>(
        &self,
        signal_name: &str,
    ) -> impl std::future::Future<Output = Result<Vec<T>>> + Send
    where
        Self: Sync,
    {
        async move {
            let values = self.drain_signals_raw(signal_name).await?;
            values
                .into_iter()
                .map(|v| {
                    serde_json::from_value(v).map_err(crate::error::FlovynError::Serialization)
                })
                .collect()
        }
    }
}

// Implement AgentContextExt for all types that implement AgentContext
impl<C: AgentContext + ?Sized> AgentContextExt for C {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_type_as_str() {
        assert_eq!(EntryType::Message.as_str(), "message");
        assert_eq!(EntryType::LlmCall.as_str(), "llm_call");
        assert_eq!(EntryType::Injection.as_str(), "injection");
    }

    #[test]
    fn test_entry_role_as_str() {
        assert_eq!(EntryRole::System.as_str(), "system");
        assert_eq!(EntryRole::User.as_str(), "user");
        assert_eq!(EntryRole::Assistant.as_str(), "assistant");
        assert_eq!(EntryRole::ToolResult.as_str(), "tool_result");
    }

    #[test]
    fn test_schedule_task_options_default() {
        let options = ScheduleAgentTaskOptions::default();
        assert!(options.queue.is_none());
        assert!(options.timeout.is_none());
        assert!(options.max_retries.is_none());
        assert!(options.idempotency_key.is_none());
    }

    #[test]
    fn test_schedule_task_options_builder() {
        let options = ScheduleAgentTaskOptions::with_key("my-key")
            .timeout(Duration::from_secs(30))
            .max_retries(3)
            .queue("high-priority");

        assert_eq!(options.idempotency_key, Some("my-key".to_string()));
        assert_eq!(options.timeout, Some(Duration::from_secs(30)));
        assert_eq!(options.max_retries, Some(3));
        assert_eq!(options.queue, Some("high-priority".to_string()));
    }

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert!(usage.model.is_none());
    }
}
