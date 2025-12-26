//! Model-Implementation Comparison Tests
//!
//! This module runs the same scenarios through both the Stateright model
//! and the real ReplayEngine implementation, asserting they produce identical results.
//!
//! ## What This Catches
//!
//! 1. **Model drift** - When implementation changes but model doesn't (or vice versa)
//! 2. **Specification bugs** - Model is wrong about expected behavior
//! 3. **Implementation bugs** - Implementation doesn't match intended behavior
//! 4. **Edge cases** - Model explores all paths, implementation must handle all
//!
//! ## How It Works
//!
//! For each command path explored by the model:
//! 1. Replay the commands through the model (tracking state transitions)
//! 2. Replay the same commands through the real ReplayEngine
//! 3. Compare: did both agree on whether a violation occurred?
//!
//! If they disagree, we found a bug!
//!
//! ## Mutation Validation
//!
//! We also inject known bugs (mutations) and verify the comparison tests detect them.
//! This proves our tests are actually capable of catching real bugs.

use super::bridge::{build_engine_from_model, execute_command, CommandOutcome};
use super::sequence_matching::{Command, SequenceMatchingModel};
use flovyn_core::workflow::ReplayEngine;
use stateright::Model;

// ============================================================================
// Mutations - Intentional bugs to verify tests catch them
// ============================================================================

/// Mutation type that can be injected into command execution
#[derive(Debug, Clone, Copy)]
pub enum Mutation {
    /// Off-by-one: use seq+1 instead of seq for task lookup
    OffByOneTaskSeq,
    /// Wrong counter: use timer command count for task lookup
    WrongCounterTaskUsesTimerCount,
    /// Inverted comparison: succeed when types don't match
    InvertedTaskComparison,
    /// Skip boundary check: always report success even during replay
    NoBoundaryCheck,
}

/// State tracker for mutation testing.
///
/// Tracks how many commands of each type have been executed,
/// allowing us to simulate wrong-counter bugs without side effects.
#[derive(Default)]
struct MutationTracker {
    task_count: u32,
    timer_count: u32,
    child_count: u32,
}

/// Execute a command with an optional mutation injected.
///
/// This allows us to verify that our comparison tests would catch real bugs.
fn execute_command_with_mutation_tracked(
    engine: &ReplayEngine,
    cmd: &Command,
    mutation: Mutation,
    tracker: &mut MutationTracker,
) -> CommandOutcome {
    match cmd {
        Command::ScheduleTask { task_type } => {
            let seq = engine.next_task_seq();

            // Apply mutations - use wrong seq for lookup
            let lookup_seq = match mutation {
                Mutation::OffByOneTaskSeq => seq.wrapping_add(1), // BUG: off-by-one
                Mutation::WrongCounterTaskUsesTimerCount => tracker.timer_count, // BUG: uses timer count
                _ => seq,
            };

            tracker.task_count += 1;

            match mutation {
                Mutation::NoBoundaryCheck => {
                    // BUG: Skip validation entirely
                    return CommandOutcome::Success;
                }
                Mutation::InvertedTaskComparison => {
                    // BUG: Inverted comparison
                    if let Some(event) = engine.get_task_event(lookup_seq) {
                        let expected = event.get_string("taskType").unwrap_or("");
                        if expected == task_type {
                            // WRONG: fail when they match
                            return CommandOutcome::Violation(format!(
                                "TaskTypeMismatch at Task({}): expected '{}', got '{}'",
                                lookup_seq, expected, task_type
                            ));
                        }
                    }
                    return CommandOutcome::Success;
                }
                _ => {}
            }

            // Normal validation with potentially wrong seq
            if let Some(event) = engine.get_task_event(lookup_seq) {
                let expected = event.get_string("taskType").unwrap_or("");
                if expected != task_type {
                    return CommandOutcome::Violation(format!(
                        "TaskTypeMismatch at Task({}): expected '{}', got '{}'",
                        lookup_seq, expected, task_type
                    ));
                }
            }
            CommandOutcome::Success
        }
        Command::StartTimer { timer_id } => {
            tracker.timer_count += 1;
            // Execute normally for timer commands
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
            tracker.child_count += 1;
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

/// Compare model and implementation with a mutation injected.
///
/// Returns Ok(()) if they agree (mutation NOT detected - bad!),
/// Returns Err if they disagree (mutation detected - good!).
fn compare_path_with_mutation(
    model: &SequenceMatchingModel,
    path: &[Command],
    mutation: Mutation,
) -> Result<(), String> {
    // Execute path on model (no mutation - this is the specification)
    let mut model_state = model.init_states()[0].clone();
    for cmd in path {
        if let Some(next) = model.next_state(&model_state, cmd.clone()) {
            model_state = next;
        }
    }
    let model_violated = model_state.violation.is_some();

    // Execute path on mutated implementation
    let engine = build_engine_from_model(model);
    let mut tracker = MutationTracker::default();
    let mut impl_violated = false;
    let mut impl_violation_msg = None;

    for cmd in path {
        match execute_command_with_mutation_tracked(&engine, cmd, mutation, &mut tracker) {
            CommandOutcome::Success => {}
            CommandOutcome::Violation(msg) => {
                impl_violated = true;
                impl_violation_msg = Some(msg);
                break;
            }
        }
    }

    // Compare outcomes
    if model_violated != impl_violated {
        return Err(format!(
            "Mutation {:?} detected on path {:?}\n  Model: {:?}\n  Impl: {:?}",
            mutation, path, model_state.violation, impl_violation_msg
        ));
    }

    Ok(())
}

/// Generate all possible command paths up to a given length.
///
/// Uses DFS to enumerate all paths through the model's state space.
/// Each path is a sequence of commands that can be executed.
fn generate_all_paths(model: &SequenceMatchingModel, max_len: usize) -> Vec<Vec<Command>> {
    let mut result = Vec::new();
    let mut stack = vec![(model.init_states()[0].clone(), vec![])];

    while let Some((state, path)) = stack.pop() {
        // Always add the current path (including empty path)
        result.push(path.clone());

        // Stop extending if we've reached max length or hit a violation
        if path.len() >= max_len || state.violation.is_some() {
            continue;
        }

        // Get all possible actions from this state
        let mut actions = Vec::new();
        model.actions(&state, &mut actions);

        // Add extensions for each action
        for action in actions {
            if let Some(next_state) = model.next_state(&state, action.clone()) {
                let mut new_path = path.clone();
                new_path.push(action);
                stack.push((next_state, new_path));
            }
        }
    }

    result
}

/// Compare model and implementation for a single command path.
///
/// Returns Ok(()) if they agree, Err with details if they disagree.
fn compare_path(model: &SequenceMatchingModel, path: &[Command]) -> Result<(), String> {
    // Execute path on model
    let mut model_state = model.init_states()[0].clone();
    for cmd in path {
        if let Some(next) = model.next_state(&model_state, cmd.clone()) {
            model_state = next;
        }
    }
    let model_violated = model_state.violation.is_some();

    // Execute path on real implementation
    let engine = build_engine_from_model(model);
    let mut impl_violated = false;
    let mut impl_violation_msg = None;

    for cmd in path {
        match execute_command(&engine, cmd) {
            CommandOutcome::Success => {}
            CommandOutcome::Violation(msg) => {
                impl_violated = true;
                impl_violation_msg = Some(msg);
                break; // Stop on first violation
            }
        }
    }

    // Compare outcomes
    if model_violated != impl_violated {
        return Err(format!(
            "Disagreement on path {:?}\n  Model violation: {:?}\n  Impl violation: {:?}",
            path, model_state.violation, impl_violation_msg
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small model for quick testing
    fn small_model() -> SequenceMatchingModel {
        SequenceMatchingModel {
            task_events: vec!["task-A".into()],
            timer_events: vec!["timer-1".into()],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-WRONG".into()],
            possible_timer_ids: vec!["timer-1".into(), "timer-WRONG".into()],
            possible_child_names: vec![],
            max_commands: 3,
        }
    }

    /// Medium model for more thorough testing
    fn medium_model() -> SequenceMatchingModel {
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

    // =========================================================================
    // Exhaustive Comparison Tests
    // =========================================================================

    /// Exhaustively compare model and implementation for all paths (small model).
    #[test]
    fn model_matches_implementation_small() {
        let model = small_model();
        let paths = generate_all_paths(&model, 3);

        println!("Testing {} paths from small model...", paths.len());

        let mut violations_tested = 0;
        let mut successes_tested = 0;

        for path in &paths {
            if let Err(msg) = compare_path(&model, path) {
                panic!("Model-implementation mismatch:\n{}", msg);
            }

            // Track statistics
            let mut state = model.init_states()[0].clone();
            for cmd in path {
                if let Some(next) = model.next_state(&state, cmd.clone()) {
                    state = next;
                }
            }
            if state.violation.is_some() {
                violations_tested += 1;
            } else {
                successes_tested += 1;
            }
        }

        println!("Paths tested: {}", paths.len());
        println!("  Paths with violations: {}", violations_tested);
        println!("  Paths without violations: {}", successes_tested);
        assert!(paths.len() > 10, "Should test a meaningful number of paths");
        assert!(violations_tested > 0, "Should test some violation paths");
        assert!(successes_tested > 0, "Should test some success paths");
    }

    /// Exhaustively compare model and implementation (medium model).
    #[test]
    fn model_matches_implementation_medium() {
        let model = medium_model();
        let paths = generate_all_paths(&model, 4);

        println!("Testing {} paths from medium model...", paths.len());

        for path in &paths {
            if let Err(msg) = compare_path(&model, path) {
                panic!("Model-implementation mismatch:\n{}", msg);
            }
        }

        println!("All {} paths match!", paths.len());
    }

    // =========================================================================
    // Specific Scenario Tests
    // =========================================================================

    /// Test interleaved command types.
    ///
    /// Verifies that per-type sequences are independent in both model and implementation.
    #[test]
    fn model_matches_impl_interleaved_types() {
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into()],
            timer_events: vec!["timer-1".into()],
            child_events: vec![],
            possible_task_types: vec!["task-A".into()],
            possible_timer_ids: vec!["timer-1".into()],
            possible_child_names: vec![],
            max_commands: 2,
        };

        // Test: timer first, then task (different order than events listed)
        let path = vec![
            Command::StartTimer {
                timer_id: "timer-1".into(),
            },
            Command::ScheduleTask {
                task_type: "task-A".into(),
            },
        ];

        assert!(
            compare_path(&model, &path).is_ok(),
            "Interleaved order should work"
        );
    }

    /// Test extension beyond replay history.
    ///
    /// Verifies that new commands beyond history are allowed in both model and implementation.
    #[test]
    fn model_matches_impl_extension() {
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into()],
            timer_events: vec![],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-NEW".into()],
            possible_timer_ids: vec![],
            possible_child_names: vec![],
            max_commands: 3,
        };

        // Replay history, then extend
        let path = vec![
            Command::ScheduleTask {
                task_type: "task-A".into(),
            }, // Replay
            Command::ScheduleTask {
                task_type: "task-NEW".into(),
            }, // Extension
        ];

        assert!(
            compare_path(&model, &path).is_ok(),
            "Extension should be allowed"
        );
    }

    /// Test violation detection.
    ///
    /// Verifies that mismatched commands cause violations in both model and implementation.
    #[test]
    fn model_matches_impl_violation() {
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into()],
            timer_events: vec![],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-WRONG".into()],
            possible_timer_ids: vec![],
            possible_child_names: vec![],
            max_commands: 2,
        };

        // Wrong task type should cause violation
        let path = vec![Command::ScheduleTask {
            task_type: "task-WRONG".into(),
        }];

        assert!(
            compare_path(&model, &path).is_ok(),
            "Violation should be detected by both"
        );
    }

    /// Test all matching commands succeed.
    #[test]
    fn model_matches_impl_all_matching() {
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into(), "task-B".into()],
            timer_events: vec!["timer-1".into()],
            child_events: vec!["child-1".into()],
            possible_task_types: vec!["task-A".into(), "task-B".into()],
            possible_timer_ids: vec!["timer-1".into()],
            possible_child_names: vec!["child-1".into()],
            max_commands: 4,
        };

        // All matching commands in order
        let path = vec![
            Command::ScheduleTask {
                task_type: "task-A".into(),
            },
            Command::ScheduleTask {
                task_type: "task-B".into(),
            },
            Command::StartTimer {
                timer_id: "timer-1".into(),
            },
            Command::ScheduleChild {
                name: "child-1".into(),
            },
        ];

        assert!(
            compare_path(&model, &path).is_ok(),
            "All matching should succeed"
        );
    }

    /// Test violation at different positions.
    #[test]
    fn model_matches_impl_violation_at_different_positions() {
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into(), "task-B".into()],
            timer_events: vec![],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-B".into(), "task-WRONG".into()],
            possible_timer_ids: vec![],
            possible_child_names: vec![],
            max_commands: 3,
        };

        // Violation at first position
        let path1 = vec![Command::ScheduleTask {
            task_type: "task-WRONG".into(),
        }];
        assert!(compare_path(&model, &path1).is_ok());

        // Violation at second position
        let path2 = vec![
            Command::ScheduleTask {
                task_type: "task-A".into(),
            },
            Command::ScheduleTask {
                task_type: "task-WRONG".into(),
            },
        ];
        assert!(compare_path(&model, &path2).is_ok());
    }

    // =========================================================================
    // Edge Case Tests
    // =========================================================================

    /// Test empty path.
    #[test]
    fn model_matches_impl_empty_path() {
        let model = small_model();
        let path: Vec<Command> = vec![];

        assert!(
            compare_path(&model, &path).is_ok(),
            "Empty path should be handled"
        );
    }

    /// Test model with no events (pure extension).
    #[test]
    fn model_matches_impl_no_events() {
        let model = SequenceMatchingModel {
            task_events: vec![],
            timer_events: vec![],
            child_events: vec![],
            possible_task_types: vec!["task-A".into()],
            possible_timer_ids: vec!["timer-1".into()],
            possible_child_names: vec![],
            max_commands: 2,
        };

        // All commands are extensions
        let path = vec![
            Command::ScheduleTask {
                task_type: "task-A".into(),
            },
            Command::StartTimer {
                timer_id: "timer-1".into(),
            },
        ];

        assert!(
            compare_path(&model, &path).is_ok(),
            "Pure extension should work"
        );
    }

    /// Test path statistics for coverage.
    #[test]
    fn path_generation_coverage() {
        let model = small_model();
        let paths = generate_all_paths(&model, 3);

        // Verify we generate meaningful paths
        let empty_paths = paths.iter().filter(|p| p.is_empty()).count();
        let single_cmd_paths = paths.iter().filter(|p| p.len() == 1).count();
        let multi_cmd_paths = paths.iter().filter(|p| p.len() > 1).count();

        println!("Path distribution:");
        println!("  Empty: {}", empty_paths);
        println!("  Single command: {}", single_cmd_paths);
        println!("  Multi command: {}", multi_cmd_paths);

        assert_eq!(empty_paths, 1, "Should have exactly one empty path");
        assert!(single_cmd_paths > 0, "Should have single command paths");
        assert!(multi_cmd_paths > 0, "Should have multi command paths");
    }

    // =========================================================================
    // Mutation Detection Tests
    // =========================================================================
    //
    // These tests inject known bugs and verify our comparison tests catch them.
    // This proves our tests are actually capable of detecting real bugs.

    /// Model for mutation testing - needs tasks to be replayed
    fn mutation_test_model() -> SequenceMatchingModel {
        SequenceMatchingModel {
            task_events: vec!["task-A".into(), "task-B".into()],
            timer_events: vec![],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-B".into()],
            possible_timer_ids: vec![],
            possible_child_names: vec![],
            max_commands: 3,
        }
    }

    /// Verify off-by-one mutation is detected.
    ///
    /// Bug: Uses seq+1 instead of seq for event lookup.
    /// Result: First task looks up second event, second task looks up nothing.
    #[test]
    fn mutation_off_by_one_is_detected() {
        let model = mutation_test_model();

        // Path that should succeed: schedule task-A (matches event at seq 0)
        let path = vec![Command::ScheduleTask {
            task_type: "task-A".into(),
        }];

        // Without mutation: model and impl agree (both succeed)
        assert!(compare_path(&model, &path).is_ok());

        // With mutation: off-by-one causes lookup at seq 1 (task-B), but we're scheduling task-A
        // Model says: succeed (task-A matches event 0)
        // Mutated impl says: fail (task-A != task-B at lookup seq 1)
        let result = compare_path_with_mutation(&model, &path, Mutation::OffByOneTaskSeq);
        assert!(
            result.is_err(),
            "Off-by-one mutation should be detected!\n{:?}",
            result
        );
    }

    /// Verify wrong-counter mutation is detected.
    ///
    /// Bug: Uses timer command count for task event lookup instead of task count.
    /// This simulates accidentally using the wrong counter variable.
    #[test]
    fn mutation_wrong_counter_is_detected() {
        // Model with 2 different task types
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into(), "task-B".into()],
            timer_events: vec!["timer-1".into()],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-B".into()],
            possible_timer_ids: vec!["timer-1".into()],
            possible_child_names: vec![],
            max_commands: 3,
        };

        // Path: timer first, then tasks
        // With correct code: task-A uses task_seq=0, matches task_events[0]=task-A ✓
        // With wrong counter: task-A uses timer_count=1, looks up task_events[1]=task-B ✗
        let path = vec![
            Command::StartTimer {
                timer_id: "timer-1".into(),
            },
            Command::ScheduleTask {
                task_type: "task-A".into(),
            },
        ];

        // Without mutation: works fine
        assert!(compare_path(&model, &path).is_ok());

        // With mutation: task lookup uses timer_count (which is 1 after timer command)
        // Looks up task_events[1] = "task-B", but we're scheduling "task-A"
        // Model says: success (task-A matches task_events[0])
        // Mutated impl says: violation (task-A != task-B at lookup index 1)
        let result =
            compare_path_with_mutation(&model, &path, Mutation::WrongCounterTaskUsesTimerCount);
        assert!(
            result.is_err(),
            "Wrong-counter mutation should be detected!\nResult: {:?}",
            result
        );
    }

    /// Verify inverted-comparison mutation is detected.
    ///
    /// Bug: Reports violation when types MATCH (opposite of correct behavior).
    #[test]
    fn mutation_inverted_comparison_is_detected() {
        let model = mutation_test_model();

        // Path that should succeed: schedule matching tasks
        let path = vec![Command::ScheduleTask {
            task_type: "task-A".into(),
        }];

        // Without mutation: succeeds
        assert!(compare_path(&model, &path).is_ok());

        // With mutation: inverted comparison
        // Model says: task-A matches event 0 (task-A), succeed
        // Mutated impl says: task-A == task-A, so VIOLATION (inverted!)
        let result = compare_path_with_mutation(&model, &path, Mutation::InvertedTaskComparison);
        assert!(
            result.is_err(),
            "Inverted-comparison mutation should be detected!\n{:?}",
            result
        );
    }

    /// Verify no-boundary-check mutation is detected.
    ///
    /// Bug: Always returns success, even when replaying mismatched commands.
    #[test]
    fn mutation_no_boundary_check_is_detected() {
        // Path that should FAIL: schedule wrong task type
        let path = vec![Command::ScheduleTask {
            task_type: "task-WRONG".into(),
        }];

        // This path causes violation in both model and impl normally
        // (So compare_path returns Ok - they agree on violation)

        // With mutation: no boundary check, always succeeds
        // Model says: task-WRONG != task-A at seq 0, VIOLATION
        // Mutated impl says: skip check, SUCCESS
        // They disagree! Mutation detected.

        // But wait, our model doesn't have "task-WRONG" in possible_task_types
        // Let me use a model that does:
        let model2 = SequenceMatchingModel {
            task_events: vec!["task-A".into()],
            timer_events: vec![],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-WRONG".into()],
            possible_timer_ids: vec![],
            possible_child_names: vec![],
            max_commands: 2,
        };

        let result = compare_path_with_mutation(&model2, &path, Mutation::NoBoundaryCheck);
        assert!(
            result.is_err(),
            "No-boundary-check mutation should be detected!\n{:?}",
            result
        );
    }

    /// Verify all critical mutations are detected across paths.
    #[test]
    fn all_mutations_detected_by_comparison_tests() {
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into(), "task-B".into()],
            timer_events: vec!["timer-1".into()],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-B".into(), "task-WRONG".into()],
            possible_timer_ids: vec!["timer-1".into()],
            possible_child_names: vec![],
            max_commands: 4,
        };

        let mutations = [
            Mutation::OffByOneTaskSeq,
            Mutation::WrongCounterTaskUsesTimerCount,
            Mutation::InvertedTaskComparison,
            Mutation::NoBoundaryCheck,
        ];

        let paths = generate_all_paths(&model, 3);

        for mutation in mutations {
            let mut detected = false;

            for path in &paths {
                if compare_path_with_mutation(&model, path, mutation).is_err() {
                    detected = true;
                    break;
                }
            }

            assert!(
                detected,
                "Mutation {:?} was NOT detected by any path! Our tests have a gap.",
                mutation
            );
        }

        println!("All critical mutations detected by comparison tests!");
    }
}
