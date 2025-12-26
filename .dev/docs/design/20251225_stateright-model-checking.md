# Design: Stateright Model Checking for SDK

## Status
**Implemented** - All core objectives achieved

## Problem Statement

The Flovyn SDK has subtle correctness requirements:

1. **Replay must be deterministic**: Same workflow code + same events = same commands
2. **Per-type sequence matching**: Independent counters for tasks, timers, children, etc.
3. **Parallel combinators**: `join_all` waits for all, `select` returns first winner
4. **Extension is allowed**: New commands beyond replay history are valid

Current testing:
- **Unit tests**: Cover individual functions, but not state machine invariants across sequences
- **E2E tests**: Cover happy paths against real server, but not all interleavings

**What bugs could slip through?**

1. A code path where per-type sequences get out of sync
2. A parallel completion order that causes incorrect matching
3. A determinism violation that's detected too late (after partial execution)
4. A combinator edge case (e.g., all tasks fail in join_all)

## Critical Questions

Before implementing, we need to answer:

### 1. What specific bugs would model checking catch?

| Potential Bug | Would Model Checking Find It? | Current Test Coverage |
|---------------|-------------------------------|----------------------|
| Wrong per-type sequence increment | Yes - exhaustive paths | Partial - some E2E tests |
| Parallel completion order issue | Yes - explores all orderings | No - E2E tests one ordering |
| join_all doesn't fail fast | Yes - verifies invariant | Partial - one failure test |
| select doesn't cancel losers | Maybe - if we model cancellation | No |
| Replay matches wrong event | Yes - explores all command sequences | Partial - specific cases |

### 2. Is the state space tractable?

**Rough calculations:**

For replay validation with:
- 3 event types × 2 possible commands each = 6 commands
- Max 5 commands = 6^5 = 7,776 states
- With mismatch detection = much smaller (violations terminate paths)

**Verdict**: Tractable for small models. Need to be careful with parallel execution models.

### 3. Is stateright the right tool?

**Pros:**
- Same language as production code
- Can reuse production types directly
- Good documentation and examples

**Cons:**
- Another dependency
- Learning curve
- Models can drift from code

**Alternative: Exhaustive unit tests with hand-crafted scenarios**
- Cheaper to implement
- Easier to maintain
- But: manual, not exhaustive

## Design

### What to Model

Focus on **high-value, tractable** models:

| Model | Value | Tractability | Priority |
|-------|-------|--------------|----------|
| Per-type sequence matching | High - core correctness | High - small state space | P0 |
| Determinism violation detection | High - catches bugs early | High - terminates on violation | P0 |
| join_all semantics | Medium - subtle edge cases | Medium - N! orderings | P1 |
| select semantics | Medium - cancellation logic | Medium - similar to join_all | P1 |
| Worker lifecycle | Low - simpler state machine | High - few states | P2 |

### What NOT to Model

- **Full workflow execution**: Too many states
- **Network behavior**: Stateright handles this, but adds complexity
- **Server interaction**: Covered by server-side model checking
- **Timeouts**: Hard to model meaningfully

### Model 1: Per-Type Sequence Matching (Priority: P0)

The core correctness property: commands are matched to events by `(type, per_type_sequence)`.

```rust
use stateright::Model;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SequenceMatchingState {
    // Per-type sequence counters (mirrors ReplayEngine)
    task_seq: u32,
    timer_seq: u32,
    child_seq: u32,
    op_seq: u32,
    state_seq: u32,

    // Events available for replay
    task_events: Vec<String>,      // task types at each index
    timer_events: Vec<String>,     // timer IDs at each index
    child_events: Vec<String>,     // child names at each index

    // Commands issued so far
    commands_issued: Vec<Command>,

    // Violation detected?
    violation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    ScheduleTask { task_type: String },
    StartTimer { timer_id: String },
    ScheduleChild { name: String },
}

pub struct SequenceMatchingModel {
    task_events: Vec<String>,
    timer_events: Vec<String>,
    child_events: Vec<String>,
    possible_task_types: Vec<String>,
    possible_timer_ids: Vec<String>,
    possible_child_names: Vec<String>,
    max_commands: usize,
}

impl Model for SequenceMatchingModel {
    type State = SequenceMatchingState;
    type Action = Command;

    fn init_states(&self) -> Vec<Self::State> {
        vec![SequenceMatchingState {
            task_seq: 0,
            timer_seq: 0,
            child_seq: 0,
            op_seq: 0,
            state_seq: 0,
            task_events: self.task_events.clone(),
            timer_events: self.timer_events.clone(),
            child_events: self.child_events.clone(),
            commands_issued: vec![],
            violation: None,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.violation.is_some() {
            return; // Stop exploring after violation
        }
        if state.commands_issued.len() >= self.max_commands {
            return;
        }

        for task_type in &self.possible_task_types {
            actions.push(Command::ScheduleTask { task_type: task_type.clone() });
        }
        for timer_id in &self.possible_timer_ids {
            actions.push(Command::StartTimer { timer_id: timer_id.clone() });
        }
        for name in &self.possible_child_names {
            actions.push(Command::ScheduleChild { name: name.clone() });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        next.commands_issued.push(action.clone());

        match &action {
            Command::ScheduleTask { task_type } => {
                let seq = next.task_seq as usize;
                next.task_seq += 1;

                if seq < next.task_events.len() {
                    let expected = &next.task_events[seq];
                    if task_type != expected {
                        next.violation = Some(format!(
                            "TaskTypeMismatch at seq {}: expected '{}', got '{}'",
                            seq, expected, task_type
                        ));
                    }
                }
                // Beyond history = extension (allowed)
            }
            Command::StartTimer { timer_id } => {
                let seq = next.timer_seq as usize;
                next.timer_seq += 1;

                if seq < next.timer_events.len() {
                    let expected = &next.timer_events[seq];
                    if timer_id != expected {
                        next.violation = Some(format!(
                            "TimerIdMismatch at seq {}: expected '{}', got '{}'",
                            seq, expected, timer_id
                        ));
                    }
                }
            }
            Command::ScheduleChild { name } => {
                let seq = next.child_seq as usize;
                next.child_seq += 1;

                if seq < next.child_events.len() {
                    let expected = &next.child_events[seq];
                    if name != expected {
                        next.violation = Some(format!(
                            "ChildNameMismatch at seq {}: expected '{}', got '{}'",
                            seq, expected, name
                        ));
                    }
                }
            }
        }

        Some(next)
    }

    fn properties(&self) -> Vec<stateright::Property<Self>> {
        vec![
            // Core property: matching commands don't cause violations
            stateright::Property::always("matching commands pass", |model, state| {
                let all_commands_match = state.commands_issued.iter()
                    .enumerate()
                    .all(|(_, cmd)| {
                        match cmd {
                            Command::ScheduleTask { task_type } => {
                                // Check if this matches the corresponding event
                                let seq = state.commands_issued.iter()
                                    .filter(|c| matches!(c, Command::ScheduleTask { .. }))
                                    .position(|c| c == cmd)
                                    .unwrap_or(0);
                                seq >= model.task_events.len() ||
                                    &model.task_events[seq] == task_type
                            }
                            _ => true // Simplified
                        }
                    });

                if all_commands_match {
                    state.violation.is_none()
                } else {
                    true // Violation expected
                }
            }),

            // Extension is allowed
            stateright::Property::sometimes("can extend beyond history", |model, state| {
                state.commands_issued.len() > model.task_events.len() +
                    model.timer_events.len() + model.child_events.len() &&
                state.violation.is_none()
            }),

            // Violations are detected
            stateright::Property::sometimes("violations are detected", |_, state| {
                state.violation.is_some()
            }),
        ]
    }
}
```

### Model 2: join_all Semantics (Priority: P1)

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JoinAllState {
    pending: Vec<usize>,           // Indices of pending futures
    results: Vec<Option<Result>>,  // Results so far (None = pending)
    resolved: Option<JoinAllResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Result {
    Ok(String),
    Err(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinAllResult {
    AllOk(Vec<String>),
    FirstError(usize, String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinAllAction {
    Complete { index: usize, result: Result },
}

pub struct JoinAllModel {
    pub num_futures: usize,
}

impl Model for JoinAllModel {
    type State = JoinAllState;
    type Action = JoinAllAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![JoinAllState {
            pending: (0..self.num_futures).collect(),
            results: vec![None; self.num_futures],
            resolved: None,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        if state.resolved.is_some() {
            return;
        }

        for &index in &state.pending {
            actions.push(JoinAllAction::Complete {
                index,
                result: Result::Ok(format!("ok-{}", index)),
            });
            actions.push(JoinAllAction::Complete {
                index,
                result: Result::Err(format!("err-{}", index)),
            });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let JoinAllAction::Complete { index, result } = action;
        let mut next = state.clone();

        next.pending.retain(|&i| i != index);
        next.results[index] = Some(result.clone());

        // Fail fast on first error
        if let Result::Err(msg) = &result {
            next.resolved = Some(JoinAllResult::FirstError(index, msg.clone()));
        } else if next.pending.is_empty() {
            // All completed successfully
            let values: Vec<_> = next.results.iter()
                .map(|r| match r {
                    Some(Result::Ok(v)) => v.clone(),
                    _ => unreachable!(),
                })
                .collect();
            next.resolved = Some(JoinAllResult::AllOk(values));
        }

        Some(next)
    }

    fn properties(&self) -> Vec<stateright::Property<Self>> {
        vec![
            // Fail fast: error resolves immediately
            stateright::Property::always("fail fast", |_, state| {
                let has_error = state.results.iter().any(|r| {
                    matches!(r, Some(Result::Err(_)))
                });
                if has_error {
                    matches!(state.resolved, Some(JoinAllResult::FirstError(..)))
                } else {
                    true
                }
            }),

            // All ok only when all complete
            stateright::Property::always("all ok requires all complete", |_, state| {
                if matches!(state.resolved, Some(JoinAllResult::AllOk(_))) {
                    state.pending.is_empty()
                } else {
                    true
                }
            }),

            // Eventually resolves
            stateright::Property::eventually("resolves", |_, state| {
                state.resolved.is_some()
            }),
        ]
    }
}
```

### Running the Models

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_sequence_matching() {
        let model = SequenceMatchingModel {
            task_events: vec!["task-A".into(), "task-B".into()],
            timer_events: vec!["timer-1".into()],
            child_events: vec![],
            possible_task_types: vec!["task-A".into(), "task-B".into(), "task-C".into()],
            possible_timer_ids: vec!["timer-1".into(), "timer-2".into()],
            possible_child_names: vec![],
            max_commands: 4,
        };

        let checker = model.checker()
            .threads(num_cpus::get())
            .spawn_bfs()
            .join();

        checker.assert_properties();

        println!("States explored: {}", checker.unique_state_count());
    }

    #[test]
    fn verify_join_all() {
        for n in 2..=4 {
            let model = JoinAllModel { num_futures: n };

            model.checker()
                .threads(num_cpus::get())
                .spawn_bfs()
                .join()
                .assert_properties();
        }
    }
}
```

## What This Does NOT Cover

1. **Actual replay engine implementation bugs**: The model verifies the *specification*, not the *implementation*. We still need tests that exercise the real code.

2. **Performance**: Model checking says nothing about efficiency.

3. **Edge cases not in the model**: If we forget to model a scenario, it won't be checked.

4. **Server-side bugs**: Event persistence, scheduling, work distribution.

## Evaluation Results (Phase 1 Complete)

Phase 1 has been completed. Here are the results:

### State Space Measurements

| Test Configuration | States Explored | Time |
|-------------------|-----------------|------|
| Small (2 task events, 1 timer, max 4 commands) | 121 | ~2.5ms |
| Medium (3 task events, 2 timers, 1 child, max 5 commands) | 1,936 | ~17ms |
| Per-type independence | 22 | <1ms |
| Extension allowed | 25 | <1ms |
| Violations detected | 5 | <1ms |

### Verdict: Proceed to Phase 2

The state space is **tractable** and model checking is **fast**:
- Even the medium configuration explores < 2,000 states in < 20ms
- Well under the 10,000 state / 30 second targets
- Properties verify correctly (matching commands pass, extensions allowed)

### What Was Verified

1. **Matching commands never cause violations** - When commands match replay history, no determinism violation occurs
2. **Extension is allowed** - Commands beyond replay history (new commands) are allowed without violation
3. **Per-type sequences are independent** - Task sequence counter doesn't affect timer sequence counter

## Implementation Approach

### Phase 1: Evaluate (1 day)
- [x] Add stateright as dev-dependency
- [x] Implement SequenceMatchingModel
- [x] Run and verify it works
- [x] Measure: states explored, time taken

**Decision point**: ✅ Proceed - model checking is fast and effective.

### Phase 2: Core Models (2-3 days)
- [x] Refine SequenceMatchingModel with all command types
- [x] Add JoinAllModel and SelectModel
- [x] Verify models catch known bug patterns (via mutation testing)

### Phase 3: Integration (1 day)
- [x] Add CI workflow
- [x] Document how to run models locally

## Open Questions (Resolved)

1. **Should models live next to implementation or separately?**
   - **Resolution**: Separately in `sdk/tests/model/` - cleaner separation, easier to maintain

2. **How do we prevent model drift?**
   - **Resolution**: Model-implementation comparison tests (`bridge.rs`, `comparison.rs`) automatically detect drift by running same scenarios through both model and real ReplayEngine

3. **Is the ROI worth it?**
   - **Resolution**: Yes - fast verification (<20ms), mutation testing proves tests catch bugs, provides regression prevention

## Conclusion

### Final Results

All four correctness requirements from the Problem Statement have been verified:

| Requirement | Verification |
|-------------|--------------|
| **Replay must be deterministic** | `determinism_props.rs` (9 tests), `state_machine_props.rs` (8 tests) |
| **Per-type sequence matching** | `sequence_matching.rs` model, `comparison.rs` (435 paths tested) |
| **Parallel combinators** | `join_all.rs` (4 tests), `select.rs` (4 tests), `combinators.rs` (16 unit tests) |
| **Extension is allowed** | Model property verified, comparison tests confirm |

### Key Achievements

1. **State space is tractable**: Medium config explores 1,936 states in ~17ms
2. **Model matches implementation**: 435 command paths tested, all agree
3. **Tests catch real bugs**: Mutation testing killed 4 injected bugs (off-by-one, wrong counter, inverted comparison, no boundary check)
4. **Gap addressed**: Model-implementation comparison bridges specification to real code

### Test Coverage Summary

| Test Suite | Tests | Purpose |
|------------|-------|---------|
| `sdk/tests/model/` | 39 tests | Stateright model checking |
| `sdk/tests/unit/determinism_props.rs` | 9 tests | Workflow determinism |
| `sdk/tests/unit/mutations.rs` | 4 tests | Mutation testing |
| `sdk/tests/unit/state_machine_props.rs` | 8 tests | Property-based tests |
| `sdk/src/workflow/combinators.rs` | 16 tests | Combinator unit tests |

### Lessons Learned

- **Model checking supplements, not replaces, other tests**: Models verify specifications; comparison tests verify implementation matches
- **Mutation testing validates test effectiveness**: Injecting known bugs proves tests can catch real issues
- **Bridging model to implementation is essential**: The original design noted models don't catch implementation bugs - this was solved by `bridge.rs` and `comparison.rs`

### Running the Tests

```bash
# Run all model checking tests
cargo test --test model -p flovyn-sdk

# Run determinism property tests
cargo test --test unit -p flovyn-sdk --features testing -- determinism_props

# Run mutation tests
cargo test --test unit -p flovyn-sdk --features testing -- mutations
```
