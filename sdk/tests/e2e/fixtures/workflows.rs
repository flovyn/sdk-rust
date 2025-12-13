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
