//! Test builders for fluent API test setup.

use crate::workflow::event::{EventType, ReplayEvent};
use chrono::Utc;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

use super::{MockTaskContext, MockWorkflowContext};

/// Builder for setting up workflow tests with replay events.
///
/// This builder allows you to construct a test scenario with specific
/// events to simulate various workflow states.
///
/// # Example
///
/// ```ignore
/// use flovyn_sdk::testing::WorkflowTestBuilder;
/// use serde_json::json;
///
/// let (ctx, events) = WorkflowTestBuilder::new()
///     .input(json!({"amount": 100}))
///     .with_operation_completed("fetch-data", json!({"data": "value"}))
///     .with_task_completed("process-data", json!({"processed": true}))
///     .build();
/// ```
#[derive(Default)]
pub struct WorkflowTestBuilder {
    workflow_execution_id: Option<Uuid>,
    org_id: Option<Uuid>,
    input: Option<Value>,
    initial_time_millis: Option<i64>,
    events: Vec<ReplayEvent>,
    sequence: i32,
    task_results: Vec<(String, Value)>,
    promise_results: Vec<(String, Value)>,
    child_workflow_results: Vec<(String, Value)>,
}

impl WorkflowTestBuilder {
    /// Create a new workflow test builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the workflow execution ID.
    pub fn workflow_execution_id(mut self, id: Uuid) -> Self {
        self.workflow_execution_id = Some(id);
        self
    }

    /// Set the tenant ID.
    pub fn org_id(mut self, id: Uuid) -> Self {
        self.org_id = Some(id);
        self
    }

    /// Set the workflow input.
    pub fn input(mut self, input: Value) -> Self {
        self.input = Some(input);
        self
    }

    /// Set the initial time.
    pub fn initial_time_millis(mut self, time: i64) -> Self {
        self.initial_time_millis = Some(time);
        self
    }

    /// Add a workflow started event.
    pub fn with_workflow_started(mut self, input: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::WorkflowStarted,
            input,
            Utc::now(),
        ));
        self
    }

    /// Add an operation completed event (from ctx.run).
    pub fn with_operation_completed(mut self, name: &str, result: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::OperationCompleted,
            serde_json::json!({
                "operationName": name,
                "result": result
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a task scheduled event.
    pub fn with_task_scheduled(mut self, task_type: &str, input: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::TaskScheduled,
            serde_json::json!({
                "kind": task_type,
                "input": input
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a task completed event.
    pub fn with_task_completed(mut self, task_type: &str, result: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::TaskCompleted,
            serde_json::json!({
                "kind": task_type,
                "result": result
            }),
            Utc::now(),
        ));
        // Also add to mock task results
        self.task_results.push((task_type.to_string(), result));
        self
    }

    /// Add a task failed event.
    pub fn with_task_failed(mut self, task_type: &str, error: &str) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::TaskFailed,
            serde_json::json!({
                "kind": task_type,
                "error": error
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a state set event.
    pub fn with_state_set(mut self, key: &str, value: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::StateSet,
            serde_json::json!({
                "key": key,
                "value": value
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a state cleared event.
    pub fn with_state_cleared(mut self, key: &str) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::StateCleared,
            serde_json::json!({
                "key": key
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a timer started event.
    pub fn with_timer_started(mut self, timer_id: &str, duration: Duration) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::TimerStarted,
            serde_json::json!({
                "timerId": timer_id,
                "durationMs": duration.as_millis() as i64
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a timer fired event.
    pub fn with_timer_fired(mut self, timer_id: &str) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::TimerFired,
            serde_json::json!({
                "timerId": timer_id
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a promise created event.
    pub fn with_promise_created(mut self, promise_id: &str) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::PromiseCreated,
            serde_json::json!({
                "promiseId": promise_id
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a promise resolved event.
    pub fn with_promise_resolved(mut self, promise_id: &str, value: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::PromiseResolved,
            serde_json::json!({
                "promiseId": promise_id,
                "value": value
            }),
            Utc::now(),
        ));
        // Also add to mock promise results
        self.promise_results.push((promise_id.to_string(), value));
        self
    }

    /// Add a child workflow initiated event.
    pub fn with_child_workflow_initiated(mut self, name: &str, kind: &str, input: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::ChildWorkflowInitiated,
            serde_json::json!({
                "childName": name,
                "childKind": kind,
                "input": input
            }),
            Utc::now(),
        ));
        self
    }

    /// Add a child workflow completed event.
    pub fn with_child_workflow_completed(mut self, kind: &str, result: Value) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::ChildWorkflowCompleted,
            serde_json::json!({
                "childKind": kind,
                "result": result
            }),
            Utc::now(),
        ));
        // Also add to mock child workflow results
        self.child_workflow_results.push((kind.to_string(), result));
        self
    }

    /// Add a cancellation requested event.
    pub fn with_cancellation_requested(mut self) -> Self {
        self.sequence += 1;
        self.events.push(ReplayEvent::new(
            self.sequence,
            EventType::CancellationRequested,
            serde_json::json!({}),
            Utc::now(),
        ));
        self
    }

    /// Build the mock context and replay events.
    pub fn build(self) -> (MockWorkflowContext, Vec<ReplayEvent>) {
        let mut builder = MockWorkflowContext::builder();

        if let Some(id) = self.workflow_execution_id {
            builder = builder.workflow_execution_id(id);
        }
        if let Some(id) = self.org_id {
            builder = builder.org_id(id);
        }
        if let Some(input) = self.input {
            builder = builder.input(input);
        }
        if let Some(time) = self.initial_time_millis {
            builder = builder.initial_time_millis(time);
        }

        // Add task results
        for (task_type, result) in self.task_results {
            builder = builder.task_result(&task_type, result);
        }

        // Add promise results
        for (name, result) in self.promise_results {
            builder = builder.promise_result(&name, result);
        }

        // Add child workflow results
        for (kind, result) in self.child_workflow_results {
            builder = builder.child_workflow_result(&kind, result);
        }

        (builder.build(), self.events)
    }
}

/// Builder for setting up task tests.
///
/// # Example
///
/// ```ignore
/// use flovyn_sdk::testing::TaskTestBuilder;
/// use serde_json::json;
///
/// let ctx = TaskTestBuilder::new()
///     .attempt(2)
///     .build();
/// ```
#[derive(Default)]
pub struct TaskTestBuilder {
    task_execution_id: Option<Uuid>,
    attempt: Option<u32>,
    cancelled: bool,
}

impl TaskTestBuilder {
    /// Create a new task test builder.
    pub fn new() -> Self {
        Self::default()
    }

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

    /// Build the mock task context.
    pub fn build(self) -> MockTaskContext {
        let mut builder = MockTaskContext::builder();

        if let Some(id) = self.task_execution_id {
            builder = builder.task_execution_id(id);
        }
        if let Some(attempt) = self.attempt {
            builder = builder.attempt(attempt);
        }
        if self.cancelled {
            builder = builder.cancelled();
        }

        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::context::TaskContext;
    use crate::workflow::context::WorkflowContext;
    use serde_json::json;

    #[test]
    fn test_workflow_test_builder_default() {
        let (ctx, events) = WorkflowTestBuilder::new().build();
        assert!(!ctx.workflow_execution_id().is_nil());
        assert!(events.is_empty());
    }

    #[test]
    fn test_workflow_test_builder_with_input() {
        let (ctx, _) = WorkflowTestBuilder::new()
            .input(json!({"key": "value"}))
            .build();
        assert_eq!(ctx.input_raw(), &json!({"key": "value"}));
    }

    #[test]
    fn test_workflow_test_builder_with_events() {
        let (_, events) = WorkflowTestBuilder::new()
            .with_workflow_started(json!({}))
            .with_operation_completed("op1", json!({"result": "data"}))
            .build();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type(), EventType::WorkflowStarted);
        assert_eq!(events[1].event_type(), EventType::OperationCompleted);
    }

    #[test]
    fn test_workflow_test_builder_with_task_events() {
        let (ctx, events) = WorkflowTestBuilder::new()
            .with_task_scheduled("task1", json!({"input": "data"}))
            .with_task_completed("task1", json!({"output": "result"}))
            .build();

        assert_eq!(events.len(), 2);
        // Task result should be in context
        assert!(!ctx.was_task_scheduled("task1")); // Not yet scheduled via context
    }

    #[test]
    fn test_workflow_test_builder_with_timer_events() {
        let (_, events) = WorkflowTestBuilder::new()
            .with_timer_started("timer1", Duration::from_secs(60))
            .with_timer_fired("timer1")
            .build();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type(), EventType::TimerStarted);
        assert_eq!(events[1].event_type(), EventType::TimerFired);
    }

    #[test]
    fn test_workflow_test_builder_with_promise_events() {
        let (ctx, events) = WorkflowTestBuilder::new()
            .with_promise_created("approval")
            .with_promise_resolved("approval", json!({"approved": true}))
            .build();

        assert_eq!(events.len(), 2);
        // Promise result should be available in context mock
        let _ = ctx; // Just checking compilation
    }

    #[test]
    fn test_workflow_test_builder_with_child_workflow() {
        let (_, events) = WorkflowTestBuilder::new()
            .with_child_workflow_initiated("child", "child-workflow", json!({}))
            .with_child_workflow_completed("child-workflow", json!({"success": true}))
            .build();

        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_workflow_test_builder_with_cancellation() {
        let (_, events) = WorkflowTestBuilder::new()
            .with_cancellation_requested()
            .build();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type(), EventType::CancellationRequested);
    }

    #[test]
    fn test_workflow_test_builder_with_state_events() {
        let (_, events) = WorkflowTestBuilder::new()
            .with_state_set("counter", json!(42))
            .with_state_cleared("counter")
            .build();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type(), EventType::StateSet);
        assert_eq!(events[1].event_type(), EventType::StateCleared);
    }

    #[test]
    fn test_task_test_builder_default() {
        let ctx = TaskTestBuilder::new().build();
        assert!(!ctx.task_execution_id().is_nil());
        assert_eq!(ctx.attempt(), 1);
    }

    #[test]
    fn test_task_test_builder_with_attempt() {
        let ctx = TaskTestBuilder::new().attempt(3).build();
        assert_eq!(ctx.attempt(), 3);
    }

    #[test]
    fn test_task_test_builder_cancelled() {
        let ctx = TaskTestBuilder::new().cancelled().build();
        assert!(ctx.is_cancelled());
    }

    #[test]
    fn test_task_test_builder_full() {
        let id = Uuid::new_v4();
        let ctx = TaskTestBuilder::new()
            .task_execution_id(id)
            .attempt(5)
            .cancelled()
            .build();

        assert_eq!(ctx.task_execution_id(), id);
        assert_eq!(ctx.attempt(), 5);
        assert!(ctx.is_cancelled());
    }
}
