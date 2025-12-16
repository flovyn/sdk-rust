//! Test workflow definitions for E2E tests

#![allow(dead_code)] // Fixtures will be used when tests are implemented

use async_trait::async_trait;
use flovyn_sdk::error::{FlovynError, Result};
use flovyn_sdk::workflow::context::WorkflowContext;
use flovyn_sdk::workflow::definition::{DynamicInput, DynamicOutput, DynamicWorkflow};
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
