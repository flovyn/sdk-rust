//! Bridge between Stateright models and real ReplayEngine implementation.
//!
//! This module converts model states/actions to real ReplayEngine operations
//! and compares outcomes between the model and implementation.
//!
//! ## Purpose
//!
//! The model verifies specifications, but we need to ensure the real implementation
//! matches. This bridge:
//!
//! 1. Builds a real ReplayEngine from model configuration
//! 2. Executes commands on the real engine
//! 3. Compares outcomes: model says violation ↔ real engine detects violation
//!
//! ## Usage
//!
//! ```ignore
//! let model = SequenceMatchingModel { /* config */ };
//! let engine = build_engine_from_model(&model);
//!
//! for cmd in commands {
//!     let impl_outcome = execute_command(&engine, &cmd);
//!     // Compare with model's expected outcome
//! }
//! ```

use super::sequence_matching::{Command, SequenceMatchingModel, SequenceMatchingState};
use chrono::Utc;
use flovyn_core::workflow::event::{EventType, ReplayEvent};
use flovyn_core::workflow::ReplayEngine;
use serde_json::json;

/// Outcome of executing a command on the real ReplayEngine.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandOutcome {
    /// Command succeeded (matched replay history or extended beyond it)
    Success,
    /// Command caused a determinism violation (mismatch with replay history)
    Violation(String),
}

/// Build a real ReplayEngine from model configuration.
///
/// Converts the model's event lists into actual ReplayEvent objects
/// and creates a ReplayEngine initialized with them.
pub fn build_engine_from_model(model: &SequenceMatchingModel) -> ReplayEngine {
    let mut events = Vec::new();
    let mut seq = 1;

    // Add task events
    for task_type in &model.task_events {
        events.push(ReplayEvent::new(
            seq,
            EventType::TaskScheduled,
            json!({
                "taskType": task_type,
                "taskExecutionId": format!("task-exec-{}", seq),
            }),
            Utc::now(),
        ));
        seq += 1;
    }

    // Add timer events
    for timer_id in &model.timer_events {
        events.push(ReplayEvent::new(
            seq,
            EventType::TimerStarted,
            json!({
                "timerId": timer_id,
            }),
            Utc::now(),
        ));
        seq += 1;
    }

    // Add child workflow events
    for name in &model.child_events {
        events.push(ReplayEvent::new(
            seq,
            EventType::ChildWorkflowInitiated,
            json!({
                "childExecutionName": name,
            }),
            Utc::now(),
        ));
        seq += 1;
    }

    ReplayEngine::new(events)
}

/// Execute a model command on the real ReplayEngine.
///
/// This mirrors what the actual SDK does during replay:
/// 1. Get the next sequence number for this command type
/// 2. Look up the event at that sequence (if replaying)
/// 3. Validate that the command matches the event (if replaying)
/// 4. Return Success or Violation accordingly
pub fn execute_command(engine: &ReplayEngine, cmd: &Command) -> CommandOutcome {
    match cmd {
        Command::ScheduleTask { task_type } => {
            let seq = engine.next_task_seq();

            // Check if we're replaying (seq < event count)
            if let Some(event) = engine.get_task_event(seq) {
                let expected = event.get_string("taskType").unwrap_or("");
                if expected != task_type {
                    return CommandOutcome::Violation(format!(
                        "TaskTypeMismatch at Task({}): expected '{}', got '{}'",
                        seq, expected, task_type
                    ));
                }
            }
            // Beyond history = extension (allowed)
            CommandOutcome::Success
        }
        Command::StartTimer { timer_id } => {
            let seq = engine.next_timer_seq();

            if let Some(event) = engine.get_timer_event(seq) {
                let expected = event.get_string("timerId").unwrap_or("");
                if expected != timer_id {
                    return CommandOutcome::Violation(format!(
                        "TimerIdMismatch at Timer({}): expected '{}', got '{}'",
                        seq, expected, timer_id
                    ));
                }
            }
            CommandOutcome::Success
        }
        Command::ScheduleChild { name } => {
            let seq = engine.next_child_workflow_seq();

            if let Some(event) = engine.get_child_workflow_event(seq) {
                let expected = event.get_string("childExecutionName").unwrap_or("");
                if expected != name {
                    return CommandOutcome::Violation(format!(
                        "ChildNameMismatch at Child({}): expected '{}', got '{}'",
                        seq, expected, name
                    ));
                }
            }
            CommandOutcome::Success
        }
    }
}

/// Execute a sequence of commands on the engine and return all outcomes.
pub fn execute_commands(engine: &ReplayEngine, commands: &[Command]) -> Vec<CommandOutcome> {
    commands
        .iter()
        .map(|cmd| execute_command(engine, cmd))
        .collect()
}

/// Check if a model state has a violation.
#[allow(dead_code)]
pub fn model_has_violation(state: &SequenceMatchingState) -> bool {
    state.violation.is_some()
}

/// Get the violation message from a model state.
#[allow(dead_code)]
pub fn model_violation_message(state: &SequenceMatchingState) -> Option<&str> {
    state.violation.as_deref()
}

/// Check if any command outcome is a violation.
#[allow(dead_code)]
pub fn outcomes_have_violation(outcomes: &[CommandOutcome]) -> bool {
    outcomes
        .iter()
        .any(|o| matches!(o, CommandOutcome::Violation(_)))
}

/// Get the first violation message from outcomes.
#[allow(dead_code)]
pub fn first_violation_message(outcomes: &[CommandOutcome]) -> Option<&str> {
    outcomes.iter().find_map(|o| match o {
        CommandOutcome::Violation(msg) => Some(msg.as_str()),
        CommandOutcome::Success => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model() -> SequenceMatchingModel {
        SequenceMatchingModel {
            task_events: vec!["task-A".into(), "task-B".into()],
            timer_events: vec!["timer-1".into()],
            child_events: vec!["child-1".into()],
            possible_task_types: vec!["task-A".into(), "task-B".into(), "task-C".into()],
            possible_timer_ids: vec!["timer-1".into(), "timer-2".into()],
            possible_child_names: vec!["child-1".into(), "child-2".into()],
            max_commands: 4,
        }
    }

    #[test]
    fn test_build_engine_from_model() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        // Verify event counts match model configuration
        assert_eq!(engine.task_event_count(), 2);
        assert_eq!(engine.timer_event_count(), 1);
        assert_eq!(engine.child_workflow_event_count(), 1);

        // Verify event content
        assert_eq!(
            engine.get_task_event(0).unwrap().get_string("taskType"),
            Some("task-A")
        );
        assert_eq!(
            engine.get_task_event(1).unwrap().get_string("taskType"),
            Some("task-B")
        );
        assert_eq!(
            engine.get_timer_event(0).unwrap().get_string("timerId"),
            Some("timer-1")
        );
        assert_eq!(
            engine
                .get_child_workflow_event(0)
                .unwrap()
                .get_string("childExecutionName"),
            Some("child-1")
        );
    }

    #[test]
    fn test_execute_command_matching_task_succeeds() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        let cmd = Command::ScheduleTask {
            task_type: "task-A".into(),
        };
        let outcome = execute_command(&engine, &cmd);

        assert_eq!(outcome, CommandOutcome::Success);
    }

    #[test]
    fn test_execute_command_mismatched_task_fails() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        let cmd = Command::ScheduleTask {
            task_type: "task-WRONG".into(),
        };
        let outcome = execute_command(&engine, &cmd);

        assert!(matches!(outcome, CommandOutcome::Violation(_)));
        if let CommandOutcome::Violation(msg) = outcome {
            assert!(msg.contains("TaskTypeMismatch"));
            assert!(msg.contains("task-A"));
            assert!(msg.contains("task-WRONG"));
        }
    }

    #[test]
    fn test_execute_command_matching_timer_succeeds() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        let cmd = Command::StartTimer {
            timer_id: "timer-1".into(),
        };
        let outcome = execute_command(&engine, &cmd);

        assert_eq!(outcome, CommandOutcome::Success);
    }

    #[test]
    fn test_execute_command_mismatched_timer_fails() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        let cmd = Command::StartTimer {
            timer_id: "timer-WRONG".into(),
        };
        let outcome = execute_command(&engine, &cmd);

        assert!(matches!(outcome, CommandOutcome::Violation(_)));
    }

    #[test]
    fn test_execute_command_matching_child_succeeds() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        let cmd = Command::ScheduleChild {
            name: "child-1".into(),
        };
        let outcome = execute_command(&engine, &cmd);

        assert_eq!(outcome, CommandOutcome::Success);
    }

    #[test]
    fn test_execute_command_mismatched_child_fails() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        let cmd = Command::ScheduleChild {
            name: "child-WRONG".into(),
        };
        let outcome = execute_command(&engine, &cmd);

        assert!(matches!(outcome, CommandOutcome::Violation(_)));
    }

    #[test]
    fn test_execute_command_extension_succeeds() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        // Execute all matching commands first
        execute_command(
            &engine,
            &Command::ScheduleTask {
                task_type: "task-A".into(),
            },
        );
        execute_command(
            &engine,
            &Command::ScheduleTask {
                task_type: "task-B".into(),
            },
        );

        // Now extend beyond history - should succeed
        let cmd = Command::ScheduleTask {
            task_type: "task-NEW".into(),
        };
        let outcome = execute_command(&engine, &cmd);

        assert_eq!(outcome, CommandOutcome::Success);
    }

    #[test]
    fn test_per_type_sequences_independent() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        // Execute timer first (doesn't affect task sequence)
        let outcome1 = execute_command(
            &engine,
            &Command::StartTimer {
                timer_id: "timer-1".into(),
            },
        );
        assert_eq!(outcome1, CommandOutcome::Success);

        // Task should still match at seq 0
        let outcome2 = execute_command(
            &engine,
            &Command::ScheduleTask {
                task_type: "task-A".into(),
            },
        );
        assert_eq!(outcome2, CommandOutcome::Success);

        // Child should still match at seq 0
        let outcome3 = execute_command(
            &engine,
            &Command::ScheduleChild {
                name: "child-1".into(),
            },
        );
        assert_eq!(outcome3, CommandOutcome::Success);
    }

    #[test]
    fn test_execute_commands_batch() {
        let model = make_model();
        let engine = build_engine_from_model(&model);

        let commands = vec![
            Command::ScheduleTask {
                task_type: "task-A".into(),
            },
            Command::StartTimer {
                timer_id: "timer-1".into(),
            },
            Command::ScheduleTask {
                task_type: "task-B".into(),
            },
        ];

        let outcomes = execute_commands(&engine, &commands);

        assert_eq!(outcomes.len(), 3);
        assert!(outcomes
            .iter()
            .all(|o| matches!(o, CommandOutcome::Success)));
    }

    #[test]
    fn test_outcomes_have_violation() {
        let outcomes = vec![
            CommandOutcome::Success,
            CommandOutcome::Violation("error".into()),
            CommandOutcome::Success,
        ];

        assert!(outcomes_have_violation(&outcomes));

        let all_success = vec![CommandOutcome::Success, CommandOutcome::Success];
        assert!(!outcomes_have_violation(&all_success));
    }
}
