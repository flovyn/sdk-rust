# Implementation Plan: Model-Implementation Comparison Testing

## Status

**Implemented** - All phases complete. See verification checklist below.

## Overview

Run the same scenarios through both the Stateright model and the real `ReplayEngine` implementation, asserting they produce identical results. This bridges the gap between abstract specification (model) and concrete implementation.

**Problem Solved:** Current model checking tests verify specifications but don't compare against real code. This approach ensures the implementation matches the model.

**Design Reference:** [Stateright Model Checking Design](../design/20251225_stateright-model-checking.md)

---

## Approach

### Core Idea

For each state transition in the model, execute the same operation on the real implementation and verify the outcomes match:

```rust
#[test]
fn model_matches_implementation() {
    let model = SequenceMatchingModel { /* config */ };

    // Explore all states in the model
    for (state, action, next_state) in model.all_transitions() {
        // Replay the same scenario on real implementation
        let engine = build_engine_from_state(&state);
        let result = execute_action_on_engine(&engine, &action);

        // Compare outcomes
        assert_eq!(
            model_has_violation(&next_state),
            implementation_has_violation(&result),
            "Model and implementation disagree on action {:?}",
            action
        );
    }
}
```

### What This Catches

1. **Model drift** - When implementation changes but model doesn't (or vice versa)
2. **Specification bugs** - Model is wrong about expected behavior
3. **Implementation bugs** - Implementation doesn't match intended behavior
4. **Edge cases** - Model explores all paths, implementation must handle all

---

## Phase 1: Model-to-Implementation Bridge

### 1.1 Create Bridge Module

**File:** `sdk/tests/model/bridge.rs` (new file)

```rust
//! Bridge between Stateright models and real implementation.
//!
//! Converts model states/actions to real ReplayEngine operations
//! and compares outcomes.

use super::sequence_matching::{Command, SequenceMatchingModel, SequenceMatchingState};
use flovyn_core::workflow::event::{EventType, ReplayEvent};
use flovyn_core::workflow::ReplayEngine;

/// Build a ReplayEngine from model configuration
pub fn build_engine_from_model(model: &SequenceMatchingModel) -> ReplayEngine {
    let mut events = Vec::new();

    // Convert model's event history to real events
    for task_type in &model.task_events {
        events.push(ReplayEvent::new(
            EventType::TaskScheduled,
            serde_json::json!({
                "taskType": task_type,
                "taskExecutionId": format!("exec-{}", events.len()),
            }),
        ));
    }

    for timer_id in &model.timer_events {
        events.push(ReplayEvent::new(
            EventType::TimerStarted,
            serde_json::json!({
                "timerId": timer_id,
            }),
        ));
    }

    for name in &model.child_events {
        events.push(ReplayEvent::new(
            EventType::ChildWorkflowInitiated,
            serde_json::json!({
                "childExecutionName": name,
            }),
        ));
    }

    ReplayEngine::new(events)
}

/// Outcome of executing a command
#[derive(Debug, PartialEq)]
pub enum CommandOutcome {
    /// Command succeeded (matched or extended)
    Success,
    /// Command caused determinism violation
    Violation(String),
}

/// Execute a model command on real ReplayEngine
pub fn execute_command(engine: &ReplayEngine, cmd: &Command) -> CommandOutcome {
    match cmd {
        Command::ScheduleTask { task_type } => {
            let seq = engine.next_task_seq();
            if let Some(event) = engine.get_task_event(seq) {
                let expected = event.get_string("taskType").unwrap_or_default();
                if expected != *task_type {
                    return CommandOutcome::Violation(format!(
                        "TaskTypeMismatch at Task({}): expected '{}', got '{}'",
                        seq, expected, task_type
                    ));
                }
            }
            CommandOutcome::Success
        }
        Command::StartTimer { timer_id } => {
            let seq = engine.next_timer_seq();
            if let Some(event) = engine.get_timer_event(seq) {
                let expected = event.get_string("timerId").unwrap_or_default();
                if expected != *timer_id {
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
                let expected = event.get_string("childExecutionName").unwrap_or_default();
                if expected != *name {
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
```

### 1.2 State Reconstruction

```rust
/// Reconstruct engine state by replaying commands
pub fn replay_commands_on_engine(
    engine: &ReplayEngine,
    commands: &[Command],
) -> Vec<CommandOutcome> {
    commands.iter().map(|cmd| execute_command(engine, cmd)).collect()
}

/// Check if model state has violation
pub fn model_has_violation(state: &SequenceMatchingState) -> bool {
    state.violation.is_some()
}

/// Get violation message from model state
pub fn model_violation_message(state: &SequenceMatchingState) -> Option<&str> {
    state.violation.as_deref()
}
```

---

## Phase 2: Comparison Tests

### 2.1 Exhaustive State Comparison

**File:** `sdk/tests/model/comparison.rs` (new file)

```rust
use super::bridge::*;
use super::sequence_matching::*;
use stateright::Model;

/// Compare model against implementation for all reachable states
#[test]
fn model_matches_implementation_exhaustive() {
    let model = SequenceMatchingModel {
        task_events: vec!["task-A".into(), "task-B".into()],
        timer_events: vec!["timer-1".into()],
        child_events: vec![],
        possible_task_types: vec!["task-A".into(), "task-B".into(), "task-WRONG".into()],
        possible_timer_ids: vec!["timer-1".into(), "timer-WRONG".into()],
        possible_child_names: vec![],
        max_commands: 4,
    };

    let mut states_checked = 0;
    let mut violations_found = 0;

    // Use BFS to explore all states
    let checker = model.checker().spawn_bfs().join();

    // For each unique state, verify implementation matches
    // Note: We need to trace paths to states, not just final states
    for path in generate_all_paths(&model, 4) {
        let engine = build_engine_from_model(&model);

        // Execute commands and track outcomes
        let mut model_state = model.init_states()[0].clone();
        let mut impl_outcomes = Vec::new();

        for cmd in &path {
            // Model transition
            if let Some(next) = model.next_state(&model_state, cmd.clone()) {
                model_state = next;
            }

            // Implementation execution
            let outcome = execute_command(&engine, cmd);
            impl_outcomes.push(outcome);
        }

        // Compare final states
        let model_violated = model_has_violation(&model_state);
        let impl_violated = impl_outcomes.iter().any(|o| matches!(o, CommandOutcome::Violation(_)));

        assert_eq!(
            model_violated, impl_violated,
            "Disagreement on path {:?}\nModel violation: {:?}\nImpl outcomes: {:?}",
            path, model_state.violation, impl_outcomes
        );

        states_checked += 1;
        if model_violated {
            violations_found += 1;
        }
    }

    println!("States checked: {}", states_checked);
    println!("Violations found: {}", violations_found);
}

/// Generate all command paths up to given length
fn generate_all_paths(model: &SequenceMatchingModel, max_len: usize) -> Vec<Vec<Command>> {
    let mut paths = vec![vec![]];
    let mut result = Vec::new();

    while let Some(path) = paths.pop() {
        if path.len() >= max_len {
            result.push(path);
            continue;
        }

        // Get current state by replaying path
        let mut state = model.init_states()[0].clone();
        for cmd in &path {
            if let Some(next) = model.next_state(&state, cmd.clone()) {
                state = next;
            }
        }

        // Generate extensions
        let mut actions = Vec::new();
        model.actions(&state, &mut actions);

        if actions.is_empty() {
            result.push(path);
        } else {
            for action in actions {
                let mut new_path = path.clone();
                new_path.push(action);
                paths.push(new_path);
            }
        }
    }

    result
}
```

### 2.2 Specific Scenario Comparisons

```rust
/// Test specific scenarios that are likely to have edge cases
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
    let commands = vec![
        Command::StartTimer { timer_id: "timer-1".into() },
        Command::ScheduleTask { task_type: "task-A".into() },
    ];

    let engine = build_engine_from_model(&model);
    let mut state = model.init_states()[0].clone();

    for cmd in &commands {
        let impl_outcome = execute_command(&engine, cmd);
        state = model.next_state(&state, cmd.clone()).unwrap();

        let model_violated = model_has_violation(&state);
        let impl_violated = matches!(impl_outcome, CommandOutcome::Violation(_));

        assert_eq!(model_violated, impl_violated,
            "Disagreement on {:?}", cmd);
    }
}

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
    let commands = vec![
        Command::ScheduleTask { task_type: "task-A".into() }, // Replay
        Command::ScheduleTask { task_type: "task-NEW".into() }, // Extension
    ];

    let engine = build_engine_from_model(&model);
    let mut state = model.init_states()[0].clone();

    for cmd in &commands {
        let impl_outcome = execute_command(&engine, cmd);
        state = model.next_state(&state, cmd.clone()).unwrap();

        // Both should succeed (no violation)
        assert!(!model_has_violation(&state));
        assert!(matches!(impl_outcome, CommandOutcome::Success));
    }
}
```

---

## Phase 3: Continuous Verification

### 3.1 Add to CI

The comparison tests run as part of the model test suite, ensuring model and implementation stay in sync.

### 3.2 Failure Handling

When comparison fails:

1. **Model is wrong**: Update model to match intended behavior
2. **Implementation is wrong**: Fix the bug in ReplayEngine
3. **Both are wrong**: Clarify intended behavior first

```rust
/// Document intentional differences (if any)
#[test]
#[should_panic(expected = "Known difference")]
fn known_model_impl_difference() {
    // Document any intentional differences here
    // (Ideally there should be none)
    panic!("Known difference: [describe]");
}
```

---

## TODO List

### Phase 1: Model-to-Implementation Bridge
- [x] Create `sdk/tests/model/bridge.rs`
- [x] Implement `build_engine_from_model()`
- [x] Implement `CommandOutcome` enum
- [x] Implement `execute_command()` for ScheduleTask
- [x] Implement `execute_command()` for StartTimer
- [x] Implement `execute_command()` for ScheduleChild
- [x] Implement `execute_commands()` (batch version)
- [x] Implement `model_has_violation()` and `model_violation_message()`
- [x] Update `sdk/tests/model/mod.rs` to include bridge module

### Phase 2: Comparison Tests
- [x] Create `sdk/tests/model/comparison.rs`
- [x] Implement `generate_all_paths()` helper
- [x] Implement `model_matches_implementation_small` test (37 paths)
- [x] Implement `model_matches_implementation_medium` test (435 paths)
- [x] Implement `model_matches_impl_interleaved_types` test
- [x] Implement `model_matches_impl_extension` test
- [x] Implement `model_matches_impl_violation` test
- [x] Implement `model_matches_impl_all_matching` test
- [x] Implement `model_matches_impl_violation_at_different_positions` test
- [x] Update `sdk/tests/model/mod.rs` to include comparison module

### Phase 3: Integration
- [x] Verify comparison tests run in CI (model tests already in CI)
- [x] Tests include diagnostic output for debugging failures

---

## Verification Checklist

- [x] All comparison tests pass (10 tests)
- [x] `cargo test --test model -p flovyn-sdk` includes comparison tests (34 total model tests)
- [x] No intentional differences between model and implementation
- [x] CI runs comparison tests (part of model-check job)

---

## Expected Outcomes

### What This Will Find

1. **Model drift** - Implementation changed but model wasn't updated
2. **Specification bugs** - Model incorrectly specifies behavior
3. **Implementation bugs** - ReplayEngine doesn't match intended behavior

### Success Criteria

1. All paths in model produce same outcome in implementation
2. OR discrepancies are found and fixed
3. Continuous verification prevents future drift
