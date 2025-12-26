# Implementation Plan: Property-Based Replay Testing

## Status

**Superseded** - The goals of this plan have been achieved through other implementations.

## Overview

Use property-based testing (proptest) to generate random event sequences and verify the real `ReplayEngine` behaves correctly. Unlike model checking which verifies an abstract specification, this approach tests the actual implementation.

**Problem Solved:** Current model checking tests verify specifications but don't exercise real code. Property-based testing generates thousands of random scenarios against the real implementation.

**Design Reference:** [Stateright Model Checking Design](../design/20251225_stateright-model-checking.md)

---

## Superseded By

This plan's goals have been achieved through the following implementations:

| Proposed Property | Implemented In |
|-------------------|----------------|
| Matching commands succeed | `sdk/tests/unit/state_machine_props.rs` - `command_with_matching_event_validates` |
| Mismatched commands fail | `sdk/tests/unit/state_machine_props.rs` - 6 `detects_*_change` property tests |
| Extension is allowed | `sdk/tests/unit/state_machine_props.rs` - `new_commands_always_valid` |
| Per-type independence | `sdk/tests/model/comparison.rs` - 435 paths test real ReplayEngine |
| Deterministic results | `sdk/tests/unit/determinism_props.rs` - 9 workflow determinism tests |

### Key Achievement

The original concern was that "models don't test real code." This was addressed by:

1. **`sdk/tests/model/bridge.rs`** - Bridges Stateright model commands to real `ReplayEngine`
2. **`sdk/tests/model/comparison.rs`** - Runs 435 command paths through both model AND real implementation, verifying they agree

This provides stronger guarantees than the proposed approach because it exhaustively compares model specification against implementation behavior.

---

## Approach

### Core Idea

Generate random but valid replay scenarios and verify invariants hold:

```rust
proptest! {
    #[test]
    fn replay_engine_maintains_per_type_sequences(
        events in arbitrary_event_sequence(1..10),
        commands in arbitrary_command_sequence(1..10),
    ) {
        let engine = ReplayEngine::new(events.clone());

        for cmd in commands {
            match replay_command(&engine, &cmd) {
                Ok(_) => { /* command matched or extended */ }
                Err(DeterminismViolation { .. }) => {
                    // Verify violation is correct
                    prop_assert!(command_should_violate(&events, &cmd));
                }
            }
        }
    }
}
```

### Properties to Test

1. **Matching commands succeed**: If command matches event at correct per-type sequence, no violation
2. **Mismatched commands fail**: If command differs from event at same sequence, violation occurs
3. **Extension is allowed**: Commands beyond replay history always succeed
4. **Per-type independence**: Task sequence doesn't affect timer sequence
5. **Deterministic results**: Same events + same commands = same outcome

---

## Phase 1: Arbitrary Generators

### 1.1 Create Test Generators Module

**File:** `sdk/tests/proptest_generators.rs` (new file)

```rust
use proptest::prelude::*;
use flovyn_core::workflow::event::{EventType, ReplayEvent};

/// Generate arbitrary task type names
pub fn arbitrary_task_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("task-A".to_string()),
        Just("task-B".to_string()),
        Just("task-C".to_string()),
        "[a-z]{1,10}".prop_map(|s| format!("task-{}", s)),
    ]
}

/// Generate arbitrary timer IDs
pub fn arbitrary_timer_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("timer-1".to_string()),
        Just("timer-2".to_string()),
        "[a-z0-9]{1,8}".prop_map(|s| format!("timer-{}", s)),
    ]
}

/// Generate a TaskScheduled event
pub fn arbitrary_task_scheduled_event() -> impl Strategy<Value = ReplayEvent> {
    arbitrary_task_type().prop_map(|task_type| {
        ReplayEvent::new(EventType::TaskScheduled, serde_json::json!({
            "taskType": task_type,
            "taskExecutionId": format!("exec-{}", uuid::Uuid::new_v4()),
        }))
    })
}

/// Generate an arbitrary sequence of replay events
pub fn arbitrary_event_sequence(size: impl Into<SizeRange>) -> impl Strategy<Value = Vec<ReplayEvent>> {
    prop::collection::vec(
        prop_oneof![
            arbitrary_task_scheduled_event(),
            arbitrary_timer_started_event(),
            arbitrary_child_workflow_event(),
        ],
        size,
    )
}
```

### 1.2 Create Command Generators

```rust
/// Commands that can be issued during replay
#[derive(Debug, Clone)]
pub enum TestCommand {
    ScheduleTask { task_type: String },
    StartTimer { timer_id: String },
    ScheduleChild { name: String },
}

/// Generate arbitrary commands
pub fn arbitrary_command() -> impl Strategy<Value = TestCommand> {
    prop_oneof![
        arbitrary_task_type().prop_map(|t| TestCommand::ScheduleTask { task_type: t }),
        arbitrary_timer_id().prop_map(|t| TestCommand::StartTimer { timer_id: t }),
        arbitrary_child_name().prop_map(|n| TestCommand::ScheduleChild { name: n }),
    ]
}

/// Generate matching commands for a given event sequence
pub fn matching_commands_for(events: &[ReplayEvent]) -> Vec<TestCommand> {
    events.iter().map(|e| match e.event_type() {
        EventType::TaskScheduled => TestCommand::ScheduleTask {
            task_type: e.get_string("taskType").unwrap(),
        },
        EventType::TimerStarted => TestCommand::StartTimer {
            timer_id: e.get_string("timerId").unwrap(),
        },
        // ... etc
    }).collect()
}
```

---

## Phase 2: Property Tests

### 2.1 Create Property Test Module

**File:** `sdk/tests/unit/replay_properties.rs` (new file)

```rust
use proptest::prelude::*;
use flovyn_core::workflow::ReplayEngine;

proptest! {
    /// Matching commands never cause violations
    #[test]
    fn matching_commands_succeed(
        events in arbitrary_event_sequence(1..5)
    ) {
        let engine = ReplayEngine::new(events.clone());
        let commands = matching_commands_for(&events);

        for cmd in commands {
            let result = execute_command(&engine, &cmd);
            prop_assert!(result.is_ok(), "Matching command should succeed");
        }
    }

    /// Extension beyond history is always allowed
    #[test]
    fn extension_always_succeeds(
        events in arbitrary_event_sequence(0..3),
        extra_commands in prop::collection::vec(arbitrary_command(), 1..5),
    ) {
        let engine = ReplayEngine::new(events.clone());

        // First, execute matching commands
        let matching = matching_commands_for(&events);
        for cmd in matching {
            execute_command(&engine, &cmd).unwrap();
        }

        // Then, execute extra commands (extension)
        for cmd in extra_commands {
            let result = execute_command(&engine, &cmd);
            prop_assert!(result.is_ok(), "Extension command should succeed");
        }
    }

    /// Per-type sequences are independent
    #[test]
    fn per_type_sequences_independent(
        task_events in prop::collection::vec(arbitrary_task_scheduled_event(), 1..3),
        timer_events in prop::collection::vec(arbitrary_timer_started_event(), 1..3),
    ) {
        // Interleave events in different orders
        let mut events = Vec::new();
        events.extend(task_events.iter().cloned());
        events.extend(timer_events.iter().cloned());

        let engine = ReplayEngine::new(events);

        // Execute tasks first, then timers
        for e in &task_events {
            let cmd = TestCommand::ScheduleTask {
                task_type: e.get_string("taskType").unwrap(),
            };
            prop_assert!(execute_command(&engine, &cmd).is_ok());
        }

        for e in &timer_events {
            let cmd = TestCommand::StartTimer {
                timer_id: e.get_string("timerId").unwrap(),
            };
            prop_assert!(execute_command(&engine, &cmd).is_ok());
        }
    }

    /// Mismatched commands cause violations
    #[test]
    fn mismatched_commands_fail(
        events in arbitrary_event_sequence(1..3),
    ) {
        let engine = ReplayEngine::new(events.clone());

        // Find first task event and try wrong type
        if let Some(task_event) = events.iter().find(|e| e.event_type() == EventType::TaskScheduled) {
            let wrong_type = format!("{}-WRONG", task_event.get_string("taskType").unwrap());
            let cmd = TestCommand::ScheduleTask { task_type: wrong_type };

            let result = execute_command(&engine, &cmd);
            prop_assert!(result.is_err(), "Mismatched command should fail");
        }
    }
}
```

### 2.2 Helper Functions

```rust
/// Execute a test command against the replay engine
fn execute_command(engine: &ReplayEngine, cmd: &TestCommand) -> Result<(), DeterminismViolation> {
    match cmd {
        TestCommand::ScheduleTask { task_type } => {
            let seq = engine.next_task_seq();
            if let Some(event) = engine.get_task_event(seq) {
                let expected = event.get_string("taskType").unwrap_or_default();
                if expected != *task_type {
                    return Err(DeterminismViolation::TaskTypeMismatch {
                        sequence: seq,
                        expected,
                        got: task_type.clone(),
                    });
                }
            }
            Ok(())
        }
        // ... other commands
    }
}
```

---

## Phase 3: Regression Tests from Failures

### 3.1 Shrinking and Reproduction

Proptest automatically shrinks failing cases to minimal examples. When a property fails:

1. Proptest outputs a seed that reproduces the failure
2. The failing case is shrunk to the smallest example
3. Add the minimal case as a regression test

```rust
#[test]
fn regression_issue_123() {
    // Minimal case found by proptest
    let events = vec![
        task_event("task-A"),
        timer_event("timer-1"),
        task_event("task-B"),
    ];

    let engine = ReplayEngine::new(events);

    // This specific sequence caused a bug
    engine.next_task_seq(); // task-A
    engine.next_timer_seq(); // timer-1
    engine.next_task_seq(); // task-B - was incorrectly matching timer

    // Verify fix
    assert_eq!(engine.get_task_event(1).unwrap().get_string("taskType"), Some("task-B"));
}
```

---

## TODO List

### Phase 1: Arbitrary Generators
- [ ] Create `sdk/tests/proptest_generators.rs`
- [ ] Implement `arbitrary_task_type()` generator
- [ ] Implement `arbitrary_timer_id()` generator
- [ ] Implement `arbitrary_child_name()` generator
- [ ] Implement `arbitrary_task_scheduled_event()` generator
- [ ] Implement `arbitrary_timer_started_event()` generator
- [ ] Implement `arbitrary_child_workflow_event()` generator
- [ ] Implement `arbitrary_event_sequence()` generator
- [ ] Implement `arbitrary_command()` generator
- [ ] Implement `matching_commands_for()` helper

### Phase 2: Property Tests
- [ ] Create `sdk/tests/unit/replay_properties.rs`
- [ ] Implement `matching_commands_succeed` property
- [ ] Implement `extension_always_succeeds` property
- [ ] Implement `per_type_sequences_independent` property
- [ ] Implement `mismatched_commands_fail` property
- [ ] Implement `execute_command()` helper that uses real ReplayEngine
- [ ] Add property tests to unit test suite

### Phase 3: Integration
- [ ] Configure proptest case count (default 256, increase for CI)
- [ ] Add proptest regressions file (`.proptest-regressions/`)
- [ ] Document how to reproduce failures from seeds

---

## Verification Checklist

- [ ] `cargo test --test unit -p flovyn-sdk` passes with property tests
- [ ] Property tests run in < 30 seconds
- [ ] At least 256 cases per property
- [ ] Proptest regressions file committed

---

## Expected Outcomes

### What This Will Find

1. **Edge cases in ReplayEngine** - Random sequences may trigger unexpected behavior
2. **Off-by-one errors** - Sequence counter bugs
3. **Type confusion** - Wrong event type matched at sequence

### Success Criteria

1. Property tests pass with 1000+ cases
2. OR property tests find real bugs that we fix
3. Regressions are captured and prevented
