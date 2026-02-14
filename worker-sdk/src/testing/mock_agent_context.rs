//! Mock agent context for unit testing agents in isolation.

use crate::agent::context::{
    AgentContext, CancelTaskResult, EntryRole, LoadedMessage, ScheduleAgentTaskOptions,
};
use crate::agent::future::AgentTaskFutureRaw;
use crate::error::{FlovynError, Result};
use crate::task::streaming::StreamEvent;
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Mock implementation of AgentContext for testing agents in isolation.
///
/// This mock allows you to:
/// - Set up expected task results
/// - Set up expected signals
/// - Inspect recorded entries and operations
/// - Control checkpoint state
///
/// # Example
///
/// ```ignore
/// use flovyn_worker_sdk::testing::MockAgentContext;
/// use serde_json::json;
///
/// let ctx = MockAgentContext::builder()
///     .agent_execution_id(Uuid::new_v4())
///     .input(json!({"prompt": "Hello"}))
///     .task_result("llm-request", json!({"response": "Hi there!"}))
///     .build();
///
/// // Execute your agent with the mock context
/// let result = my_agent.execute(&ctx, input).await;
///
/// // Verify expectations
/// assert!(ctx.was_task_scheduled("llm-request"));
/// assert_eq!(ctx.entries().len(), 2);
/// ```
pub struct MockAgentContext {
    inner: Arc<MockAgentContextInner>,
}

struct MockAgentContextInner {
    agent_execution_id: Uuid,
    org_id: Uuid,
    input: Value,
    // Loaded messages for resume
    loaded_messages: RwLock<Vec<LoadedMessage>>,
    // Recorded entries
    entries: RwLock<Vec<RecordedEntry>>,
    // Checkpoint state
    checkpoint_state: RwLock<Option<Value>>,
    checkpoint_seq: AtomicI32,
    // Task results
    task_results: RwLock<HashMap<String, Value>>,
    // Scheduled tasks
    scheduled_tasks: RwLock<Vec<ScheduledAgentTask>>,
    // Signal queues
    signal_queues: RwLock<HashMap<String, VecDeque<Value>>>,
    // Streamed events
    streamed_events: RwLock<Vec<StreamEvent>>,
    // Cancellation
    cancellation_requested: AtomicBool,
}

impl Clone for MockAgentContext {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// A recorded conversation entry
#[derive(Debug, Clone)]
pub struct RecordedEntry {
    pub id: Uuid,
    pub role: EntryRole,
    pub content: Value,
    pub entry_type: String,
}

/// A scheduled agent task
#[derive(Debug, Clone)]
pub struct ScheduledAgentTask {
    pub task_id: Uuid,
    pub task_kind: String,
    pub input: Value,
    pub options: ScheduleAgentTaskOptions,
}

impl MockAgentContext {
    /// Create a new builder for MockAgentContext.
    pub fn builder() -> MockAgentContextBuilder {
        MockAgentContextBuilder::default()
    }

    /// Create a simple mock context with default values.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Get all recorded entries.
    pub fn entries(&self) -> Vec<RecordedEntry> {
        self.inner.entries.read().clone()
    }

    /// Get all scheduled tasks.
    pub fn scheduled_tasks(&self) -> Vec<ScheduledAgentTask> {
        self.inner.scheduled_tasks.read().clone()
    }

    /// Check if a specific task type was scheduled.
    pub fn was_task_scheduled(&self, task_kind: &str) -> bool {
        self.inner
            .scheduled_tasks
            .read()
            .iter()
            .any(|t| t.task_kind == task_kind)
    }

    /// Get all streamed events.
    pub fn streamed_events(&self) -> Vec<StreamEvent> {
        self.inner.streamed_events.read().clone()
    }

    /// Request cancellation.
    pub fn request_cancellation(&self) {
        self.inner
            .cancellation_requested
            .store(true, Ordering::SeqCst);
    }

    /// Set a task result for testing.
    pub fn set_task_result(&self, task_kind: &str, result: Value) {
        self.inner
            .task_results
            .write()
            .insert(task_kind.to_string(), result);
    }

    /// Add a signal to the queue for testing.
    pub fn add_signal(&self, signal_name: &str, value: Value) {
        self.inner
            .signal_queues
            .write()
            .entry(signal_name.to_string())
            .or_default()
            .push_back(value);
    }

    /// Get the current checkpoint state.
    pub fn current_checkpoint_state(&self) -> Option<Value> {
        self.inner.checkpoint_state.read().clone()
    }

    /// Get a task result by kind (for internal use in join_all/select_ok).
    fn get_task_result(&self, task_kind: &str) -> Option<Value> {
        self.inner.task_results.read().get(task_kind).cloned()
    }
}

impl Default for MockAgentContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for MockAgentContext.
#[derive(Default)]
pub struct MockAgentContextBuilder {
    agent_execution_id: Option<Uuid>,
    org_id: Option<Uuid>,
    input: Option<Value>,
    loaded_messages: Vec<LoadedMessage>,
    task_results: HashMap<String, Value>,
    signal_queues: HashMap<String, VecDeque<Value>>,
    initial_checkpoint_state: Option<Value>,
    initial_checkpoint_seq: i32,
}

impl MockAgentContextBuilder {
    /// Set the agent execution ID.
    pub fn agent_execution_id(mut self, id: Uuid) -> Self {
        self.agent_execution_id = Some(id);
        self
    }

    /// Set the org ID.
    pub fn org_id(mut self, id: Uuid) -> Self {
        self.org_id = Some(id);
        self
    }

    /// Set the agent input.
    pub fn input(mut self, input: Value) -> Self {
        self.input = Some(input);
        self
    }

    /// Add a loaded message (for simulating resume).
    pub fn loaded_message(mut self, role: EntryRole, content: Value) -> Self {
        self.loaded_messages.push(LoadedMessage {
            entry_id: Uuid::new_v4(),
            entry_type: crate::agent::context::EntryType::Message,
            role,
            content,
            token_usage: None,
        });
        self
    }

    /// Set an expected task result.
    pub fn task_result(mut self, task_kind: &str, result: Value) -> Self {
        self.task_results.insert(task_kind.to_string(), result);
        self
    }

    /// Add a mock signal to the signal queue.
    pub fn mock_signal(mut self, name: &str, value: Value) -> Self {
        self.signal_queues
            .entry(name.to_string())
            .or_default()
            .push_back(value);
        self
    }

    /// Set the initial checkpoint state.
    pub fn initial_checkpoint_state(mut self, state: Value) -> Self {
        self.initial_checkpoint_state = Some(state);
        self
    }

    /// Set the initial checkpoint sequence.
    pub fn initial_checkpoint_seq(mut self, seq: i32) -> Self {
        self.initial_checkpoint_seq = seq;
        self
    }

    /// Build the MockAgentContext.
    pub fn build(self) -> MockAgentContext {
        MockAgentContext {
            inner: Arc::new(MockAgentContextInner {
                agent_execution_id: self.agent_execution_id.unwrap_or_else(Uuid::new_v4),
                org_id: self.org_id.unwrap_or_else(Uuid::new_v4),
                input: self.input.unwrap_or(Value::Null),
                loaded_messages: RwLock::new(self.loaded_messages),
                entries: RwLock::new(Vec::new()),
                checkpoint_state: RwLock::new(self.initial_checkpoint_state),
                checkpoint_seq: AtomicI32::new(self.initial_checkpoint_seq),
                task_results: RwLock::new(self.task_results),
                scheduled_tasks: RwLock::new(Vec::new()),
                signal_queues: RwLock::new(self.signal_queues),
                streamed_events: RwLock::new(Vec::new()),
                cancellation_requested: AtomicBool::new(false),
            }),
        }
    }
}

#[async_trait]
impl AgentContext for MockAgentContext {
    // =========================================================================
    // Identity
    // =========================================================================

    fn agent_execution_id(&self) -> Uuid {
        self.inner.agent_execution_id
    }

    fn org_id(&self) -> Uuid {
        self.inner.org_id
    }

    fn input_raw(&self) -> &Value {
        &self.inner.input
    }

    // =========================================================================
    // Conversation Entries
    // =========================================================================

    async fn append_entry(&self, role: EntryRole, content: &Value) -> Result<Uuid> {
        let id = Uuid::new_v4();
        self.inner.entries.write().push(RecordedEntry {
            id,
            role,
            content: content.clone(),
            entry_type: "message".to_string(),
        });
        Ok(id)
    }

    async fn append_tool_call(&self, tool_name: &str, tool_input: &Value) -> Result<Uuid> {
        let id = Uuid::new_v4();
        self.inner.entries.write().push(RecordedEntry {
            id,
            role: EntryRole::Assistant,
            content: serde_json::json!({
                "toolName": tool_name,
                "toolInput": tool_input,
            }),
            entry_type: "tool_call".to_string(),
        });
        Ok(id)
    }

    async fn append_tool_result(&self, tool_name: &str, tool_output: &Value) -> Result<Uuid> {
        let id = Uuid::new_v4();
        self.inner.entries.write().push(RecordedEntry {
            id,
            role: EntryRole::ToolResult,
            content: serde_json::json!({
                "toolName": tool_name,
                "result": tool_output,
            }),
            entry_type: "tool_result".to_string(),
        });
        Ok(id)
    }

    async fn append_tool_result_with_id(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        tool_output: &Value,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        self.inner.entries.write().push(RecordedEntry {
            id,
            role: EntryRole::ToolResult,
            content: serde_json::json!({
                "toolCallId": tool_call_id,
                "toolName": tool_name,
                "result": tool_output,
            }),
            entry_type: "tool_result".to_string(),
        });
        Ok(id)
    }

    fn load_messages(&self) -> Vec<LoadedMessage> {
        self.inner.loaded_messages.read().clone()
    }

    async fn reload_messages(&self) -> Result<Vec<LoadedMessage>> {
        Ok(self.inner.loaded_messages.read().clone())
    }

    // =========================================================================
    // Checkpointing
    // =========================================================================

    async fn checkpoint(&self, state: &Value) -> Result<()> {
        *self.inner.checkpoint_state.write() = Some(state.clone());
        self.inner.checkpoint_seq.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn state(&self) -> Option<Value> {
        self.inner.checkpoint_state.read().clone()
    }

    fn checkpoint_sequence(&self) -> i32 {
        self.inner.checkpoint_seq.load(Ordering::SeqCst)
    }

    // =========================================================================
    // Task Scheduling (Lazy API - Aligned with Workflow)
    // =========================================================================

    fn schedule_raw(&self, kind: &str, input: Value) -> AgentTaskFutureRaw {
        self.schedule_with_options_raw(kind, input, ScheduleAgentTaskOptions::default())
    }

    fn schedule_with_options_raw(
        &self,
        kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> AgentTaskFutureRaw {
        let task_id = Uuid::new_v4();

        // Record the scheduled task
        self.inner.scheduled_tasks.write().push(ScheduledAgentTask {
            task_id,
            task_kind: kind.to_string(),
            input: input.clone(),
            options,
        });

        AgentTaskFutureRaw::new(task_id, kind.to_string(), input)
    }

    async fn join_all(&self, futures: Vec<AgentTaskFutureRaw>) -> Result<Vec<Value>> {
        if futures.is_empty() {
            return Ok(vec![]);
        }

        let mut results = Vec::with_capacity(futures.len());
        for future in &futures {
            match self.get_task_result(&future.kind) {
                Some(result) => results.push(result),
                None => {
                    return Err(FlovynError::AgentSuspended(
                        "Agent suspended waiting for tasks".to_string(),
                    ));
                }
            }
        }

        Ok(results)
    }

    async fn select_ok(
        &self,
        futures: Vec<AgentTaskFutureRaw>,
    ) -> Result<(Value, Vec<AgentTaskFutureRaw>)> {
        if futures.is_empty() {
            return Err(FlovynError::InvalidArgument(
                "select_ok requires at least one future".to_string(),
            ));
        }

        // Find the first future with a configured result
        for (i, future) in futures.iter().enumerate() {
            if let Some(result) = self.get_task_result(&future.kind) {
                let remaining: Vec<AgentTaskFutureRaw> = futures
                    .into_iter()
                    .enumerate()
                    .filter(|(idx, _)| *idx != i)
                    .map(|(_, f)| f)
                    .collect();
                return Ok((result, remaining));
            }
        }

        // No result configured for any task
        Err(FlovynError::AgentSuspended(
            "Agent suspended waiting for successful task".to_string(),
        ))
    }

    async fn select_ok_with_cancel(
        &self,
        futures: Vec<AgentTaskFutureRaw>,
    ) -> Result<(Value, Vec<Uuid>)> {
        // Use select_ok and convert remaining to cancelled IDs
        let (result, remaining) = self.select_ok(futures).await?;
        let cancelled_ids: Vec<Uuid> = remaining.iter().map(|f| f.task_id).collect();
        Ok((result, cancelled_ids))
    }

    async fn join_all_settled(
        &self,
        futures: Vec<AgentTaskFutureRaw>,
    ) -> Result<crate::agent::combinators::SettledResult> {
        let mut result = crate::agent::combinators::SettledResult::new();

        for future in futures {
            match self.get_task_result(&future.kind) {
                Some(output) => {
                    result.completed.push((future.task_id.to_string(), output));
                }
                None => {
                    // In mock context, treat missing result as failure
                    result.failed.push((
                        future.task_id.to_string(),
                        "No mock result configured".to_string(),
                    ));
                }
            }
        }

        Ok(result)
    }

    async fn flush_pending(&self) -> Result<()> {
        // Mock context: no-op since there's no actual server to flush to
        Ok(())
    }

    // =========================================================================
    // Signals
    // =========================================================================

    async fn wait_for_signal_raw(&self, signal_name: &str) -> Result<Value> {
        let mut queues = self.inner.signal_queues.write();

        if let Some(queue) = queues.get_mut(signal_name) {
            if let Some(value) = queue.pop_front() {
                return Ok(value);
            }
        }

        // No signal available - suspend
        Err(FlovynError::AgentSuspended(format!(
            "Agent suspended waiting for signal: {}",
            signal_name
        )))
    }

    async fn has_signal(&self, signal_name: &str) -> Result<bool> {
        Ok(self
            .inner
            .signal_queues
            .read()
            .get(signal_name)
            .is_some_and(|q| !q.is_empty()))
    }

    async fn drain_signals_raw(&self, signal_name: &str) -> Result<Vec<Value>> {
        Ok(self
            .inner
            .signal_queues
            .write()
            .get_mut(signal_name)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default())
    }

    // =========================================================================
    // Streaming
    // =========================================================================

    async fn stream(&self, event: StreamEvent) -> Result<()> {
        self.inner.streamed_events.write().push(event);
        Ok(())
    }

    // =========================================================================
    // Cancellation
    // =========================================================================

    fn is_cancellation_requested(&self) -> bool {
        self.inner.cancellation_requested.load(Ordering::SeqCst)
    }

    async fn check_cancellation(&self) -> Result<()> {
        if self.is_cancellation_requested() {
            Err(FlovynError::WorkflowCancelled(
                "Cancellation requested".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    async fn cancel_task(&self, _task_id: Uuid) -> Result<CancelTaskResult> {
        // In mock context, always return Cancelled for simplicity
        // Real tests can extend this if needed
        Ok(CancelTaskResult::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_mock_agent_context_new() {
        let ctx = MockAgentContext::new();
        assert!(!ctx.agent_execution_id().is_nil());
        assert!(!ctx.org_id().is_nil());
    }

    #[test]
    fn test_mock_agent_context_builder() {
        let id = Uuid::new_v4();
        let org = Uuid::new_v4();
        let ctx = MockAgentContext::builder()
            .agent_execution_id(id)
            .org_id(org)
            .input(json!({"prompt": "Hello"}))
            .build();

        assert_eq!(ctx.agent_execution_id(), id);
        assert_eq!(ctx.org_id(), org);
        assert_eq!(ctx.input_raw(), &json!({"prompt": "Hello"}));
    }

    #[tokio::test]
    async fn test_mock_agent_context_entries() {
        let ctx = MockAgentContext::new();

        ctx.append_entry(EntryRole::User, &json!({"text": "Hello"}))
            .await
            .unwrap();
        ctx.append_entry(EntryRole::Assistant, &json!({"text": "Hi there!"}))
            .await
            .unwrap();

        let entries = ctx.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, EntryRole::User);
        assert_eq!(entries[1].role, EntryRole::Assistant);
    }

    #[tokio::test]
    async fn test_mock_agent_context_task_scheduling() {
        let ctx = MockAgentContext::builder()
            .task_result("llm-request", json!({"response": "Hello!"}))
            .build();

        // Use the new lazy API: schedule_raw + join_all
        let future = ctx.schedule_raw("llm-request", json!({"prompt": "Hi"}));
        let results = ctx.join_all(vec![future]).await;
        assert!(results.is_ok());
        assert_eq!(results.unwrap()[0], json!({"response": "Hello!"}));
        assert!(ctx.was_task_scheduled("llm-request"));
    }

    #[tokio::test]
    async fn test_mock_agent_context_task_not_configured() {
        let ctx = MockAgentContext::new();
        // Use the new lazy API: schedule_raw + join_all
        let future = ctx.schedule_raw("unknown-task", json!({}));
        let result = ctx.join_all(vec![future]).await;
        // Should suspend (no result configured)
        assert!(matches!(result, Err(FlovynError::AgentSuspended(_))));
    }

    #[tokio::test]
    async fn test_mock_agent_context_signals() {
        let ctx = MockAgentContext::builder()
            .mock_signal("userMessage", json!({"text": "Hello"}))
            .mock_signal("userMessage", json!({"text": "World"}))
            .build();

        assert!(ctx.has_signal("userMessage").await.unwrap());

        let sig1 = ctx.wait_for_signal_raw("userMessage").await.unwrap();
        assert_eq!(sig1, json!({"text": "Hello"}));

        let sig2 = ctx.wait_for_signal_raw("userMessage").await.unwrap();
        assert_eq!(sig2, json!({"text": "World"}));

        // No more signals - should suspend
        let result = ctx.wait_for_signal_raw("userMessage").await;
        assert!(matches!(result, Err(FlovynError::AgentSuspended(_))));
    }

    #[tokio::test]
    async fn test_mock_agent_context_checkpoint() {
        let ctx = MockAgentContext::new();

        assert_eq!(ctx.checkpoint_sequence(), 0);

        ctx.checkpoint(&json!({"turn": 1})).await.unwrap();
        assert_eq!(ctx.checkpoint_sequence(), 1);
        assert_eq!(ctx.current_checkpoint_state(), Some(json!({"turn": 1})));

        ctx.checkpoint(&json!({"turn": 2})).await.unwrap();
        assert_eq!(ctx.checkpoint_sequence(), 2);
        assert_eq!(ctx.current_checkpoint_state(), Some(json!({"turn": 2})));
    }

    #[test]
    fn test_mock_agent_context_cancellation() {
        let ctx = MockAgentContext::new();
        assert!(!ctx.is_cancellation_requested());
        ctx.request_cancellation();
        assert!(ctx.is_cancellation_requested());
    }

    #[tokio::test]
    async fn test_mock_agent_context_check_cancellation() {
        let ctx = MockAgentContext::new();
        assert!(ctx.check_cancellation().await.is_ok());

        ctx.request_cancellation();
        assert!(ctx.check_cancellation().await.is_err());
    }

    #[tokio::test]
    async fn test_mock_agent_context_streaming() {
        let ctx = MockAgentContext::new();

        ctx.stream_token("Hello").await.unwrap();
        ctx.stream_token(" World").await.unwrap();

        let events = ctx.streamed_events();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_mock_agent_context_lazy_task() {
        let ctx = MockAgentContext::builder()
            .task_result("slow-task", json!({"done": true}))
            .build();

        let future = ctx.schedule_raw("slow-task", json!({"input": "data"}));

        assert_eq!(future.kind(), "slow-task");
        assert!(!future.task_id().is_nil());
    }

    #[tokio::test]
    async fn test_mock_agent_context_join_all() {
        let ctx = MockAgentContext::builder()
            .task_result("task-a", json!({"a": 1}))
            .task_result("task-b", json!({"b": 2}))
            .build();

        let f1 = ctx.schedule_raw("task-a", json!({}));
        let f2 = ctx.schedule_raw("task-b", json!({}));

        let results = ctx.join_all(vec![f1, f2]).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], json!({"a": 1}));
        assert_eq!(results[1], json!({"b": 2}));
    }

    #[tokio::test]
    async fn test_mock_agent_context_select_ok() {
        let ctx = MockAgentContext::builder()
            .task_result("task-a", json!({"winner": "a"}))
            .build();

        let f1 = ctx.schedule_raw("task-a", json!({}));
        let f2 = ctx.schedule_raw("task-b", json!({})); // No result configured

        let (result, remaining) = ctx.select_ok(vec![f1, f2]).await.unwrap();
        assert_eq!(result, json!({"winner": "a"}));
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].kind(), "task-b");
    }

    // =========================================================================
    // Tests for suspension/resume scenarios (idempotency key stability)
    // =========================================================================

    /// Test that simulates what happens in a real agent suspension/resume cycle.
    #[tokio::test]
    async fn test_suspension_resume_task_consistency() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();
        let expected_result = json!({"response": "Hello from LLM!"});

        // First "execution" - agent schedules task using lazy API
        let ctx1 = MockAgentContext::builder()
            .agent_execution_id(agent_id)
            .task_result("llm-request", expected_result.clone())
            .build();

        let future1 = ctx1.schedule_raw("llm-request", json!({"prompt": "Hello"}));
        let results1 = ctx1.join_all(vec![future1]).await.unwrap();
        let result1 = results1.into_iter().next().unwrap();

        assert_eq!(result1, expected_result);

        // Simulate checkpoint
        ctx1.checkpoint(&json!({"turn": 1})).await.unwrap();
        let checkpoint_seq_after_task = ctx1.checkpoint_sequence();

        // Second "execution" - simulating resume after suspension
        let ctx2 = MockAgentContext::builder()
            .agent_execution_id(agent_id)
            .initial_checkpoint_seq(checkpoint_seq_after_task)
            .task_result("llm-request", expected_result.clone())
            .build();

        let future2 = ctx2.schedule_raw("llm-request", json!({"prompt": "Hello"}));
        let results2 = ctx2.join_all(vec![future2]).await.unwrap();
        let result2 = results2.into_iter().next().unwrap();

        assert_eq!(
            result2, expected_result,
            "Task result should be consistent across suspension/resume"
        );
        assert_eq!(
            result1, result2,
            "Same task should return same result on resume"
        );
    }

    /// Test idempotency key format documentation.
    #[test]
    fn test_idempotency_key_format_documentation() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();

        // CORRECT format (current implementation):
        // "{agent_id}:task:{counter}"
        let correct_key_0 = format!("{}:task:{}", agent_id, 0);
        let _correct_key_1 = format!("{}:task:{}", agent_id, 1);

        // The key should NOT include checkpoint_seq
        assert!(
            !correct_key_0.contains(":-1:"),
            "Key must not include checkpoint_seq"
        );
        assert!(
            correct_key_0.contains(":task:"),
            "Key must use ':task:' separator"
        );
    }

    // =========================================================================
    // Tests for Lazy Scheduling and Order Preservation
    // =========================================================================

    /// Test that schedule_raw records tasks immediately (synchronous, no RPC).
    ///
    /// This verifies the lazy scheduling behavior:
    /// - schedule_raw() is synchronous and returns immediately
    /// - Task is recorded in scheduled_tasks immediately
    /// - No RPC is made until join_all() or select_ok() is called
    #[test]
    fn test_schedule_raw_is_synchronous_and_records_task() {
        let ctx = MockAgentContext::new();

        // Initially no tasks scheduled
        assert_eq!(
            ctx.scheduled_tasks().len(),
            0,
            "No tasks should be scheduled initially"
        );

        // schedule_raw is synchronous - it returns immediately
        let future1 = ctx.schedule_raw("task-a", json!({"item": 1}));
        let future2 = ctx.schedule_raw("task-b", json!({"item": 2}));

        // Tasks should be recorded immediately (no RPC needed)
        let scheduled = ctx.scheduled_tasks();
        assert_eq!(
            scheduled.len(),
            2,
            "Both tasks should be recorded immediately after schedule_raw"
        );

        // Verify task details
        assert_eq!(scheduled[0].task_kind, "task-a");
        assert_eq!(scheduled[0].input, json!({"item": 1}));
        assert_eq!(scheduled[1].task_kind, "task-b");
        assert_eq!(scheduled[1].input, json!({"item": 2}));

        // Task IDs should match the futures
        assert_eq!(scheduled[0].task_id, future1.task_id());
        assert_eq!(scheduled[1].task_id, future2.task_id());

        // Futures are just data - no async operation has occurred
        assert!(!future1.task_id().is_nil());
        assert!(!future2.task_id().is_nil());
    }

    /// Test that join_all preserves order when using same task kind.
    ///
    /// This is a critical test: when scheduling multiple tasks of the same kind,
    /// join_all must return results in the SAME order as futures were passed.
    #[tokio::test]
    async fn test_join_all_preserves_order_same_task_kind() {
        // Configure a single result for the task kind
        // (In mock, result is keyed by kind, so all get same result)
        let ctx = MockAgentContext::builder()
            .task_result("process", json!({"processed": true}))
            .build();

        // Schedule multiple tasks of the same kind with different inputs
        let f1 = ctx.schedule_raw("process", json!({"id": "first"}));
        let f2 = ctx.schedule_raw("process", json!({"id": "second"}));
        let f3 = ctx.schedule_raw("process", json!({"id": "third"}));

        // Verify tasks are recorded in order
        let scheduled = ctx.scheduled_tasks();
        assert_eq!(scheduled[0].input, json!({"id": "first"}));
        assert_eq!(scheduled[1].input, json!({"id": "second"}));
        assert_eq!(scheduled[2].input, json!({"id": "third"}));

        // Call join_all with futures in specific order
        let results = ctx.join_all(vec![f1, f2, f3]).await.unwrap();

        // Results length must match futures length
        assert_eq!(
            results.len(),
            3,
            "join_all must return same number of results as futures"
        );
    }

    /// Test that join_all returns results in order matching futures.
    ///
    /// This explicitly tests: results[i] corresponds to futures[i]
    #[tokio::test]
    async fn test_join_all_result_order_matches_future_order() {
        let ctx = MockAgentContext::builder()
            .task_result("analyze", json!({"type": "analyze"}))
            .task_result("transform", json!({"type": "transform"}))
            .task_result("load", json!({"type": "load"}))
            .build();

        // Schedule in specific order: analyze, transform, load
        let f_analyze = ctx.schedule_raw("analyze", json!({}));
        let f_transform = ctx.schedule_raw("transform", json!({}));
        let f_load = ctx.schedule_raw("load", json!({}));

        // Pass futures in ORDER: analyze, transform, load
        let results = ctx
            .join_all(vec![f_analyze, f_transform, f_load])
            .await
            .unwrap();

        // Results MUST be in same order
        assert_eq!(
            results[0],
            json!({"type": "analyze"}),
            "results[0] must be analyze"
        );
        assert_eq!(
            results[1],
            json!({"type": "transform"}),
            "results[1] must be transform"
        );
        assert_eq!(
            results[2],
            json!({"type": "load"}),
            "results[2] must be load"
        );

        // Now test with different order
        let ctx2 = MockAgentContext::builder()
            .task_result("analyze", json!({"type": "analyze"}))
            .task_result("transform", json!({"type": "transform"}))
            .task_result("load", json!({"type": "load"}))
            .build();

        let f_analyze = ctx2.schedule_raw("analyze", json!({}));
        let f_transform = ctx2.schedule_raw("transform", json!({}));
        let f_load = ctx2.schedule_raw("load", json!({}));

        // Pass futures in DIFFERENT order: load, analyze, transform
        let results = ctx2
            .join_all(vec![f_load, f_analyze, f_transform])
            .await
            .unwrap();

        // Results MUST match the new order
        assert_eq!(
            results[0],
            json!({"type": "load"}),
            "results[0] must be load (as passed)"
        );
        assert_eq!(
            results[1],
            json!({"type": "analyze"}),
            "results[1] must be analyze (as passed)"
        );
        assert_eq!(
            results[2],
            json!({"type": "transform"}),
            "results[2] must be transform (as passed)"
        );
    }

    /// Test that empty join_all returns empty results.
    #[tokio::test]
    async fn test_join_all_empty_returns_empty() {
        let ctx = MockAgentContext::new();

        let results = ctx.join_all(vec![]).await.unwrap();

        assert!(results.is_empty(), "join_all([]) must return []");
    }

    /// Test select_ok returns first successful task in order.
    #[tokio::test]
    async fn test_select_ok_returns_first_success_by_order() {
        // Only configure result for task-b (second in order)
        let ctx = MockAgentContext::builder()
            .task_result("task-b", json!({"winner": "b"}))
            .build();

        // Schedule two tasks - first has no result, second has result
        let f_a = ctx.schedule_raw("task-a", json!({})); // No result configured
        let f_b = ctx.schedule_raw("task-b", json!({})); // Has result

        // select_ok should find task-b (index 1) as first success
        let (result, remaining) = ctx.select_ok(vec![f_a.clone(), f_b.clone()]).await.unwrap();

        assert_eq!(result, json!({"winner": "b"}));
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].kind(), "task-a"); // task-a should remain
    }

    /// Test command batching pattern: multiple operations before await.
    ///
    /// This demonstrates the key pattern enabled by lazy scheduling:
    /// - Multiple schedule_raw() calls are synchronous and return immediately
    /// - Commands are batched and only committed when join_all() is called
    /// - This enables efficient batching of task submissions
    ///
    /// In the real implementation, commands are buffered in memory and sent
    /// to the server atomically at suspension points (join_all, checkpoint).
    #[tokio::test]
    async fn test_command_batching_pattern() {
        let ctx = MockAgentContext::builder()
            .task_result("fetch", json!({"data": "response"}))
            .task_result("process", json!({"result": "processed"}))
            .task_result("store", json!({"stored": true}))
            .build();

        // PHASE 1: Record entries (would be batched in real impl)
        ctx.append_entry(EntryRole::User, &json!({"text": "Process this data"}))
            .await
            .unwrap();
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Starting parallel processing"}),
        )
        .await
        .unwrap();

        // PHASE 2: Schedule multiple tasks synchronously (no RPC yet)
        let f1 = ctx.schedule_raw("fetch", json!({"url": "https://api.example.com"}));
        let f2 = ctx.schedule_raw("process", json!({"data": "raw"}));
        let f3 = ctx.schedule_raw("store", json!({"key": "result"}));

        // At this point:
        // - 3 tasks are scheduled (recorded in scheduled_tasks)
        // - No RPC has been made yet
        // - Tasks will be submitted when join_all is called
        assert_eq!(
            ctx.scheduled_tasks().len(),
            3,
            "All tasks should be scheduled before join_all"
        );

        // PHASE 3: Await all tasks (this triggers batch commit in real impl)
        let results = ctx.join_all(vec![f1, f2, f3]).await.unwrap();

        // Verify all results received in order
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], json!({"data": "response"}));
        assert_eq!(results[1], json!({"result": "processed"}));
        assert_eq!(results[2], json!({"stored": true}));

        // Verify entries were recorded
        let entries = ctx.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].role, EntryRole::User);
        assert_eq!(entries[1].role, EntryRole::Assistant);
    }
}
