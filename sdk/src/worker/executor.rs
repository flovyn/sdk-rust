//! WorkflowExecutor - Executes workflows with deterministic replay

use crate::error::{FlovynError, Result};
use crate::worker::determinism::DeterminismValidator;
use crate::workflow::command::WorkflowCommand;
use crate::workflow::context_impl::WorkflowContextImpl;
use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::recorder::{CommandCollector, ValidatingCommandRecorder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::sync::Arc;
use uuid::Uuid;

/// Status of a workflow execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowStatus {
    /// Workflow is currently running
    Running,
    /// Workflow completed successfully
    Completed,
    /// Workflow is suspended waiting for external event
    Suspended,
    /// Workflow failed with an error
    Failed,
    /// Workflow was cancelled
    Cancelled,
}

/// Result of a workflow task execution
#[derive(Debug, Clone)]
pub struct WorkflowTaskResult {
    /// Commands generated during execution
    pub commands: Vec<WorkflowCommand>,
    /// Final status of the workflow
    pub status: WorkflowStatus,
    /// Output value if completed
    pub output: Option<Value>,
    /// Error message if failed
    pub error: Option<String>,
    /// Stack trace if failed
    pub stack_trace: Option<String>,
    /// Failure type if failed
    pub failure_type: Option<String>,
}

impl WorkflowTaskResult {
    /// Create a completed result
    pub fn completed(commands: Vec<WorkflowCommand>, output: Value) -> Self {
        Self {
            commands,
            status: WorkflowStatus::Completed,
            output: Some(output),
            error: None,
            stack_trace: None,
            failure_type: None,
        }
    }

    /// Create a suspended result
    pub fn suspended(commands: Vec<WorkflowCommand>) -> Self {
        Self {
            commands,
            status: WorkflowStatus::Suspended,
            output: None,
            error: None,
            stack_trace: None,
            failure_type: None,
        }
    }

    /// Create a failed result
    pub fn failed(
        commands: Vec<WorkflowCommand>,
        error: String,
        stack_trace: Option<String>,
        failure_type: &str,
    ) -> Self {
        Self {
            commands,
            status: WorkflowStatus::Failed,
            output: None,
            error: Some(error),
            stack_trace,
            failure_type: Some(failure_type.to_string()),
        }
    }

    /// Create a cancelled result
    pub fn cancelled(commands: Vec<WorkflowCommand>) -> Self {
        Self {
            commands,
            status: WorkflowStatus::Cancelled,
            output: None,
            error: None,
            stack_trace: None,
            failure_type: None,
        }
    }
}

/// Configuration for workflow execution
#[derive(Debug, Clone)]
pub struct WorkflowExecutorConfig {
    /// Whether to enable determinism validation during replay
    pub enable_determinism_validation: bool,
    /// Current workflow task timestamp
    pub workflow_task_time: i64,
}

impl Default for WorkflowExecutorConfig {
    fn default() -> Self {
        Self {
            enable_determinism_validation: true,
            workflow_task_time: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// WorkflowExecutor executes workflow definitions with deterministic replay
pub struct WorkflowExecutor {
    workflow_execution_id: Uuid,
    org_id: Uuid,
    input: Value,
    existing_events: Vec<ReplayEvent>,
    config: WorkflowExecutorConfig,
}

impl WorkflowExecutor {
    /// Create a new workflow executor
    pub fn new(
        workflow_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        existing_events: Vec<ReplayEvent>,
    ) -> Self {
        Self {
            workflow_execution_id,
            org_id,
            input,
            existing_events,
            config: WorkflowExecutorConfig::default(),
        }
    }

    /// Create a new workflow executor with custom configuration
    pub fn with_config(
        workflow_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        existing_events: Vec<ReplayEvent>,
        config: WorkflowExecutorConfig,
    ) -> Self {
        Self {
            workflow_execution_id,
            org_id,
            input,
            existing_events,
            config,
        }
    }

    /// Check if events already contain a specific terminal event type
    fn has_terminal_event(&self, event_type: EventType) -> bool {
        self.existing_events
            .iter()
            .any(|e| e.event_type() == event_type)
    }

    /// Get the last event in the sequence
    fn last_event(&self) -> Option<&ReplayEvent> {
        self.existing_events.last()
    }

    /// Execute a workflow with a simple command collector (no validation)
    pub async fn execute<F, Fut>(&self, workflow_fn: F) -> WorkflowTaskResult
    where
        F: FnOnce(Arc<WorkflowContextImpl<CommandCollector>>, Value) -> Fut,
        Fut: Future<Output = Result<Value>>,
    {
        let recorder = CommandCollector::new();
        let mut current_sequence = self.existing_events.len() as i32;

        let ctx = Arc::new(WorkflowContextImpl::new(
            self.workflow_execution_id,
            self.org_id,
            self.input.clone(),
            recorder,
            self.existing_events.clone(),
            self.config.workflow_task_time,
        ));

        let result = workflow_fn(Arc::clone(&ctx), self.input.clone()).await;
        self.handle_result(ctx, result, &mut current_sequence)
    }

    /// Execute a workflow with determinism validation
    pub async fn execute_with_validation<F, Fut>(&self, workflow_fn: F) -> WorkflowTaskResult
    where
        F: FnOnce(Arc<WorkflowContextImpl<ValidatingCommandRecorder>>, Value) -> Fut,
        Fut: Future<Output = Result<Value>>,
    {
        let recorder =
            if self.config.enable_determinism_validation && !self.existing_events.is_empty() {
                ValidatingCommandRecorder::new(
                    DeterminismValidator::new(),
                    self.existing_events.clone(),
                )
            } else {
                ValidatingCommandRecorder::new(DeterminismValidator::new(), vec![])
            };

        let mut current_sequence = self.existing_events.len() as i32;

        let ctx = Arc::new(WorkflowContextImpl::new(
            self.workflow_execution_id,
            self.org_id,
            self.input.clone(),
            recorder,
            self.existing_events.clone(),
            self.config.workflow_task_time,
        ));

        let result = workflow_fn(Arc::clone(&ctx), self.input.clone()).await;
        self.handle_result(ctx, result, &mut current_sequence)
    }

    fn handle_result<R: crate::workflow::recorder::CommandRecorder>(
        &self,
        ctx: Arc<WorkflowContextImpl<R>>,
        result: Result<Value>,
        current_sequence: &mut i32,
    ) -> WorkflowTaskResult {
        let mut commands = ctx.get_commands();

        match result {
            Ok(output) => {
                // Idempotency: Only add completion command if not already completed
                if !self.has_terminal_event(EventType::WorkflowCompleted) {
                    *current_sequence += 1;
                    commands.push(WorkflowCommand::CompleteWorkflow {
                        sequence_number: *current_sequence,
                        output: output.clone(),
                    });
                }

                WorkflowTaskResult::completed(commands, output)
            }

            Err(FlovynError::Suspended { reason }) => {
                // Idempotency: Only add suspension command if not already suspended
                let last_is_suspended = self
                    .last_event()
                    .map(|e| e.event_type() == EventType::WorkflowSuspended)
                    .unwrap_or(false);

                if !last_is_suspended {
                    *current_sequence += 1;
                    commands.push(WorkflowCommand::SuspendWorkflow {
                        sequence_number: *current_sequence,
                        reason,
                    });
                }

                WorkflowTaskResult::suspended(commands)
            }

            Err(FlovynError::WorkflowCancelled(reason)) => {
                *current_sequence += 1;
                commands.push(WorkflowCommand::CancelWorkflow {
                    sequence_number: *current_sequence,
                    reason,
                });

                WorkflowTaskResult::cancelled(commands)
            }

            Err(FlovynError::DeterminismViolation(violation)) => {
                *current_sequence += 1;
                let error_msg = violation.to_string();
                commands.push(WorkflowCommand::FailWorkflow {
                    sequence_number: *current_sequence,
                    error: error_msg.clone(),
                    stack_trace: String::new(),
                    failure_type: Some("DETERMINISM_VIOLATION".to_string()),
                });

                WorkflowTaskResult::failed(commands, error_msg, None, "DETERMINISM_VIOLATION")
            }

            Err(e) => {
                *current_sequence += 1;
                let error_msg = e.to_string();
                let failure_type = classify_error(&e);
                commands.push(WorkflowCommand::FailWorkflow {
                    sequence_number: *current_sequence,
                    error: error_msg.clone(),
                    stack_trace: String::new(),
                    failure_type: Some(failure_type.to_string()),
                });

                WorkflowTaskResult::failed(commands, error_msg, None, failure_type)
            }
        }
    }
}

/// Classify an error to determine retry behavior
fn classify_error(error: &FlovynError) -> &'static str {
    match error {
        FlovynError::NonRetryable(_) => "NON_RETRYABLE",
        FlovynError::InvalidConfiguration(_) => "NON_RETRYABLE",
        FlovynError::WorkflowNotFound(_) => "NON_RETRYABLE",
        FlovynError::TaskNotFound(_) => "NON_RETRYABLE",
        FlovynError::DeterminismViolation(_) => "DETERMINISM_VIOLATION",
        FlovynError::Grpc(_) => "TRANSIENT",
        FlovynError::Io(_) => "TRANSIENT",
        FlovynError::Serialization(_) => "TRANSIENT",
        FlovynError::TimerError(_) => "TRANSIENT",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn test_workflow_status_serialize() {
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::Running).unwrap(),
            "\"RUNNING\""
        );
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::Completed).unwrap(),
            "\"COMPLETED\""
        );
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::Suspended).unwrap(),
            "\"SUSPENDED\""
        );
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::Failed).unwrap(),
            "\"FAILED\""
        );
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::Cancelled).unwrap(),
            "\"CANCELLED\""
        );
    }

    #[test]
    fn test_workflow_task_result_completed() {
        let result = WorkflowTaskResult::completed(vec![], json!({"result": 42}));
        assert_eq!(result.status, WorkflowStatus::Completed);
        assert_eq!(result.output, Some(json!({"result": 42})));
        assert!(result.error.is_none());
    }

    #[test]
    fn test_workflow_task_result_suspended() {
        let result = WorkflowTaskResult::suspended(vec![]);
        assert_eq!(result.status, WorkflowStatus::Suspended);
        assert!(result.output.is_none());
        assert!(result.error.is_none());
    }

    #[test]
    fn test_workflow_task_result_failed() {
        let result = WorkflowTaskResult::failed(
            vec![],
            "Something went wrong".to_string(),
            Some("at main.rs:10".to_string()),
            "TRANSIENT",
        );
        assert_eq!(result.status, WorkflowStatus::Failed);
        assert!(result.output.is_none());
        assert_eq!(result.error, Some("Something went wrong".to_string()));
        assert_eq!(result.stack_trace, Some("at main.rs:10".to_string()));
        assert_eq!(result.failure_type, Some("TRANSIENT".to_string()));
    }

    #[test]
    fn test_workflow_task_result_cancelled() {
        let result = WorkflowTaskResult::cancelled(vec![]);
        assert_eq!(result.status, WorkflowStatus::Cancelled);
        assert!(result.output.is_none());
    }

    #[test]
    fn test_workflow_executor_config_default() {
        let config = WorkflowExecutorConfig::default();
        assert!(config.enable_determinism_validation);
        assert!(config.workflow_task_time > 0);
    }

    #[test]
    fn test_classify_error_non_retryable() {
        assert_eq!(
            classify_error(&FlovynError::NonRetryable("error".to_string())),
            "NON_RETRYABLE"
        );
        assert_eq!(
            classify_error(&FlovynError::InvalidConfiguration("error".to_string())),
            "NON_RETRYABLE"
        );
        assert_eq!(
            classify_error(&FlovynError::WorkflowNotFound("wf".to_string())),
            "NON_RETRYABLE"
        );
    }

    #[test]
    fn test_classify_error_transient() {
        assert_eq!(
            classify_error(&FlovynError::TimerError("error".to_string())),
            "TRANSIENT"
        );
    }

    #[test]
    fn test_classify_error_determinism_violation() {
        assert_eq!(
            classify_error(&FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::TypeMismatch {
                    sequence: 1,
                    expected: EventType::OperationCompleted,
                    actual: EventType::TaskScheduled,
                }
            )),
            "DETERMINISM_VIOLATION"
        );
    }

    #[test]
    fn test_workflow_executor_new() {
        let executor = WorkflowExecutor::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            json!({"input": "test"}),
            vec![],
        );

        assert!(executor.config.enable_determinism_validation);
    }

    #[test]
    fn test_workflow_executor_has_terminal_event() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::WorkflowCompleted,
            json!({}),
            now(),
        )];

        let executor = WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({}), events);

        assert!(executor.has_terminal_event(EventType::WorkflowCompleted));
        assert!(!executor.has_terminal_event(EventType::WorkflowExecutionFailed));
    }

    #[test]
    fn test_workflow_executor_last_event() {
        let events = vec![
            ReplayEvent::new(1, EventType::WorkflowStarted, json!({}), now()),
            ReplayEvent::new(2, EventType::OperationCompleted, json!({}), now()),
        ];

        let executor = WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({}), events);

        let last = executor.last_event().unwrap();
        assert_eq!(last.sequence_number(), 2);
        assert_eq!(last.event_type(), EventType::OperationCompleted);
    }

    #[tokio::test]
    async fn test_execute_success() {
        let executor =
            WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({"x": 5}), vec![]);

        let result = executor
            .execute(|_ctx, input| async move {
                let x = input.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
                Ok(json!({"result": x * 2}))
            })
            .await;

        assert_eq!(result.status, WorkflowStatus::Completed);
        assert_eq!(result.output, Some(json!({"result": 10})));
        assert!(!result.commands.is_empty());

        // Should have CompleteWorkflow command
        let has_complete = result
            .commands
            .iter()
            .any(|c| matches!(c, WorkflowCommand::CompleteWorkflow { .. }));
        assert!(has_complete);
    }

    #[tokio::test]
    async fn test_execute_suspension() {
        let executor = WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({}), vec![]);

        let result = executor
            .execute(|_ctx, _input| async move {
                Err(FlovynError::Suspended {
                    reason: "Waiting for task".to_string(),
                })
            })
            .await;

        assert_eq!(result.status, WorkflowStatus::Suspended);
        assert!(result.output.is_none());

        // Should have SuspendWorkflow command
        let has_suspend = result
            .commands
            .iter()
            .any(|c| matches!(c, WorkflowCommand::SuspendWorkflow { .. }));
        assert!(has_suspend);
    }

    #[tokio::test]
    async fn test_execute_failure() {
        let executor = WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({}), vec![]);

        let result = executor
            .execute(|_ctx, _input| async move {
                Err(FlovynError::Other("Something went wrong".to_string()))
            })
            .await;

        assert_eq!(result.status, WorkflowStatus::Failed);
        assert!(result.error.is_some());
        assert!(result
            .error
            .as_ref()
            .unwrap()
            .contains("Something went wrong"));

        // Should have FailWorkflow command
        let has_fail = result
            .commands
            .iter()
            .any(|c| matches!(c, WorkflowCommand::FailWorkflow { .. }));
        assert!(has_fail);
    }

    #[tokio::test]
    async fn test_execute_cancellation() {
        let executor = WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({}), vec![]);

        let result = executor
            .execute(|_ctx, _input| async move {
                Err(FlovynError::WorkflowCancelled(
                    "User requested cancellation".to_string(),
                ))
            })
            .await;

        assert_eq!(result.status, WorkflowStatus::Cancelled);

        // Should have CancelWorkflow command
        let has_cancel = result
            .commands
            .iter()
            .any(|c| matches!(c, WorkflowCommand::CancelWorkflow { .. }));
        assert!(has_cancel);
    }

    #[tokio::test]
    async fn test_execute_idempotent_completion() {
        // If workflow already completed, don't add another CompleteWorkflow command
        let events = vec![ReplayEvent::new(
            1,
            EventType::WorkflowCompleted,
            json!({}),
            now(),
        )];

        let executor = WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({}), events);

        let result = executor
            .execute(|_ctx, _input| async move { Ok(json!({"result": "done"})) })
            .await;

        // Should not have CompleteWorkflow command (idempotency)
        let complete_count = result
            .commands
            .iter()
            .filter(|c| matches!(c, WorkflowCommand::CompleteWorkflow { .. }))
            .count();
        assert_eq!(complete_count, 0);
    }

    #[tokio::test]
    async fn test_execute_idempotent_suspension() {
        // If last event is suspension, don't add another SuspendWorkflow command
        let events = vec![ReplayEvent::new(
            1,
            EventType::WorkflowSuspended,
            json!({}),
            now(),
        )];

        let executor = WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({}), events);

        let result = executor
            .execute(|_ctx, _input| async move {
                Err(FlovynError::Suspended {
                    reason: "Waiting".to_string(),
                })
            })
            .await;

        // Should not have SuspendWorkflow command (idempotency)
        let suspend_count = result
            .commands
            .iter()
            .filter(|c| matches!(c, WorkflowCommand::SuspendWorkflow { .. }))
            .count();
        assert_eq!(suspend_count, 0);
    }

    #[tokio::test]
    async fn test_execute_with_validation() {
        let executor =
            WorkflowExecutor::new(Uuid::new_v4(), Uuid::new_v4(), json!({"x": 10}), vec![]);

        let result = executor
            .execute_with_validation(|_ctx, input| async move {
                let x = input.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
                Ok(json!({"result": x + 1}))
            })
            .await;

        assert_eq!(result.status, WorkflowStatus::Completed);
        assert_eq!(result.output, Some(json!({"result": 11})));
    }
}
