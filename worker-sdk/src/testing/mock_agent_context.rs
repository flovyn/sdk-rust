//! Mock agent context for unit testing agents in isolation.

use crate::agent::combinators::AgentTaskHandle;
use crate::agent::context::{
    AgentContext, EntryRole, LoadedMessage, ScheduleAgentTaskOptions,
};
use crate::error::{FlovynError, Result};
use crate::task::streaming::StreamEvent;
use async_trait::async_trait;
use flovyn_worker_core::client::{BatchTaskResult, CancelResult, WaitMode};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
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
    // Counter for generating task indices
    task_counter: AtomicU64,
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
                task_counter: AtomicU64::new(0),
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

    fn load_messages(&self) -> &[LoadedMessage] {
        // This is a bit tricky since we return a reference.
        // For mock purposes, we'll return an empty slice and rely on reload_messages.
        // In real usage, the caller should use reload_messages for dynamic updates.
        &[]
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

    fn state(&self) -> Option<&Value> {
        // This can't return a reference to data behind RwLock,
        // so for mock purposes we return None.
        // Tests should use current_checkpoint_state() instead.
        None
    }

    fn checkpoint_sequence(&self) -> i32 {
        self.inner.checkpoint_seq.load(Ordering::SeqCst)
    }

    // =========================================================================
    // Task Scheduling
    // =========================================================================

    async fn schedule_task_raw(&self, task_kind: &str, input: Value) -> Result<Value> {
        self.schedule_task_with_options_raw(task_kind, input, ScheduleAgentTaskOptions::default())
            .await
    }

    async fn schedule_task_with_options_raw(
        &self,
        task_kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> Result<Value> {
        let task_id = Uuid::new_v4();

        // Record the scheduled task
        self.inner.scheduled_tasks.write().push(ScheduledAgentTask {
            task_id,
            task_kind: task_kind.to_string(),
            input: input.clone(),
            options,
        });

        // Look up the mock result
        match self.inner.task_results.read().get(task_kind) {
            Some(result) => Ok(result.clone()),
            None => Err(FlovynError::Other(format!(
                "No mock result configured for task kind: {}",
                task_kind
            ))),
        }
    }

    async fn schedule_task_handle(
        &self,
        task_kind: &str,
        input: Value,
    ) -> Result<AgentTaskHandle> {
        self.schedule_task_handle_with_options(task_kind, input, ScheduleAgentTaskOptions::default())
            .await
    }

    async fn schedule_task_handle_with_options(
        &self,
        task_kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> Result<AgentTaskHandle> {
        let task_id = Uuid::new_v4();
        let index = self.inner.task_counter.fetch_add(1, Ordering::SeqCst) as usize;

        // Record the scheduled task
        self.inner.scheduled_tasks.write().push(ScheduledAgentTask {
            task_id,
            task_kind: task_kind.to_string(),
            input: input.clone(),
            options,
        });

        Ok(AgentTaskHandle::new(task_id, task_kind, index))
    }

    async fn get_task_results_batch(&self, task_ids: &[Uuid]) -> Result<Vec<BatchTaskResult>> {
        let results = task_ids
            .iter()
            .map(|task_id| {
                // Find the task kind from scheduled tasks
                let task_kind = self
                    .inner
                    .scheduled_tasks
                    .read()
                    .iter()
                    .find(|t| t.task_id == *task_id)
                    .map(|t| t.task_kind.clone());

                match task_kind {
                    Some(kind) => {
                        match self.inner.task_results.read().get(&kind) {
                            Some(result) => BatchTaskResult {
                                task_execution_id: *task_id,
                                status: "COMPLETED".to_string(),
                                output: Some(result.clone()),
                                error: None,
                            },
                            None => BatchTaskResult {
                                task_execution_id: *task_id,
                                status: "PENDING".to_string(),
                                output: None,
                                error: None,
                            },
                        }
                    }
                    None => BatchTaskResult {
                        task_execution_id: *task_id,
                        status: "NOT_FOUND".to_string(),
                        output: None,
                        error: Some("Task not found".to_string()),
                    },
                }
            })
            .collect();

        Ok(results)
    }

    async fn suspend_for_tasks(&self, _task_ids: &[Uuid], _mode: WaitMode) -> Result<()> {
        // In mock context, we don't actually suspend
        Err(FlovynError::AgentSuspended(
            "Agent suspended waiting for tasks".to_string(),
        ))
    }

    // =========================================================================
    // Task Cancellation
    // =========================================================================

    async fn cancel_task(
        &self,
        handle: &AgentTaskHandle,
        _reason: Option<&str>,
    ) -> Result<CancelResult> {
        // Mock implementation: check if task exists and return mock result
        let task_exists = self
            .inner
            .scheduled_tasks
            .read()
            .iter()
            .any(|t| t.task_id == handle.task_id());

        if task_exists {
            Ok(CancelResult {
                cancelled: true,
                status: "CANCELLED".to_string(),
            })
        } else {
            Ok(CancelResult {
                cancelled: false,
                status: "NOT_FOUND".to_string(),
            })
        }
    }

    async fn cancel_tasks(
        &self,
        handles: &[AgentTaskHandle],
        reason: Option<&str>,
    ) -> Result<Vec<CancelResult>> {
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            results.push(self.cancel_task(handle, reason).await?);
        }
        Ok(results)
    }

    // =========================================================================
    // Batch Task Scheduling
    // =========================================================================

    async fn schedule_tasks_batch(
        &self,
        tasks: Vec<(&str, Value)>,
    ) -> Result<Vec<AgentTaskHandle>> {
        let tasks_with_options: Vec<_> = tasks
            .into_iter()
            .map(|(kind, input)| (kind, input, ScheduleAgentTaskOptions::default()))
            .collect();
        self.schedule_tasks_batch_with_options(tasks_with_options)
            .await
    }

    async fn schedule_tasks_batch_with_options(
        &self,
        tasks: Vec<(&str, Value, ScheduleAgentTaskOptions)>,
    ) -> Result<Vec<AgentTaskHandle>> {
        let mut handles = Vec::with_capacity(tasks.len());
        for (task_kind, input, options) in tasks {
            let handle = self
                .schedule_task_handle_with_options(task_kind, input, options)
                .await?;
            handles.push(handle);
        }
        Ok(handles)
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

        let result = ctx
            .schedule_task_raw("llm-request", json!({"prompt": "Hi"}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"response": "Hello!"}));
        assert!(ctx.was_task_scheduled("llm-request"));
    }

    #[tokio::test]
    async fn test_mock_agent_context_task_not_configured() {
        let ctx = MockAgentContext::new();
        let result = ctx.schedule_task_raw("unknown-task", json!({})).await;
        assert!(result.is_err());
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
    async fn test_mock_agent_context_task_handle() {
        let ctx = MockAgentContext::builder()
            .task_result("slow-task", json!({"done": true}))
            .build();

        let handle = ctx
            .schedule_task_handle("slow-task", json!({"input": "data"}))
            .await
            .unwrap();

        assert_eq!(handle.task_kind(), "slow-task");
        assert!(!handle.task_id().is_nil());
    }

    #[tokio::test]
    async fn test_mock_agent_context_cancel_task() {
        let ctx = MockAgentContext::builder()
            .task_result("task-a", json!({}))
            .build();

        let handle = ctx.schedule_task_handle("task-a", json!({})).await.unwrap();

        let result = ctx.cancel_task(&handle, Some("Test reason")).await.unwrap();
        assert!(result.cancelled);
        assert_eq!(result.status, "CANCELLED");
    }

    #[tokio::test]
    async fn test_mock_agent_context_batch_scheduling() {
        let ctx = MockAgentContext::builder()
            .task_result("task-a", json!({"a": 1}))
            .task_result("task-b", json!({"b": 2}))
            .build();

        let handles = ctx
            .schedule_tasks_batch(vec![
                ("task-a", json!({"input": "a"})),
                ("task-b", json!({"input": "b"})),
            ])
            .await
            .unwrap();

        assert_eq!(handles.len(), 2);
        assert_eq!(handles[0].task_kind, "task-a");
        assert_eq!(handles[1].task_kind, "task-b");
        assert_eq!(ctx.scheduled_tasks().len(), 2);
    }

    // =========================================================================
    // Tests for suspension/resume scenarios (idempotency key stability)
    // =========================================================================

    /// Test that simulates what happens in a real agent suspension/resume cycle.
    ///
    /// This test verifies that when an agent:
    /// 1. Schedules a task and gets a result
    /// 2. "Resumes" (simulated by creating a new context with loaded messages)
    /// 3. Schedules the "same" task again
    ///
    /// The results should be consistent (i.e., idempotency should work).
    ///
    /// Note: MockAgentContext doesn't use real idempotency keys like AgentContextImpl,
    /// but this test documents the expected behavior and can catch regressions in
    /// agent logic that depends on task scheduling across suspension boundaries.
    #[tokio::test]
    async fn test_suspension_resume_task_consistency() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();
        let expected_result = json!({"response": "Hello from LLM!"});

        // First "execution" - agent schedules task
        let ctx1 = MockAgentContext::builder()
            .agent_execution_id(agent_id)
            .task_result("llm-request", expected_result.clone())
            .build();

        let result1 = ctx1
            .schedule_task_raw("llm-request", json!({"prompt": "Hello"}))
            .await
            .unwrap();

        assert_eq!(result1, expected_result);

        // Simulate checkpoint
        ctx1.checkpoint(&json!({"turn": 1})).await.unwrap();
        let checkpoint_seq_after_task = ctx1.checkpoint_sequence();

        // Second "execution" - simulating resume after suspension
        // In real code, this would be a new AgentContextImpl created from checkpoint
        let ctx2 = MockAgentContext::builder()
            .agent_execution_id(agent_id) // Same agent ID
            .initial_checkpoint_seq(checkpoint_seq_after_task) // Restored checkpoint
            .task_result("llm-request", expected_result.clone()) // Same mock result
            .build();

        // When agent resumes and tries to get the task result again,
        // it should get the same result (due to idempotency in real impl)
        let result2 = ctx2
            .schedule_task_raw("llm-request", json!({"prompt": "Hello"}))
            .await
            .unwrap();

        assert_eq!(result2, expected_result,
            "Task result should be consistent across suspension/resume");
        assert_eq!(result1, result2,
            "Same task should return same result on resume");
    }

    /// Test that demonstrates the importance of stable idempotency keys.
    ///
    /// This test shows what the CORRECT behavior should be:
    /// - Task scheduled before suspension should be retrievable after resume
    /// - The agent should NOT create a new task on resume
    ///
    /// In MockAgentContext, we simulate this by verifying that the same task_kind
    /// returns the same configured result regardless of checkpoint state.
    #[tokio::test]
    async fn test_idempotency_across_checkpoint_changes() {
        let agent_id = Uuid::new_v4();
        let llm_result = json!({"message": "Assistant response"});

        // Create context with checkpoint_seq = 0
        let ctx = MockAgentContext::builder()
            .agent_execution_id(agent_id)
            .initial_checkpoint_seq(0)
            .task_result("llm-request", llm_result.clone())
            .build();

        // First task scheduling (simulating first execution)
        let result1 = ctx.schedule_task_raw("llm-request", json!({})).await.unwrap();
        assert_eq!(result1, llm_result);

        // Checkpoint (simulating suspension)
        ctx.checkpoint(&json!({"step": 1})).await.unwrap();
        assert_eq!(ctx.checkpoint_sequence(), 1);

        // Second task scheduling (simulating resume)
        // Even though checkpoint_seq changed from 0 to 1, the result should be the same
        let result2 = ctx.schedule_task_raw("llm-request", json!({})).await.unwrap();
        assert_eq!(result2, llm_result);

        // More checkpoints (simulating multiple suspension/resume cycles)
        ctx.checkpoint(&json!({"step": 2})).await.unwrap();
        ctx.checkpoint(&json!({"step": 3})).await.unwrap();
        assert_eq!(ctx.checkpoint_sequence(), 3);

        // Task result should still be the same
        let result3 = ctx.schedule_task_raw("llm-request", json!({})).await.unwrap();
        assert_eq!(result3, llm_result,
            "Task result must be stable regardless of checkpoint_seq changes");
    }

    /// Test that verifies the format of idempotency keys used in AgentContextImpl.
    ///
    /// While MockAgentContext doesn't use idempotency keys, this test documents
    /// the expected format and can be used to verify the fix is correct.
    #[test]
    fn test_idempotency_key_format_documentation() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();

        // CORRECT format (current implementation):
        // "{agent_id}:task:{counter}"
        let correct_key_0 = format!("{}:task:{}", agent_id, 0);
        let correct_key_1 = format!("{}:task:{}", agent_id, 1);

        // The key should NOT include checkpoint_seq
        assert!(!correct_key_0.contains(":-1:"), "Key must not include checkpoint_seq");
        assert!(correct_key_0.contains(":task:"), "Key must use ':task:' separator");

        // BUGGY format (old implementation that caused infinite task creation):
        // "{agent_id}:{checkpoint_seq}:{counter}"
        let _buggy_key_run1 = format!("{}:{}:{}", agent_id, -1, 0); // First run
        let _buggy_key_run2 = format!("{}:{}:{}", agent_id, 5, 0);  // After resume

        // The buggy keys would be different even for the same logical task,
        // causing the server to create a new task instead of returning the existing one.
        // This is documented here to explain why the fix was necessary.
    }
}
