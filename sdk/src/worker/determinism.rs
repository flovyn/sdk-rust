//! Determinism validation for workflow replay

use crate::error::DeterminismViolationError;
use crate::workflow::command::WorkflowCommand;
use crate::workflow::event::{EventType, ReplayEvent};

/// Result of determinism validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterminismValidationResult {
    /// Command matches historical event - determinism maintained
    Valid,

    /// New command added beyond historical events - this is valid (workflow extended)
    NewCommand { message: String },

    /// Command type doesn't match event type - DETERMINISM VIOLATION
    TypeMismatch {
        sequence: i32,
        expected: EventType,
        actual: EventType,
    },

    /// Operation name doesn't match - DETERMINISM VIOLATION
    OperationNameMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// Task type doesn't match - DETERMINISM VIOLATION
    TaskTypeMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// Promise name doesn't match - DETERMINISM VIOLATION
    PromiseNameMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// Timer ID doesn't match - DETERMINISM VIOLATION
    TimerIdMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// State key doesn't match - DETERMINISM VIOLATION
    StateKeyMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// Child workflow mismatch - DETERMINISM VIOLATION
    ChildWorkflowMismatch {
        sequence: i32,
        field: String,
        expected: String,
        actual: String,
    },
}

impl DeterminismValidationResult {
    /// Check if this result represents a violation
    pub fn is_violation(&self) -> bool {
        !matches!(self, Self::Valid | Self::NewCommand { .. })
    }

    /// Get error message if this is a violation
    pub fn error_message(&self) -> Option<String> {
        match self {
            Self::Valid | Self::NewCommand { .. } => None,
            Self::TypeMismatch {
                sequence,
                expected,
                actual,
            } => Some(format!(
                "Type mismatch at sequence {}: expected {:?}, got {:?}",
                sequence, expected, actual
            )),
            Self::OperationNameMismatch {
                sequence,
                expected,
                actual,
            } => Some(format!(
                "Operation name mismatch at sequence {}: expected '{}', got '{}'",
                sequence, expected, actual
            )),
            Self::TaskTypeMismatch {
                sequence,
                expected,
                actual,
            } => Some(format!(
                "Task type mismatch at sequence {}: expected '{}', got '{}'",
                sequence, expected, actual
            )),
            Self::PromiseNameMismatch {
                sequence,
                expected,
                actual,
            } => Some(format!(
                "Promise name mismatch at sequence {}: expected '{}', got '{}'",
                sequence, expected, actual
            )),
            Self::TimerIdMismatch {
                sequence,
                expected,
                actual,
            } => Some(format!(
                "Timer ID mismatch at sequence {}: expected '{}', got '{}'",
                sequence, expected, actual
            )),
            Self::StateKeyMismatch {
                sequence,
                expected,
                actual,
            } => Some(format!(
                "State key mismatch at sequence {}: expected '{}', got '{}'",
                sequence, expected, actual
            )),
            Self::ChildWorkflowMismatch {
                sequence,
                field,
                expected,
                actual,
            } => Some(format!(
                "Child workflow {} mismatch at sequence {}: expected '{}', got '{}'",
                field, sequence, expected, actual
            )),
        }
    }

    /// Convert to DeterminismViolationError if this is a violation
    pub fn into_error(self) -> Option<DeterminismViolationError> {
        match self {
            Self::Valid | Self::NewCommand { .. } => None,
            Self::TypeMismatch {
                sequence,
                expected,
                actual,
            } => Some(DeterminismViolationError::TypeMismatch {
                sequence,
                expected,
                actual,
            }),
            Self::OperationNameMismatch {
                sequence,
                expected,
                actual,
            } => Some(DeterminismViolationError::OperationNameMismatch {
                sequence,
                expected,
                actual,
            }),
            Self::TaskTypeMismatch {
                sequence,
                expected,
                actual,
            } => Some(DeterminismViolationError::TaskTypeMismatch {
                sequence,
                expected,
                actual,
            }),
            Self::PromiseNameMismatch {
                sequence,
                expected,
                actual,
            } => Some(DeterminismViolationError::PromiseNameMismatch {
                sequence,
                expected,
                actual,
            }),
            Self::TimerIdMismatch {
                sequence,
                expected,
                actual,
            } => Some(DeterminismViolationError::TimerIdMismatch {
                sequence,
                expected,
                actual,
            }),
            Self::StateKeyMismatch {
                sequence,
                expected,
                actual,
            } => Some(DeterminismViolationError::StateKeyMismatch {
                sequence,
                expected,
                actual,
            }),
            Self::ChildWorkflowMismatch {
                sequence,
                field,
                expected,
                actual,
            } => Some(DeterminismViolationError::ChildWorkflowMismatch {
                sequence,
                field,
                expected,
                actual,
            }),
        }
    }
}

/// Validates workflow execution determinism by comparing commands to historical events.
///
/// During replay, new commands generated by workflow code must match historical events
/// in the same order. This validator detects violations:
/// - Command sequence mismatch (operation added/removed/reordered)
/// - Command type mismatch (run → schedule, etc.)
/// - Operation name mismatch (for RecordOperation commands)
/// - Task type mismatch (for ScheduleTask commands)
///
/// Violations indicate non-deterministic code changes and should fail permanently.
#[derive(Debug, Clone, Default)]
pub struct DeterminismValidator;

impl DeterminismValidator {
    /// Create a new determinism validator
    pub fn new() -> Self {
        Self
    }

    /// Validate that a command matches the expected historical event.
    ///
    /// # Arguments
    /// * `command` - The command generated during current execution
    /// * `event` - The historical event from previous execution (None if no event at this sequence)
    ///
    /// # Returns
    /// * `Ok(())` if validation passes
    /// * `Err(DeterminismViolationError)` if a violation is detected
    pub fn validate_command(
        &self,
        command: &WorkflowCommand,
        event: Option<&ReplayEvent>,
    ) -> Result<(), DeterminismViolationError> {
        let result = self.validate_command_result(command, event);
        match result.into_error() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Validate a command and return a detailed result
    pub fn validate_command_result(
        &self,
        command: &WorkflowCommand,
        event: Option<&ReplayEvent>,
    ) -> DeterminismValidationResult {
        let sequence = command.sequence_number();

        // If no event exists at this sequence, it means we added a new command
        // This is only valid if it comes AFTER all historical events
        let Some(event) = event else {
            return DeterminismValidationResult::NewCommand {
                message: format!(
                    "New command at sequence {} (beyond historical events)",
                    sequence
                ),
            };
        };

        // Validate command type matches event type
        let expected_event_type = self.command_to_event_type(command);
        if expected_event_type != event.event_type() {
            return DeterminismValidationResult::TypeMismatch {
                sequence,
                expected: event.event_type(),
                actual: expected_event_type,
            };
        }

        // Type-specific validation
        match command {
            WorkflowCommand::RecordOperation { operation_name, .. } => {
                if let Some(historical_name) = event.get_string("operationName") {
                    if historical_name != operation_name {
                        return DeterminismValidationResult::OperationNameMismatch {
                            sequence,
                            expected: historical_name.to_string(),
                            actual: operation_name.clone(),
                        };
                    }
                }
            }
            WorkflowCommand::ScheduleTask { task_type, .. } => {
                if let Some(historical_type) = event.get_string("taskType") {
                    if historical_type != task_type {
                        return DeterminismValidationResult::TaskTypeMismatch {
                            sequence,
                            expected: historical_type.to_string(),
                            actual: task_type.clone(),
                        };
                    }
                }
            }
            WorkflowCommand::SetState { key, .. } | WorkflowCommand::ClearState { key, .. } => {
                if let Some(historical_key) = event.get_string("key") {
                    if historical_key != key {
                        return DeterminismValidationResult::StateKeyMismatch {
                            sequence,
                            expected: historical_key.to_string(),
                            actual: key.clone(),
                        };
                    }
                }
            }
            WorkflowCommand::CreatePromise { promise_id, .. }
            | WorkflowCommand::ResolvePromise { promise_id, .. } => {
                if let Some(historical_id) = event.get_string("promiseId") {
                    if historical_id != promise_id {
                        return DeterminismValidationResult::PromiseNameMismatch {
                            sequence,
                            expected: historical_id.to_string(),
                            actual: promise_id.clone(),
                        };
                    }
                }
            }
            WorkflowCommand::StartTimer { timer_id, .. }
            | WorkflowCommand::CancelTimer { timer_id, .. } => {
                if let Some(historical_id) = event.get_string("timerId") {
                    if historical_id != timer_id {
                        return DeterminismValidationResult::TimerIdMismatch {
                            sequence,
                            expected: historical_id.to_string(),
                            actual: timer_id.clone(),
                        };
                    }
                }
            }
            WorkflowCommand::ScheduleChildWorkflow { name, kind, .. } => {
                if let Some(historical_name) = event.get_string("name") {
                    if historical_name != name {
                        return DeterminismValidationResult::ChildWorkflowMismatch {
                            sequence,
                            field: "name".to_string(),
                            expected: historical_name.to_string(),
                            actual: name.clone(),
                        };
                    }
                }
                if let Some(historical_kind) = event.get_string("kind") {
                    if let Some(cmd_kind) = kind {
                        if historical_kind != cmd_kind {
                            return DeterminismValidationResult::ChildWorkflowMismatch {
                                sequence,
                                field: "kind".to_string(),
                                expected: historical_kind.to_string(),
                                actual: cmd_kind.clone(),
                            };
                        }
                    }
                }
            }
            // Terminal commands don't need detailed field validation
            WorkflowCommand::CompleteWorkflow { .. }
            | WorkflowCommand::FailWorkflow { .. }
            | WorkflowCommand::SuspendWorkflow { .. }
            | WorkflowCommand::CancelWorkflow { .. } => {}
        }

        DeterminismValidationResult::Valid
    }

    /// Maps WorkflowCommand to corresponding EventType.
    pub fn command_to_event_type(&self, command: &WorkflowCommand) -> EventType {
        match command {
            WorkflowCommand::RecordOperation { .. } => EventType::OperationCompleted,
            WorkflowCommand::SetState { .. } => EventType::StateSet,
            WorkflowCommand::ClearState { .. } => EventType::StateCleared,
            WorkflowCommand::ScheduleTask { .. } => EventType::TaskScheduled,
            WorkflowCommand::ScheduleChildWorkflow { .. } => EventType::ChildWorkflowInitiated,
            WorkflowCommand::CompleteWorkflow { .. } => EventType::WorkflowCompleted,
            WorkflowCommand::FailWorkflow { .. } => EventType::WorkflowExecutionFailed,
            WorkflowCommand::SuspendWorkflow { .. } => EventType::WorkflowSuspended,
            WorkflowCommand::CancelWorkflow { .. } => EventType::CancellationRequested,
            WorkflowCommand::CreatePromise { .. } => EventType::PromiseCreated,
            WorkflowCommand::ResolvePromise { .. } => EventType::PromiseResolved,
            WorkflowCommand::StartTimer { .. } => EventType::TimerStarted,
            WorkflowCommand::CancelTimer { .. } => EventType::TimerCancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    // ==================== DeterminismValidationResult Tests ====================

    #[test]
    fn test_validation_result_is_violation() {
        assert!(!DeterminismValidationResult::Valid.is_violation());
        assert!(!DeterminismValidationResult::NewCommand {
            message: "test".to_string()
        }
        .is_violation());

        assert!(DeterminismValidationResult::TypeMismatch {
            sequence: 1,
            expected: EventType::OperationCompleted,
            actual: EventType::TaskScheduled,
        }
        .is_violation());

        assert!(DeterminismValidationResult::OperationNameMismatch {
            sequence: 1,
            expected: "a".to_string(),
            actual: "b".to_string(),
        }
        .is_violation());
    }

    #[test]
    fn test_validation_result_error_message() {
        assert!(DeterminismValidationResult::Valid.error_message().is_none());

        let result = DeterminismValidationResult::TypeMismatch {
            sequence: 5,
            expected: EventType::OperationCompleted,
            actual: EventType::TaskScheduled,
        };
        let msg = result.error_message().unwrap();
        assert!(msg.contains("Type mismatch at sequence 5"));
    }

    // ==================== DeterminismValidator Tests ====================

    #[test]
    fn test_valid_matching_command_and_event() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: "fetch-user".to_string(),
            result: json!({"id": 1}),
        };
        let event = ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            json!({"operationName": "fetch-user", "result": {"id": 1}}),
            now(),
        );

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_ok());
    }

    #[test]
    fn test_type_mismatch_violation() {
        let validator = DeterminismValidator::new();
        // Command is RecordOperation, but event is TaskScheduled
        let command = WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: "op".to_string(),
            result: json!({}),
        };
        let event = ReplayEvent::new(
            1,
            EventType::TaskScheduled,
            json!({"taskType": "some-task"}),
            now(),
        );

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::TypeMismatch { .. }
        ));
    }

    #[test]
    fn test_operation_name_mismatch_violation() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: "new-operation".to_string(),
            result: json!({}),
        };
        let event = ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            json!({"operationName": "old-operation"}),
            now(),
        );

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(
            err,
            DeterminismViolationError::OperationNameMismatch {
                expected,
                actual,
                ..
            } if expected == "old-operation" && actual == "new-operation"
        ));
    }

    #[test]
    fn test_task_type_mismatch_violation() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::ScheduleTask {
            sequence_number: 1,
            task_type: "payment-task".to_string(),
            task_execution_id: Uuid::new_v4(),
            input: json!({}),
            priority_seconds: None,
        };
        let event = ReplayEvent::new(
            1,
            EventType::TaskScheduled,
            json!({"taskType": "shipping-task"}),
            now(),
        );

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::TaskTypeMismatch { .. }
        ));
    }

    #[test]
    fn test_new_command_beyond_history_is_valid() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::RecordOperation {
            sequence_number: 5,
            operation_name: "new-op".to_string(),
            result: json!({}),
        };

        // No event at this sequence - new command, valid
        let result = validator.validate_command(&command, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_state_key_mismatch_violation() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::SetState {
            sequence_number: 1,
            key: "new-key".to_string(),
            value: json!({}),
        };
        let event = ReplayEvent::new(1, EventType::StateSet, json!({"key": "old-key"}), now());

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::StateKeyMismatch { .. }
        ));
    }

    #[test]
    fn test_promise_name_mismatch_violation() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::CreatePromise {
            sequence_number: 1,
            promise_id: "new-promise".to_string(),
            timeout_ms: None,
        };
        let event = ReplayEvent::new(
            1,
            EventType::PromiseCreated,
            json!({"promiseId": "old-promise"}),
            now(),
        );

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::PromiseNameMismatch { .. }
        ));
    }

    #[test]
    fn test_timer_id_mismatch_violation() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::StartTimer {
            sequence_number: 1,
            timer_id: "timer-2".to_string(),
            duration_ms: 1000,
        };
        let event = ReplayEvent::new(
            1,
            EventType::TimerStarted,
            json!({"timerId": "timer-1"}),
            now(),
        );

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::TimerIdMismatch { .. }
        ));
    }

    #[test]
    fn test_child_workflow_name_mismatch_violation() {
        let validator = DeterminismValidator::new();
        let command = WorkflowCommand::ScheduleChildWorkflow {
            sequence_number: 1,
            name: "new-workflow".to_string(),
            kind: Some("workflow-kind".to_string()),
            definition_id: None,
            child_execution_id: Uuid::new_v4(),
            input: json!({}),
            task_queue: "default".to_string(),
            priority_seconds: 0,
        };
        let event = ReplayEvent::new(
            1,
            EventType::ChildWorkflowInitiated,
            json!({"name": "old-workflow", "kind": "workflow-kind"}),
            now(),
        );

        let result = validator.validate_command(&command, Some(&event));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::ChildWorkflowMismatch { field, .. } if field == "name"
        ));
    }

    // ==================== Command to Event Type Mapping Tests ====================

    #[test]
    fn test_command_to_event_type_mapping() {
        let validator = DeterminismValidator::new();

        // Test all command types
        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::RecordOperation {
                sequence_number: 0,
                operation_name: "".to_string(),
                result: json!(null),
            }),
            EventType::OperationCompleted
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::SetState {
                sequence_number: 0,
                key: "".to_string(),
                value: json!(null),
            }),
            EventType::StateSet
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::ClearState {
                sequence_number: 0,
                key: "".to_string(),
            }),
            EventType::StateCleared
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::ScheduleTask {
                sequence_number: 0,
                task_type: "".to_string(),
                task_execution_id: Uuid::nil(),
                input: json!(null),
                priority_seconds: None,
            }),
            EventType::TaskScheduled
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::ScheduleChildWorkflow {
                sequence_number: 0,
                name: "".to_string(),
                kind: None,
                definition_id: None,
                child_execution_id: Uuid::nil(),
                input: json!(null),
                task_queue: "".to_string(),
                priority_seconds: 0,
            }),
            EventType::ChildWorkflowInitiated
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::CompleteWorkflow {
                sequence_number: 0,
                output: json!(null),
            }),
            EventType::WorkflowCompleted
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::FailWorkflow {
                sequence_number: 0,
                error: "".to_string(),
                stack_trace: "".to_string(),
                failure_type: None,
            }),
            EventType::WorkflowExecutionFailed
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::SuspendWorkflow {
                sequence_number: 0,
                reason: "".to_string(),
            }),
            EventType::WorkflowSuspended
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::CancelWorkflow {
                sequence_number: 0,
                reason: "".to_string(),
            }),
            EventType::CancellationRequested
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::CreatePromise {
                sequence_number: 0,
                promise_id: "".to_string(),
                timeout_ms: None,
            }),
            EventType::PromiseCreated
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::ResolvePromise {
                sequence_number: 0,
                promise_id: "".to_string(),
                value: json!(null),
            }),
            EventType::PromiseResolved
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::StartTimer {
                sequence_number: 0,
                timer_id: "".to_string(),
                duration_ms: 0,
            }),
            EventType::TimerStarted
        );

        assert_eq!(
            validator.command_to_event_type(&WorkflowCommand::CancelTimer {
                sequence_number: 0,
                timer_id: "".to_string(),
            }),
            EventType::TimerCancelled
        );
    }

    // ==================== Integration Tests ====================

    #[test]
    fn test_validate_complete_workflow_sequence() {
        let validator = DeterminismValidator::new();

        // Simulate a workflow that:
        // 1. Does an operation
        // 2. Schedules a task
        // 3. Completes

        let events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                json!({"operationName": "validate-input", "result": true}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskScheduled,
                json!({"taskType": "process-data"}),
                now(),
            ),
            ReplayEvent::new(
                3,
                EventType::WorkflowCompleted,
                json!({"output": {"success": true}}),
                now(),
            ),
        ];

        let commands = vec![
            WorkflowCommand::RecordOperation {
                sequence_number: 1,
                operation_name: "validate-input".to_string(),
                result: json!(true),
            },
            WorkflowCommand::ScheduleTask {
                sequence_number: 2,
                task_type: "process-data".to_string(),
                task_execution_id: Uuid::new_v4(),
                input: json!({}),
                priority_seconds: None,
            },
            WorkflowCommand::CompleteWorkflow {
                sequence_number: 3,
                output: json!({"success": true}),
            },
        ];

        // All commands should validate successfully
        for (cmd, event) in commands.iter().zip(events.iter()) {
            let result = validator.validate_command(cmd, Some(event));
            assert!(
                result.is_ok(),
                "Command {:?} should validate",
                cmd.type_name()
            );
        }
    }

    #[test]
    fn test_detect_added_operation() {
        let validator = DeterminismValidator::new();

        // Historical: only one operation
        let events = vec![ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            json!({"operationName": "step-1"}),
            now(),
        )];

        // New code: two operations, then tries to add a third one that doesn't exist
        let cmd1 = WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: "step-1".to_string(),
            result: json!({}),
        };
        let cmd2 = WorkflowCommand::RecordOperation {
            sequence_number: 2,
            operation_name: "step-2".to_string(),
            result: json!({}),
        };

        // First command matches
        assert!(validator.validate_command(&cmd1, Some(&events[0])).is_ok());

        // Second command is new (no event) - this is OK
        assert!(validator.validate_command(&cmd2, None).is_ok());
    }

    #[test]
    fn test_detect_removed_operation() {
        let validator = DeterminismValidator::new();

        // Historical: two operations
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                json!({"operationName": "step-1"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::OperationCompleted,
                json!({"operationName": "step-2"}),
                now(),
            ),
        ];

        // New code: tries to skip step-1 and go directly to completion
        // This would be a type mismatch (RecordOperation vs expected OperationCompleted at seq 1)
        let cmd = WorkflowCommand::CompleteWorkflow {
            sequence_number: 1,
            output: json!({}),
        };

        let result = validator.validate_command(&cmd, Some(&events[0]));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::TypeMismatch { .. }
        ));
    }
}
