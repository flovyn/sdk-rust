//! Test workflow definitions for E2E tests

#![allow(dead_code)] // Fixtures will be used when tests are implemented

use async_trait::async_trait;
use flovyn_worker_sdk::error::{FlovynError, Result};
use flovyn_worker_sdk::workflow::context::WorkflowContext;
use flovyn_worker_sdk::workflow::definition::{DynamicInput, DynamicOutput, DynamicWorkflow};
use serde_json::Value;

/// Simple workflow that doubles a numeric value.
pub struct DoublerWorkflow;

#[async_trait]
impl DynamicWorkflow for DoublerWorkflow {
    fn kind(&self) -> &str {
        "doubler-workflow"
    }

    async fn execute(
        &self,
        _ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let value = input.get("value").and_then(|v| v.as_i64()).unwrap_or(0);

        let mut output = DynamicOutput::new();
        output.insert("result".to_string(), Value::Number((value * 2).into()));
        Ok(output)
    }
}

/// Simple workflow that echoes its input.
pub struct EchoWorkflow;

#[async_trait]
impl DynamicWorkflow for EchoWorkflow {
    fn kind(&self) -> &str {
        "echo-workflow"
    }

    async fn execute(
        &self,
        _ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        Ok(input)
    }
}

/// Workflow that always fails with an error.
pub struct FailingWorkflow {
    pub message: String,
}

impl FailingWorkflow {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Default for FailingWorkflow {
    fn default() -> Self {
        Self::new("Intentional workflow failure")
    }
}

#[async_trait]
impl DynamicWorkflow for FailingWorkflow {
    fn kind(&self) -> &str {
        "failing-workflow"
    }

    async fn execute(
        &self,
        _ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        Err(FlovynError::WorkflowFailed(self.message.clone()))
    }
}

/// Workflow that uses ctx.run_raw() to execute a local operation.
pub struct RunOperationWorkflow;

#[async_trait]
impl DynamicWorkflow for RunOperationWorkflow {
    fn kind(&self) -> &str {
        "run-operation-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let value = input.get("value").and_then(|v| v.as_i64()).unwrap_or(0);

        // Use ctx.run_raw() to execute a deterministic operation
        let doubled = Value::Number((value * 2).into());
        let result = ctx.run_raw("double", doubled).await?;

        let mut output = DynamicOutput::new();
        output.insert("result".to_string(), result);
        Ok(output)
    }
}

/// Workflow that demonstrates state management.
pub struct StatefulWorkflow;

#[async_trait]
impl DynamicWorkflow for StatefulWorkflow {
    fn kind(&self) -> &str {
        "stateful-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        // Set some state
        let key = input
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let value = input.get("value").cloned().unwrap_or(Value::Null);

        ctx.set_raw(key, value.clone()).await?;

        // Get it back
        let retrieved = ctx.get_raw(key).await?;

        let mut output = DynamicOutput::new();
        output.insert("stored".to_string(), value);
        output.insert("retrieved".to_string(), retrieved.unwrap_or(Value::Null));
        Ok(output)
    }
}

/// Workflow that schedules a task.
pub struct TaskSchedulingWorkflow;

#[async_trait]
impl DynamicWorkflow for TaskSchedulingWorkflow {
    fn kind(&self) -> &str {
        "task-scheduling-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        // Schedule the echo task
        let task_input = input
            .get("taskInput")
            .cloned()
            .unwrap_or(Value::Object(DynamicInput::new()));

        let task_result = ctx.schedule_raw("echo-task", task_input).await?;

        let mut output = DynamicOutput::new();
        output.insert("taskResult".to_string(), task_result);
        Ok(output)
    }
}

/// Workflow that schedules multiple tasks sequentially.
pub struct MultiTaskWorkflow;

#[async_trait]
impl DynamicWorkflow for MultiTaskWorkflow {
    fn kind(&self) -> &str {
        "multi-task-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        // Schedule echo task
        let echo_result = ctx
            .schedule_raw(
                "echo-task",
                serde_json::json!({ "message": "from multi-task workflow" }),
            )
            .await?;

        // Schedule slow task
        let slow_result = ctx
            .schedule_raw("slow-task", serde_json::json!({ "sleepMs": 100 }))
            .await?;

        let mut output = DynamicOutput::new();
        output.insert("taskCount".to_string(), Value::Number(2.into()));
        output.insert("echoResult".to_string(), echo_result);
        output.insert("slowResult".to_string(), slow_result);
        Ok(output)
    }
}

/// Workflow that sleeps for a configurable duration.
pub struct TimerWorkflow;

#[async_trait]
impl DynamicWorkflow for TimerWorkflow {
    fn kind(&self) -> &str {
        "timer-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let sleep_ms = input
            .get("sleepMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        let start_time = ctx.current_time_millis();

        // Sleep using durable timer
        ctx.sleep(std::time::Duration::from_millis(sleep_ms))
            .await?;

        let end_time = ctx.current_time_millis();

        let mut output = DynamicOutput::new();
        output.insert("sleptMs".to_string(), Value::Number(sleep_ms.into()));
        output.insert(
            "elapsedMs".to_string(),
            Value::Number((end_time - start_time).into()),
        );
        Ok(output)
    }
}

/// Workflow that waits for a promise to be resolved externally.
pub struct PromiseWorkflow;

#[async_trait]
impl DynamicWorkflow for PromiseWorkflow {
    fn kind(&self) -> &str {
        "promise-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let promise_name = input
            .get("promiseName")
            .and_then(|v| v.as_str())
            .unwrap_or("approval");

        // Wait for the promise to be resolved externally
        let promise_value = ctx.promise_raw(promise_name).await?;

        let mut output = DynamicOutput::new();
        output.insert(
            "promiseName".to_string(),
            Value::String(promise_name.to_string()),
        );
        output.insert("promiseValue".to_string(), promise_value);
        Ok(output)
    }
}

/// Workflow that waits for a promise with timeout.
pub struct PromiseWithTimeoutWorkflow;

#[async_trait]
impl DynamicWorkflow for PromiseWithTimeoutWorkflow {
    fn kind(&self) -> &str {
        "promise-timeout-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let promise_name = input
            .get("promiseName")
            .and_then(|v| v.as_str())
            .unwrap_or("approval");
        let timeout_ms = input
            .get("timeoutMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000);

        // Wait for the promise with timeout
        let result = ctx
            .promise_with_timeout_raw(promise_name, std::time::Duration::from_millis(timeout_ms))
            .await;

        let mut output = DynamicOutput::new();
        output.insert(
            "promiseName".to_string(),
            Value::String(promise_name.to_string()),
        );
        match result {
            Ok(value) => {
                output.insert("resolved".to_string(), Value::Bool(true));
                output.insert("promiseValue".to_string(), value);
            }
            Err(e) => {
                output.insert("resolved".to_string(), Value::Bool(false));
                output.insert("error".to_string(), Value::String(e.to_string()));
            }
        }
        Ok(output)
    }
}

/// Workflow that schedules a child workflow.
pub struct ParentWorkflow;

#[async_trait]
impl DynamicWorkflow for ParentWorkflow {
    fn kind(&self) -> &str {
        "parent-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let child_input = input
            .get("childInput")
            .cloned()
            .unwrap_or(Value::Object(DynamicInput::new()));

        // Schedule a child workflow
        let child_result = ctx
            .schedule_workflow_raw("child-1", "child-workflow", child_input)
            .await?;

        let mut output = DynamicOutput::new();
        output.insert("childResult".to_string(), child_result);
        Ok(output)
    }
}

/// Simple child workflow that doubles a value.
pub struct ChildWorkflow;

#[async_trait]
impl DynamicWorkflow for ChildWorkflow {
    fn kind(&self) -> &str {
        "child-workflow"
    }

    async fn execute(
        &self,
        _ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let value = input.get("value").and_then(|v| v.as_i64()).unwrap_or(0);

        let mut output = DynamicOutput::new();
        output.insert("result".to_string(), Value::Number((value * 2).into()));
        Ok(output)
    }
}

/// Child workflow that always fails.
pub struct FailingChildWorkflow {
    pub message: String,
}

impl FailingChildWorkflow {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Default for FailingChildWorkflow {
    fn default() -> Self {
        Self::new("Child workflow intentional failure")
    }
}

#[async_trait]
impl DynamicWorkflow for FailingChildWorkflow {
    fn kind(&self) -> &str {
        "failing-child-workflow"
    }

    async fn execute(
        &self,
        _ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        Err(FlovynError::WorkflowFailed(self.message.clone()))
    }
}

/// Parent workflow that schedules a failing child.
pub struct ParentWithFailingChildWorkflow;

#[async_trait]
impl DynamicWorkflow for ParentWithFailingChildWorkflow {
    fn kind(&self) -> &str {
        "parent-failing-child-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        // Schedule a child workflow that will fail
        let result = ctx
            .schedule_workflow_raw(
                "failing-child-1",
                "failing-child-workflow",
                serde_json::json!({}),
            )
            .await;

        let mut output = DynamicOutput::new();
        match result {
            Ok(child_result) => {
                output.insert("childSucceeded".to_string(), Value::Bool(true));
                output.insert("childResult".to_string(), child_result);
            }
            Err(e) => {
                output.insert("childSucceeded".to_string(), Value::Bool(false));
                output.insert("childError".to_string(), Value::String(e.to_string()));
            }
        }
        Ok(output)
    }
}

/// Grandparent workflow that schedules a parent workflow.
pub struct GrandparentWorkflow;

#[async_trait]
impl DynamicWorkflow for GrandparentWorkflow {
    fn kind(&self) -> &str {
        "grandparent-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let value = input.get("value").and_then(|v| v.as_i64()).unwrap_or(10);

        // Schedule parent workflow which will schedule child workflow
        let parent_result = ctx
            .schedule_workflow_raw(
                "parent-1",
                "parent-workflow",
                serde_json::json!({ "childInput": { "value": value } }),
            )
            .await?;

        let mut output = DynamicOutput::new();
        output.insert("parentResult".to_string(), parent_result);
        Ok(output)
    }
}

/// Comprehensive workflow that tests multiple SDK features in a single execution.
/// Tests: basic execution, state set/get, operation recording (ctx.run_raw).
pub struct ComprehensiveWorkflow;

#[async_trait]
impl DynamicWorkflow for ComprehensiveWorkflow {
    fn kind(&self) -> &str {
        "comprehensive-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let mut output = DynamicOutput::new();
        let mut tests_passed = Vec::new();

        // Test 1: Basic input processing
        let value = input.get("value").and_then(|v| v.as_i64()).unwrap_or(10);
        output.insert("inputValue".to_string(), Value::Number(value.into()));
        tests_passed.push("basic_input");

        // Test 2: Operation recording with ctx.run_raw()
        let doubled = Value::Number((value * 2).into());
        let run_result = ctx.run_raw("double-operation", doubled.clone()).await?;
        output.insert("runResult".to_string(), run_result);
        tests_passed.push("run_operation");

        // Test 3: State set
        let state_key = "test-state-key";
        let state_value = serde_json::json!({
            "counter": value,
            "message": "state test",
            "nested": {"a": 1, "b": 2}
        });
        ctx.set_raw(state_key, state_value.clone()).await?;
        output.insert("stateSet".to_string(), Value::Bool(true));
        tests_passed.push("state_set");

        // Test 4: State get (should return what we just set)
        let retrieved = ctx.get_raw(state_key).await?;
        output.insert(
            "stateRetrieved".to_string(),
            retrieved.clone().unwrap_or(Value::Null),
        );

        // Verify state matches
        let state_matches = retrieved.as_ref() == Some(&state_value);
        output.insert("stateMatches".to_string(), Value::Bool(state_matches));
        if state_matches {
            tests_passed.push("state_get");
        }

        // Test 5: Multiple operations to test replay
        let tripled = Value::Number((value * 3).into());
        let triple_result = ctx.run_raw("triple-operation", tripled).await?;
        output.insert("tripleResult".to_string(), triple_result);
        tests_passed.push("multiple_operations");

        // Summary
        output.insert(
            "testsPassedCount".to_string(),
            Value::Number(tests_passed.len().into()),
        );
        output.insert(
            "testsPassed".to_string(),
            Value::Array(
                tests_passed
                    .into_iter()
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            ),
        );

        Ok(output)
    }
}

/// Comprehensive workflow that also tests task scheduling.
/// This workflow schedules a task and waits for its result.
pub struct ComprehensiveWithTaskWorkflow;

#[async_trait]
impl DynamicWorkflow for ComprehensiveWithTaskWorkflow {
    fn kind(&self) -> &str {
        "comprehensive-with-task-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let mut output = DynamicOutput::new();
        let mut tests_passed = Vec::new();

        // Test 1: Basic input
        let value = input.get("value").and_then(|v| v.as_i64()).unwrap_or(10);
        output.insert("inputValue".to_string(), Value::Number(value.into()));
        tests_passed.push("basic_input");

        // Test 2: State operations
        ctx.set_raw("workflow-state", serde_json::json!({"step": 1}))
            .await?;
        tests_passed.push("state_set");

        // Test 3: Operation recording
        let op_result = ctx
            .run_raw("compute", Value::Number((value * 2).into()))
            .await?;
        output.insert("opResult".to_string(), op_result);
        tests_passed.push("operation");

        // Test 4: Task scheduling
        let task_input = serde_json::json!({
            "message": format!("Task for value {}", value),
            "number": value
        });
        let task_result = ctx.schedule_raw("echo-task", task_input).await?;
        output.insert("taskResult".to_string(), task_result);
        tests_passed.push("task_scheduling");

        // Test 5: Verify state persists
        let state = ctx.get_raw("workflow-state").await?;
        output.insert("stateAfterTask".to_string(), state.unwrap_or(Value::Null));
        tests_passed.push("state_persistence");

        output.insert(
            "testsPassedCount".to_string(),
            Value::Number(tests_passed.len().into()),
        );
        output.insert(
            "testsPassed".to_string(),
            Value::Array(
                tests_passed
                    .into_iter()
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            ),
        );

        Ok(output)
    }
}

// ============================================================================
// Replay Test Fixtures
// ============================================================================
// Workflows designed for testing sequence-based replay and determinism validation.

/// Workflow that schedules N tasks of the same type in a loop.
/// Tests sequence-based replay with multiple tasks of the same type.
pub struct TaskLoopWorkflow;

#[async_trait]
impl DynamicWorkflow for TaskLoopWorkflow {
    fn kind(&self) -> &str {
        "task-loop-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let items: Vec<String> = input
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    "item-1".to_string(),
                    "item-2".to_string(),
                    "item-3".to_string(),
                ]
            });

        let mut results = Vec::new();

        // Schedule a task for each item
        for item in &items {
            let task_result = ctx
                .schedule_raw("process-item", serde_json::json!({ "item": item }))
                .await?;
            let processed = task_result
                .get("processed")
                .and_then(|v| v.as_str())
                .unwrap_or(item)
                .to_string();
            results.push(processed);
        }

        let mut output = DynamicOutput::new();
        output.insert(
            "results".to_string(),
            Value::Array(results.into_iter().map(Value::String).collect()),
        );
        Ok(output)
    }
}

/// Workflow that schedules N child workflows of the same type in a loop.
/// Tests sequence-based replay with multiple child workflows.
pub struct ChildWorkflowLoopWorkflow;

#[async_trait]
impl DynamicWorkflow for ChildWorkflowLoopWorkflow {
    fn kind(&self) -> &str {
        "child-workflow-loop-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let count = input.get("count").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

        let mut results = Vec::new();

        // Schedule a child workflow for each index
        for i in 1..=count {
            let child_name = format!("child-{}", i);
            let child_result = ctx
                .schedule_workflow_raw(
                    &child_name,
                    "process-child",
                    serde_json::json!({ "index": i }),
                )
                .await?;
            let processed = child_result
                .get("processed")
                .and_then(|v| v.as_u64())
                .unwrap_or(i as u64);
            results.push(processed as i64);
        }

        let mut output = DynamicOutput::new();
        output.insert(
            "results".to_string(),
            Value::Array(
                results
                    .into_iter()
                    .map(|n| Value::Number(n.into()))
                    .collect(),
            ),
        );
        Ok(output)
    }
}

/// Child workflow that processes an index.
pub struct ProcessChildWorkflow;

#[async_trait]
impl DynamicWorkflow for ProcessChildWorkflow {
    fn kind(&self) -> &str {
        "process-child"
    }

    async fn execute(
        &self,
        _ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let index = input.get("index").and_then(|v| v.as_u64()).unwrap_or(0);

        let mut output = DynamicOutput::new();
        output.insert("processed".to_string(), Value::Number(index.into()));
        Ok(output)
    }
}

/// Workflow that mixes tasks, timers, and child workflows.
/// Tests per-type sequence matching with interleaved command types.
pub struct MixedCommandWorkflow;

#[async_trait]
impl DynamicWorkflow for MixedCommandWorkflow {
    fn kind(&self) -> &str {
        "mixed-commands-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let mut output = DynamicOutput::new();

        // Task 1: Fetch data
        let fetch_result = ctx
            .schedule_raw("fetch-data", serde_json::json!({ "source": "api" }))
            .await?;
        output.insert("fetchResult".to_string(), fetch_result);

        // Timer: Wait 100ms
        ctx.sleep(std::time::Duration::from_millis(100)).await?;

        // Child workflow: Process data
        let process_result = ctx
            .schedule_workflow_raw(
                "process-data",
                "processor",
                serde_json::json!({ "data": "fetched" }),
            )
            .await?;
        output.insert("processResult".to_string(), process_result);

        // Task 2: Send notification
        let notify_result = ctx
            .schedule_raw(
                "send-notification",
                serde_json::json!({ "message": "done" }),
            )
            .await?;
        output.insert("notifyResult".to_string(), notify_result);

        output.insert("success".to_string(), Value::Bool(true));
        Ok(output)
    }
}

/// Workflow that schedules different task types.
/// Original version: schedules task-A then task-B.
pub struct OriginalTaskOrderWorkflow;

#[async_trait]
impl DynamicWorkflow for OriginalTaskOrderWorkflow {
    fn kind(&self) -> &str {
        "task-order-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        // Original order: A then B
        let result_a = ctx
            .schedule_raw("task-A", serde_json::json!({ "order": 1 }))
            .await?;

        let result_b = ctx
            .schedule_raw("task-B", serde_json::json!({ "order": 2 }))
            .await?;

        let mut output = DynamicOutput::new();
        output.insert("resultA".to_string(), result_a);
        output.insert("resultB".to_string(), result_b);
        Ok(output)
    }
}

/// Changed workflow that swaps task order (for determinism violation testing).
/// Changed version: schedules task-B then task-A (violates determinism).
pub struct ChangedTaskOrderWorkflow;

#[async_trait]
impl DynamicWorkflow for ChangedTaskOrderWorkflow {
    fn kind(&self) -> &str {
        "task-order-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        // CHANGED order: B then A (violates determinism)
        let result_b = ctx
            .schedule_raw("task-B", serde_json::json!({ "order": 1 }))
            .await?;

        let result_a = ctx
            .schedule_raw("task-A", serde_json::json!({ "order": 2 }))
            .await?;

        let mut output = DynamicOutput::new();
        output.insert("resultB".to_string(), result_b);
        output.insert("resultA".to_string(), result_a);
        Ok(output)
    }
}

// ============================================================================
// Parallel Execution Test Fixtures
// ============================================================================
// Workflows designed for testing parallel execution patterns.

use flovyn_worker_sdk::workflow::combinators::join_all;

/// Helper to add prefix to a kind name
fn prefixed(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}:{}", prefix, name)
    }
}

/// Workflow that schedules multiple tasks in parallel using join_all.
pub struct ParallelTasksWorkflow {
    prefix: String,
    kind: String,
}

impl ParallelTasksWorkflow {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            kind: prefixed(prefix, "parallel-tasks-workflow"),
        }
    }
}

impl Default for ParallelTasksWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicWorkflow for ParallelTasksWorkflow {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let items: Vec<String> = input
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["item-1".into(), "item-2".into(), "item-3".into()]);

        let task_futures: Vec<_> = items
            .iter()
            .map(|item| {
                ctx.schedule_raw(
                    &prefixed(&self.prefix, "process-item"),
                    serde_json::json!({ "item": item }),
                )
            })
            .collect();

        let results = join_all(task_futures).await?;

        let processed: Vec<Value> = results
            .into_iter()
            .map(|r| r.get("processed").cloned().unwrap_or(Value::Null))
            .collect();

        let mut output = DynamicOutput::new();
        output.insert("itemCount".to_string(), Value::Number(items.len().into()));
        output.insert("results".to_string(), Value::Array(processed));
        Ok(output)
    }
}

/// Workflow that races two tasks - first one to complete wins.
pub struct RacingTasksWorkflow {
    prefix: String,
    kind: String,
}

impl RacingTasksWorkflow {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            kind: prefixed(prefix, "racing-tasks-workflow"),
        }
    }
}

impl Default for RacingTasksWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicWorkflow for RacingTasksWorkflow {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        use flovyn_worker_sdk::workflow::combinators::select;

        let primary_url = input
            .get("primaryUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("http://primary.example.com");
        let fallback_url = input
            .get("fallbackUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("http://fallback.example.com");

        let primary = ctx.schedule_raw(
            &prefixed(&self.prefix, "fetch-data"),
            serde_json::json!({ "url": primary_url, "name": "primary" }),
        );
        let fallback = ctx.schedule_raw(
            &prefixed(&self.prefix, "fetch-data"),
            serde_json::json!({ "url": fallback_url, "name": "fallback" }),
        );

        let (winner_index, winner_result) = select(vec![primary, fallback]).await?;

        let mut output = DynamicOutput::new();
        output.insert(
            "winnerIndex".to_string(),
            Value::Number(winner_index.into()),
        );
        output.insert("winner".to_string(), winner_result);
        Ok(output)
    }
}

/// Workflow that adds timeout protection to a slow task.
pub struct TimeoutTaskWorkflow {
    prefix: String,
    kind: String,
}

impl TimeoutTaskWorkflow {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            kind: prefixed(prefix, "timeout-task-workflow"),
        }
    }
}

impl Default for TimeoutTaskWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicWorkflow for TimeoutTaskWorkflow {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        use flovyn_worker_sdk::workflow::combinators::with_timeout;
        use std::time::Duration;

        let timeout_ms = input
            .get("timeoutMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000);
        let operation = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("slow-op");

        let slow_task = ctx.schedule_raw(
            &prefixed(&self.prefix, "slow-operation"),
            serde_json::json!({ "op": operation }),
        );
        let result = with_timeout(slow_task, ctx.sleep(Duration::from_millis(timeout_ms))).await;

        let mut output = DynamicOutput::new();
        match result {
            Ok(task_result) => {
                output.insert("completed".to_string(), Value::Bool(true));
                output.insert("result".to_string(), task_result);
            }
            Err(FlovynError::Timeout(msg)) => {
                output.insert("completed".to_string(), Value::Bool(false));
                output.insert("error".to_string(), Value::String(msg));
            }
            Err(e) => return Err(e),
        }
        Ok(output)
    }
}

/// Workflow that demonstrates fan-out/fan-in with result aggregation.
pub struct FanOutFanInWorkflow {
    prefix: String,
    kind: String,
}

impl FanOutFanInWorkflow {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            kind: prefixed(prefix, "fan-out-fan-in-workflow"),
        }
    }
}

impl Default for FanOutFanInWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicWorkflow for FanOutFanInWorkflow {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let items: Vec<String> = input
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    "apple".into(),
                    "banana".into(),
                    "cherry".into(),
                    "date".into(),
                ]
            });

        let item_count = items.len();

        let task_futures: Vec<_> = items
            .iter()
            .map(|item| {
                ctx.schedule_raw(
                    &prefixed(&self.prefix, "process-item"),
                    serde_json::json!({ "item": item }),
                )
            })
            .collect();

        let results = join_all(task_futures).await?;

        let processed_items: Vec<String> = results
            .into_iter()
            .filter_map(|r| {
                r.get("processed")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        let mut output = DynamicOutput::new();
        output.insert("inputCount".to_string(), Value::Number(item_count.into()));
        output.insert(
            "outputCount".to_string(),
            Value::Number(processed_items.len().into()),
        );
        output.insert(
            "processedItems".to_string(),
            Value::Array(processed_items.into_iter().map(Value::String).collect()),
        );
        Ok(output)
    }
}

/// Workflow that combines parallel tasks with timers.
pub struct MixedParallelWorkflow {
    prefix: String,
    kind: String,
}

impl MixedParallelWorkflow {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            kind: prefixed(prefix, "mixed-parallel-workflow"),
        }
    }
}

impl Default for MixedParallelWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicWorkflow for MixedParallelWorkflow {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        use std::time::Duration;

        let mut output = DynamicOutput::new();

        // Phase 1: Two parallel tasks
        let task1 = ctx.schedule_raw(
            &prefixed(&self.prefix, "process-item"),
            serde_json::json!({ "item": "phase1-a" }),
        );
        let task2 = ctx.schedule_raw(
            &prefixed(&self.prefix, "process-item"),
            serde_json::json!({ "item": "phase1-b" }),
        );
        let phase1_results = join_all(vec![task1, task2]).await?;
        output.insert(
            "phase1".to_string(),
            Value::Array(phase1_results.into_iter().collect()),
        );

        // Phase 2: Timer + task
        ctx.sleep(Duration::from_millis(100)).await?;
        let task3_result = ctx
            .schedule_raw(
                &prefixed(&self.prefix, "slow-operation"),
                serde_json::json!({ "op": "fast" }),
            )
            .await?;
        output.insert("timerFired".to_string(), Value::Bool(true));
        output.insert("phase2TaskResult".to_string(), task3_result);

        // Phase 3: Three parallel tasks
        let task4 = ctx.schedule_raw(
            &prefixed(&self.prefix, "process-item"),
            serde_json::json!({ "item": "phase3-a" }),
        );
        let task5 = ctx.schedule_raw(
            &prefixed(&self.prefix, "process-item"),
            serde_json::json!({ "item": "phase3-b" }),
        );
        let task6 = ctx.schedule_raw(
            &prefixed(&self.prefix, "process-item"),
            serde_json::json!({ "item": "phase3-c" }),
        );
        let phase3_results = join_all(vec![task4, task5, task6]).await?;
        output.insert(
            "phase3".to_string(),
            Value::Array(phase3_results.into_iter().collect()),
        );

        output.insert("success".to_string(), Value::Bool(true));
        Ok(output)
    }
}

// ============================================================================
// Streaming Workflow Fixtures
// ============================================================================

/// Workflow that schedules a streaming token task.
pub struct StreamingTokenWorkflow;

#[async_trait]
impl DynamicWorkflow for StreamingTokenWorkflow {
    fn kind(&self) -> &str {
        "streaming-token-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let task_input = input
            .get("taskInput")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let task_result = ctx.schedule_raw("streaming-token-task", task_input).await?;

        let mut output = DynamicOutput::new();
        output.insert("taskResult".to_string(), task_result);
        Ok(output)
    }
}

/// Workflow that schedules a streaming progress task.
pub struct StreamingProgressWorkflow;

#[async_trait]
impl DynamicWorkflow for StreamingProgressWorkflow {
    fn kind(&self) -> &str {
        "streaming-progress-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let task_input = input
            .get("taskInput")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let task_result = ctx
            .schedule_raw("streaming-progress-task", task_input)
            .await?;

        let mut output = DynamicOutput::new();
        output.insert("taskResult".to_string(), task_result);
        Ok(output)
    }
}

/// Workflow that schedules a streaming data task.
pub struct StreamingDataWorkflow;

#[async_trait]
impl DynamicWorkflow for StreamingDataWorkflow {
    fn kind(&self) -> &str {
        "streaming-data-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let task_input = input
            .get("taskInput")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let task_result = ctx.schedule_raw("streaming-data-task", task_input).await?;

        let mut output = DynamicOutput::new();
        output.insert("taskResult".to_string(), task_result);
        Ok(output)
    }
}

/// Workflow that schedules a streaming error task.
pub struct StreamingErrorWorkflow;

#[async_trait]
impl DynamicWorkflow for StreamingErrorWorkflow {
    fn kind(&self) -> &str {
        "streaming-error-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let task_input = input
            .get("taskInput")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let task_result = ctx.schedule_raw("streaming-error-task", task_input).await?;

        let mut output = DynamicOutput::new();
        output.insert("taskResult".to_string(), task_result);
        Ok(output)
    }
}

/// Workflow that schedules a task that streams all event types.
pub struct StreamingAllTypesWorkflow;

#[async_trait]
impl DynamicWorkflow for StreamingAllTypesWorkflow {
    fn kind(&self) -> &str {
        "streaming-all-types-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: DynamicInput,
    ) -> Result<DynamicOutput> {
        let task_result = ctx
            .schedule_raw("streaming-all-types-task", serde_json::json!({}))
            .await?;

        let mut output = DynamicOutput::new();
        output.insert("taskResult".to_string(), task_result);
        Ok(output)
    }
}
