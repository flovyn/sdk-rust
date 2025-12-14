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
        ctx.sleep(std::time::Duration::from_millis(sleep_ms)).await?;

        let end_time = ctx.current_time_millis();

        let mut output = DynamicOutput::new();
        output.insert(
            "sleptMs".to_string(),
            Value::Number(sleep_ms.into()),
        );
        output.insert(
            "elapsedMs".to_string(),
            Value::Number((end_time - start_time).into()),
        );
        Ok(output)
    }
}
