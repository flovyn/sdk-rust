//! Property-based tests for state machine validation
//!
//! These tests use proptest to verify state machine invariants

use chrono::Utc;
use flovyn_sdk::prelude::*;
use proptest::prelude::*;
use uuid::Uuid;

/// Generate an arbitrary WorkflowCommand for property-based testing
fn arb_command() -> impl Strategy<Value = WorkflowCommand> {
    prop_oneof![
        // RecordOperation
        (
            "[a-z]{3,10}",
            any::<i64>().prop_map(|n| serde_json::json!(n))
        )
            .prop_map(|(op_name, result)| WorkflowCommand::RecordOperation {
                sequence_number: 1,
                operation_name: op_name,
                result,
            }),
        // SetState
        (
            "[a-z]{3,10}",
            any::<i64>().prop_map(|n| serde_json::json!(n))
        )
            .prop_map(|(key, value)| WorkflowCommand::SetState {
                sequence_number: 1,
                key,
                value,
            }),
        // ClearState
        "[a-z]{3,10}".prop_map(|key| WorkflowCommand::ClearState {
            sequence_number: 1,
            key,
        }),
        // ScheduleTask
        (
            "[a-z]{3,10}",
            any::<i64>().prop_map(|n| serde_json::json!(n))
        )
            .prop_map(|(kind, input)| WorkflowCommand::ScheduleTask {
                sequence_number: 1,
                kind,
                task_execution_id: Uuid::new_v4(),
                input,
                priority_seconds: None,
                max_retries: None,
                timeout_ms: None,
                queue: None,
            }),
        // CreatePromise
        "[a-z]{3,10}".prop_map(|promise_id| WorkflowCommand::CreatePromise {
            sequence_number: 1,
            promise_id,
            timeout_ms: None,
        }),
        // StartTimer
        ("[a-z]{3,10}", any::<i64>().prop_map(|n| n.abs())).prop_map(|(timer_id, duration_ms)| {
            WorkflowCommand::StartTimer {
                sequence_number: 1,
                timer_id,
                duration_ms,
            }
        }),
        // CompleteWorkflow
        any::<i64>()
            .prop_map(|n| serde_json::json!(n))
            .prop_map(|output| WorkflowCommand::CompleteWorkflow {
                sequence_number: 1,
                output,
            }),
        // FailWorkflow
        "[a-z]{3,20}".prop_map(|error| WorkflowCommand::FailWorkflow {
            sequence_number: 1,
            error,
            stack_trace: "at test".to_string(),
            failure_type: None,
        }),
    ]
}

/// Generate a matching ReplayEvent for a given WorkflowCommand
fn command_to_matching_event(cmd: &WorkflowCommand) -> ReplayEvent {
    let validator = DeterminismValidator::new();
    let event_type = validator.command_to_event_type(cmd);

    let mut event = ReplayEvent::new(
        cmd.sequence_number(),
        event_type,
        serde_json::json!({}),
        Utc::now(),
    );

    // Set relevant data based on command type
    match cmd {
        WorkflowCommand::RecordOperation { operation_name, .. } => {
            event = event.with_operation_name(operation_name.clone());
        }
        WorkflowCommand::SetState { key, value, .. } => {
            event = event.with_state_key(key.clone());
            event = event.with_result(value.clone());
        }
        WorkflowCommand::ClearState { key, .. } => {
            event = event.with_state_key(key.clone());
        }
        WorkflowCommand::ScheduleTask { kind, .. } => {
            event = event.with_task_kind(kind.clone());
        }
        WorkflowCommand::CreatePromise { promise_id, .. } => {
            event = event.with_promise_name(promise_id.clone());
        }
        WorkflowCommand::StartTimer { timer_id, .. } => {
            event = event.with_timer_id(timer_id.clone());
        }
        WorkflowCommand::CompleteWorkflow { output, .. } => {
            event = event.with_result(output.clone());
        }
        WorkflowCommand::FailWorkflow { error, .. } => {
            event = event.with_error(error.clone());
        }
        _ => {}
    }

    event
}

proptest! {
    /// Property: A command and its matching event should always validate successfully
    #[test]
    fn command_with_matching_event_validates(cmd in arb_command()) {
        let event = command_to_matching_event(&cmd);
        let validator = DeterminismValidator::new();

        let result = validator.validate_command(&cmd, Some(&event));
        prop_assert!(
            result.is_ok(),
            "Command {:?} should validate with matching event, but got: {:?}",
            cmd,
            result
        );
    }

    /// Property: A sequence of commands produces matching event types
    #[test]
    fn command_sequence_produces_matching_events(commands in prop::collection::vec(arb_command(), 1..10)) {
        let validator = DeterminismValidator::new();

        for cmd in &commands {
            let expected_type = validator.command_to_event_type(cmd);
            let event = command_to_matching_event(cmd);

            prop_assert_eq!(
                event.event_type(),
                expected_type,
                "Event type mismatch for command {:?}",
                cmd
            );
        }
    }

    /// Property: Changing an operation name causes a determinism violation
    #[test]
    fn detects_operation_name_change(
        original_name in "[a-z]{3,10}",
        modified_name in "[a-z]{3,10}"
    ) {
        // Only test when names are actually different
        prop_assume!(original_name != modified_name);

        let original_cmd = WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: original_name.clone(),
            result: serde_json::json!(42),
        };

        let modified_cmd = WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: modified_name.clone(),
            result: serde_json::json!(42),
        };

        // Create event matching original command
        let event = command_to_matching_event(&original_cmd);

        // Try to validate modified command against original event
        let validator = DeterminismValidator::new();
        let result = validator.validate_command(&modified_cmd, Some(&event));

        prop_assert!(
            result.is_err(),
            "Should detect operation name change from '{}' to '{}'",
            original_name,
            modified_name
        );
    }

    /// Property: Changing a task type causes a determinism violation
    #[test]
    fn detects_task_type_change(
        original_type in "[a-z]{3,10}",
        modified_type in "[a-z]{3,10}"
    ) {
        prop_assume!(original_type != modified_type);

        let original_cmd = WorkflowCommand::ScheduleTask {
            sequence_number: 1,
            kind: original_type.clone(),
            task_execution_id: Uuid::new_v4(),
            input: serde_json::json!({}),
            priority_seconds: None,
            max_retries: None,
            timeout_ms: None,
            queue: None,
        };

        let modified_cmd = WorkflowCommand::ScheduleTask {
            sequence_number: 1,
            kind: modified_type.clone(),
            task_execution_id: Uuid::new_v4(),
            input: serde_json::json!({}),
            priority_seconds: None,
            max_retries: None,
            timeout_ms: None,
            queue: None,
        };

        let event = command_to_matching_event(&original_cmd);
        let validator = DeterminismValidator::new();
        let result = validator.validate_command(&modified_cmd, Some(&event));

        prop_assert!(
            result.is_err(),
            "Should detect task type change from '{}' to '{}'",
            original_type,
            modified_type
        );
    }

    /// Property: Changing a promise ID causes a determinism violation
    #[test]
    fn detects_promise_id_change(
        original_id in "[a-z]{3,10}",
        modified_id in "[a-z]{3,10}"
    ) {
        prop_assume!(original_id != modified_id);

        let original_cmd = WorkflowCommand::CreatePromise {
            sequence_number: 1,
            promise_id: original_id.clone(),
            timeout_ms: None,
        };

        let modified_cmd = WorkflowCommand::CreatePromise {
            sequence_number: 1,
            promise_id: modified_id.clone(),
            timeout_ms: None,
        };

        let event = command_to_matching_event(&original_cmd);
        let validator = DeterminismValidator::new();
        let result = validator.validate_command(&modified_cmd, Some(&event));

        prop_assert!(
            result.is_err(),
            "Should detect promise ID change from '{}' to '{}'",
            original_id,
            modified_id
        );
    }

    /// Property: Changing a state key causes a determinism violation
    #[test]
    fn detects_state_key_change(
        original_key in "[a-z]{3,10}",
        modified_key in "[a-z]{3,10}"
    ) {
        prop_assume!(original_key != modified_key);

        let original_cmd = WorkflowCommand::SetState {
            sequence_number: 1,
            key: original_key.clone(),
            value: serde_json::json!(42),
        };

        let modified_cmd = WorkflowCommand::SetState {
            sequence_number: 1,
            key: modified_key.clone(),
            value: serde_json::json!(42),
        };

        let event = command_to_matching_event(&original_cmd);
        let validator = DeterminismValidator::new();
        let result = validator.validate_command(&modified_cmd, Some(&event));

        prop_assert!(
            result.is_err(),
            "Should detect state key change from '{}' to '{}'",
            original_key,
            modified_key
        );
    }

    /// Property: Changing a timer ID causes a determinism violation
    #[test]
    fn detects_timer_id_change(
        original_id in "[a-z]{3,10}",
        modified_id in "[a-z]{3,10}"
    ) {
        prop_assume!(original_id != modified_id);

        let original_cmd = WorkflowCommand::StartTimer {
            sequence_number: 1,
            timer_id: original_id.clone(),
            duration_ms: 1000,
        };

        let modified_cmd = WorkflowCommand::StartTimer {
            sequence_number: 1,
            timer_id: modified_id.clone(),
            duration_ms: 1000,
        };

        let event = command_to_matching_event(&original_cmd);
        let validator = DeterminismValidator::new();
        let result = validator.validate_command(&modified_cmd, Some(&event));

        prop_assert!(
            result.is_err(),
            "Should detect timer id change from '{}' to '{}'",
            original_id,
            modified_id
        );
    }

    /// Property: New commands beyond history are always valid (fresh execution)
    #[test]
    fn new_commands_always_valid(cmd in arb_command()) {
        let validator = DeterminismValidator::new();
        let result = validator.validate_command(&cmd, None);

        prop_assert!(
            result.is_ok(),
            "New command without history should always be valid: {:?}, error: {:?}",
            cmd,
            result
        );
    }
}
