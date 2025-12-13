//! Test task definitions for E2E tests

#![allow(dead_code)] // Fixtures will be used when tests are implemented

use async_trait::async_trait;
use flovyn_sdk::error::{FlovynError, Result};
use flovyn_sdk::task::context::TaskContext;
use flovyn_sdk::task::definition::{DynamicTask, DynamicTaskInput, DynamicTaskOutput};

/// Task that echoes its input.
pub struct EchoTask;

#[async_trait]
impl DynamicTask for EchoTask {
    fn kind(&self) -> &str {
        "echo-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        Ok(input)
    }
}

/// Task that sleeps for a configurable duration.
pub struct SlowTask;

#[async_trait]
impl DynamicTask for SlowTask {
    fn kind(&self) -> &str {
        "slow-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let sleep_ms = input
            .get("sleepMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "sleptMs".to_string(),
            serde_json::Value::Number(sleep_ms.into()),
        );
        Ok(output)
    }
}

/// Task that fails a configurable number of times before succeeding.
pub struct FailingTask {
    attempts: std::sync::atomic::AtomicU32,
    fail_count: u32,
}

impl FailingTask {
    pub fn new(fail_count: u32) -> Self {
        Self {
            attempts: std::sync::atomic::AtomicU32::new(0),
            fail_count,
        }
    }
}

impl Default for FailingTask {
    fn default() -> Self {
        Self::new(2) // Fail twice, succeed on third attempt
    }
}

#[async_trait]
impl DynamicTask for FailingTask {
    fn kind(&self) -> &str {
        "failing-task"
    }

    async fn execute(
        &self,
        _input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let attempt = self
            .attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if attempt < self.fail_count {
            return Err(FlovynError::Other(format!(
                "Intentional failure (attempt {})",
                attempt + 1
            )));
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "attempts".to_string(),
            serde_json::Value::Number((attempt + 1).into()),
        );
        Ok(output)
    }
}

/// Task that reports progress.
pub struct ProgressTask;

#[async_trait]
impl DynamicTask for ProgressTask {
    fn kind(&self) -> &str {
        "progress-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let steps = input.get("steps").and_then(|v| v.as_u64()).unwrap_or(5) as u32;

        for i in 0..steps {
            let progress = (i + 1) as f64 / steps as f64;
            let message = format!("Step {}/{}", i + 1, steps);
            ctx.report_progress(progress, Some(&message)).await?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "completedSteps".to_string(),
            serde_json::Value::Number(steps.into()),
        );
        Ok(output)
    }
}
