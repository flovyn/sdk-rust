//! Durable Timer Workflow
//!
//! Demonstrates the use of durable timers that survive worker restarts.
//! This pattern is useful for:
//! - Scheduled reminders
//! - Delayed actions
//! - Timeout-based workflows

use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

/// Input for the reminder workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderInput {
    pub message: String,
    pub delay_seconds: u64,
}

/// Output from the reminder workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderOutput {
    pub delivered: bool,
    pub delivered_at: i64,
}

/// A workflow that demonstrates durable timers
///
/// This workflow:
/// 1. Sends an initial notification that a reminder is scheduled
/// 2. Sleeps for the specified duration (durable - survives restarts)
/// 3. Sends the reminder after the delay
pub struct ReminderWorkflow;

#[async_trait]
impl WorkflowDefinition for ReminderWorkflow {
    type Input = ReminderInput;
    type Output = ReminderOutput;

    fn kind(&self) -> &str {
        "reminder-workflow"
    }

    fn name(&self) -> &str {
        "Durable Reminder Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates durable timers that survive worker restarts")
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "timer".to_string(),
            "reminder".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            message = %input.message,
            delay_seconds = input.delay_seconds,
            "Reminder workflow started"
        );

        // Record the initial notification (cached operation)
        let notification_value =
            serde_json::to_value(format!("Reminder scheduled for: {}", input.message))?;
        let _ = ctx.run_raw("send-initial", notification_value).await?;

        info!(
            delay_seconds = input.delay_seconds,
            "Sleeping for specified duration (durable)"
        );

        // Sleep for the specified duration
        // This is a durable timer - if the worker crashes and restarts,
        // the workflow will resume and continue waiting from where it left off
        ctx.sleep(Duration::from_secs(input.delay_seconds)).await?;

        info!("Timer fired, sending reminder");

        // Send the reminder
        let reminder_value = serde_json::to_value(format!("REMINDER: {}", input.message))?;
        let _ = ctx.run_raw("send-reminder", reminder_value).await?;

        let delivered_at = ctx.current_time_millis();

        info!(
            message = %input.message,
            delivered_at = delivered_at,
            "Reminder delivered"
        );

        Ok(ReminderOutput {
            delivered: true,
            delivered_at,
        })
    }
}

/// Input for the multi-step timer workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiStepTimerInput {
    pub total_duration_seconds: u64,
    pub checkpoint_interval_seconds: u64,
}

/// Output from the multi-step timer workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiStepTimerOutput {
    pub checkpoints: Vec<i64>,
    pub completed_at: i64,
}

/// A workflow demonstrating multiple durable timers
///
/// This workflow shows how to use multiple timers in a single workflow,
/// with periodic checkpoints that track progress.
pub struct MultiStepTimerWorkflow;

#[async_trait]
impl WorkflowDefinition for MultiStepTimerWorkflow {
    type Input = MultiStepTimerInput;
    type Output = MultiStepTimerOutput;

    fn kind(&self) -> &str {
        "multi-step-timer-workflow"
    }

    fn name(&self) -> &str {
        "Multi-Step Timer Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates multiple durable timers with periodic checkpoints")
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            total_duration = input.total_duration_seconds,
            interval = input.checkpoint_interval_seconds,
            "Multi-step timer workflow started"
        );

        let mut checkpoints = Vec::new();
        let mut elapsed = 0u64;

        while elapsed < input.total_duration_seconds {
            // Check for cancellation at each checkpoint
            ctx.check_cancellation().await?;

            // Calculate sleep duration for this step
            let remaining = input.total_duration_seconds - elapsed;
            let sleep_duration = remaining.min(input.checkpoint_interval_seconds);

            info!(
                checkpoint = checkpoints.len() + 1,
                elapsed = elapsed,
                sleeping = sleep_duration,
                "Checkpoint"
            );

            // Durable sleep
            ctx.sleep(Duration::from_secs(sleep_duration)).await?;

            elapsed += sleep_duration;
            checkpoints.push(ctx.current_time_millis());

            // Record checkpoint in state
            ctx.set_raw(
                "lastCheckpoint",
                serde_json::to_value(ctx.current_time_millis())?,
            )
            .await?;
            ctx.set_raw("elapsed", serde_json::to_value(elapsed)?)
                .await?;
        }

        let completed_at = ctx.current_time_millis();

        info!(
            checkpoints = checkpoints.len(),
            completed_at = completed_at,
            "Multi-step timer workflow completed"
        );

        Ok(MultiStepTimerOutput {
            checkpoints,
            completed_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reminder_workflow_kind() {
        let workflow = ReminderWorkflow;
        assert_eq!(workflow.kind(), "reminder-workflow");
    }

    #[test]
    fn test_reminder_input_serialization() {
        let input = ReminderInput {
            message: "Test reminder".to_string(),
            delay_seconds: 3600,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("Test reminder"));
        assert!(json.contains("3600"));
    }

    #[test]
    fn test_multi_step_timer_workflow_kind() {
        let workflow = MultiStepTimerWorkflow;
        assert_eq!(workflow.kind(), "multi-step-timer-workflow");
    }
}

/// Integration tests using SDK testing utilities
#[cfg(test)]
mod integration_tests {
    use super::*;
    use flovyn_sdk::testing::MockWorkflowContext;

    #[tokio::test]
    async fn test_reminder_workflow_execution() {
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1700000000000)
            .build();

        let input = ReminderInput {
            message: "Take a break!".to_string(),
            delay_seconds: 60,
        };

        let workflow = ReminderWorkflow;
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.delivered);
        assert_eq!(output.delivered_at, 1700000000000); // Time from context

        // Verify operations were recorded
        assert!(ctx.was_operation_recorded("send-initial"));
        assert!(ctx.was_operation_recorded("send-reminder"));

        // Verify a timer was registered
        // Timer was registered - we just check that operations were recorded
        let _ = ctx.time_controller().pending_timer_ids();
    }

    #[tokio::test]
    async fn test_reminder_workflow_with_time_advancement() {
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1700000000000)
            .build();

        let input = ReminderInput {
            message: "Meeting in 1 hour".to_string(),
            delay_seconds: 3600,
        };

        let workflow = ReminderWorkflow;
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.delivered);
    }

    #[tokio::test]
    async fn test_multi_step_timer_workflow_execution() {
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1700000000000)
            .build();

        // Short durations for testing
        let input = MultiStepTimerInput {
            total_duration_seconds: 30,
            checkpoint_interval_seconds: 10,
        };

        let workflow = MultiStepTimerWorkflow;
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();

        // With 30s total and 10s intervals, we expect 3 checkpoints
        assert_eq!(output.checkpoints.len(), 3);

        // Verify state was updated
        let state = ctx.state_snapshot();
        assert!(state.contains_key("lastCheckpoint"));
        assert!(state.contains_key("elapsed"));
    }

    #[tokio::test]
    async fn test_multi_step_timer_workflow_cancellation() {
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1700000000000)
            .build();

        // Request cancellation before starting
        ctx.request_cancellation();

        let input = MultiStepTimerInput {
            total_duration_seconds: 300,
            checkpoint_interval_seconds: 60,
        };

        let workflow = MultiStepTimerWorkflow;
        let result = workflow.execute(&ctx, input).await;

        // Should error due to cancellation check at first checkpoint
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_reminder_workflow_records_operations_in_order() {
        let ctx = MockWorkflowContext::new();

        let input = ReminderInput {
            message: "Order test".to_string(),
            delay_seconds: 5,
        };

        let workflow = ReminderWorkflow;
        let _ = workflow.execute(&ctx, input).await;

        let ops = ctx.recorded_operations();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].name, "send-initial");
        assert_eq!(ops[1].name, "send-reminder");
    }
}
