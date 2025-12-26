# SDK and FFI Core Consolidation - Implementation Plan

## Overview

This plan implements the consolidation opportunities identified in [SDK and FFI Module Consolidation Analysis](../design/sdk-ffi-core-consolidation.md).

**Goal**: Eliminate code duplication between `flovyn-sdk`, `flovyn-ffi`, and `flovyn-core` by:
1. Extracting a shared `ReplayEngine` to core (HIGH VALUE - ~400 LOC reduction)
2. Consolidating `SeededRandom` and `DeterministicRandom` usage

**Constraint**: After each step, all tests must pass. No public API changes.

## Current State Analysis

### High-Level Duplication

Both SDK and FFI implement nearly identical replay logic:

| Component | SDK (`context_impl.rs`) | FFI (`context.rs`) |
|-----------|-------------------------|---------------------|
| Event pre-filtering | Lines 238-279 | Lines 295-333 |
| Per-type sequence counters | 6 counters | 6 counters |
| Terminal event lookup | `find_terminal_*` methods | Eager result maps |
| State management | `state` HashMap | `state` HashMap |
| Operation cache | `operation_cache` HashMap | `operation_cache` HashMap |
| SeededRandom | Lines 23-77 | Lines 191-215 |

### Detailed Duplication Table

| Duplication | SDK Location | FFI Location | Core Location | Lines |
|-------------|--------------|--------------|---------------|-------|
| `SeededRandom` | `context_impl.rs:23-77` | `context.rs:191-215` | `execution.rs:115-168` | ~150 total |
| `DeterministicRandom` trait | `context.rs:31-44` | N/A | `execution.rs:93-109` | ~26 total |
| Event pre-filtering | `context_impl.rs:238-279` | `context.rs:295-333` | N/A | ~80 total |
| Per-type counters | `context_impl.rs:303-314` | `context.rs:248-254` | N/A | ~24 total |
| Terminal lookup | `context_impl.rs:333-432` | `context.rs:381-479` | Partial in `EventLookup` | ~200 total |
| Cache/state building | `context_impl.rs:209-236` | `context.rs:339-342` | `execution.rs:283-324` | ~50 total |

**Core already provides (partial):**
- `SeededRandom` struct with xorshift64 algorithm
- `DeterministicRandom` trait
- `EventLookup` utilities for finding terminal events
- `build_operation_cache()` and `build_initial_state()` functions

## Implementation Strategy

The refactoring follows a **phased approach**:

1. **Phase 1**: Quick wins - Consolidate `SeededRandom` and `DeterministicRandom`
2. **Phase 2**: Extract `ReplayEngine` to core (main consolidation)
3. **Phase 3**: Update SDK to use `ReplayEngine`
4. **Phase 4**: Update FFI to use `ReplayEngine`
5. **Phase 5**: Verification and documentation

## TODO List

### Phase 1: Quick Wins - SeededRandom and DeterministicRandom

#### Step 1.1: Re-export DeterministicRandom from Core

The SDK currently defines its own `DeterministicRandom` trait in `context.rs`. Replace with re-export from core.

- [ ] Update `sdk/src/workflow/context.rs`:
  - Remove local `DeterministicRandom` trait definition (lines 31-44)
  - Add `pub use flovyn_core::workflow::execution::DeterministicRandom;`
- [ ] Verify: `cargo build -p flovyn-sdk`
- [ ] Verify: `cargo test -p flovyn-sdk`

#### Step 1.2: Replace SDK SeededRandom with Core Implementation

- [ ] Update `sdk/src/workflow/context_impl.rs`:
  - Remove local `SeededRandom` struct definition (lines 23-77)
  - Add import: `use flovyn_core::workflow::execution::SeededRandom;`
- [ ] Verify: `cargo build -p flovyn-sdk`
- [ ] Verify: `cargo test -p flovyn-sdk`

#### Step 1.3: Replace FFI SeededRandom with Core Implementation

- [ ] Update `ffi/src/context.rs`:
  - Remove local `SeededRandom` struct (lines 191-215)
  - Add import: `use flovyn_core::workflow::execution::SeededRandom;`
- [ ] Verify: `cargo build -p flovyn-ffi`
- [ ] Verify: `cargo test -p flovyn-ffi`

#### Step 1.4: Phase 1 Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`

### Phase 2: Extract ReplayEngine to Core (HIGH VALUE)

This is the main consolidation opportunity. Extract shared replay logic into a `ReplayEngine` struct.

#### Step 2.1: Design ReplayEngine API

Create `core/src/workflow/replay_engine.rs` with the following structure:

```rust
/// Language-agnostic replay engine for workflow execution.
///
/// Handles event pre-filtering, sequence management, and terminal event lookup.
/// Used by both Rust SDK and FFI layer.
pub struct ReplayEngine {
    // Pre-filtered event lists by type
    task_events: Vec<ReplayEvent>,
    timer_events: Vec<ReplayEvent>,
    promise_events: Vec<ReplayEvent>,
    child_workflow_events: Vec<ReplayEvent>,
    operation_events: Vec<ReplayEvent>,
    state_events: Vec<ReplayEvent>,

    // All events for terminal lookup
    all_events: Vec<ReplayEvent>,

    // Per-type sequence counters (atomic for thread-safety)
    next_task_seq: AtomicU32,
    next_timer_seq: AtomicU32,
    next_promise_seq: AtomicU32,
    next_child_workflow_seq: AtomicU32,
    next_operation_seq: AtomicU32,
    next_state_seq: AtomicU32,

    // Caches
    operation_cache: HashMap<String, Value>,
    state: RwLock<HashMap<String, Value>>,
}

impl ReplayEngine {
    /// Create a new ReplayEngine from replay events.
    pub fn new(events: Vec<ReplayEvent>) -> Self;

    // === Sequence Management ===
    /// Get next task sequence number and increment.
    pub fn next_task_seq(&self) -> u32;
    pub fn next_timer_seq(&self) -> u32;
    pub fn next_promise_seq(&self) -> u32;
    pub fn next_child_workflow_seq(&self) -> u32;
    pub fn next_operation_seq(&self) -> u32;
    pub fn next_state_seq(&self) -> u32;

    // === Event Lookup (for replay validation) ===
    /// Get the task event at the given sequence index (if replaying).
    pub fn get_task_event(&self, seq: u32) -> Option<&ReplayEvent>;
    pub fn get_timer_event(&self, seq: u32) -> Option<&ReplayEvent>;
    pub fn get_promise_event(&self, seq: u32) -> Option<&ReplayEvent>;
    pub fn get_child_workflow_event(&self, seq: u32) -> Option<&ReplayEvent>;
    pub fn get_operation_event(&self, seq: u32) -> Option<&ReplayEvent>;
    pub fn get_state_event(&self, seq: u32) -> Option<&ReplayEvent>;

    /// Check if currently replaying for the given event type.
    pub fn is_replaying_task(&self, seq: u32) -> bool;
    pub fn is_replaying_timer(&self, seq: u32) -> bool;
    // ... etc

    // === Terminal Event Lookup ===
    /// Find terminal event for a task by execution ID.
    pub fn find_terminal_task_event(&self, task_id: &str) -> Option<&ReplayEvent>;
    pub fn find_terminal_timer_event(&self, timer_id: &str) -> Option<&ReplayEvent>;
    pub fn find_terminal_promise_event(&self, promise_name: &str) -> Option<&ReplayEvent>;
    pub fn find_terminal_child_workflow_event(&self, name: &str) -> Option<&ReplayEvent>;

    // === State Management ===
    pub fn get_state(&self, key: &str) -> Option<Value>;
    pub fn set_state(&mut self, key: &str, value: Value);
    pub fn clear_state(&mut self, key: &str);
    pub fn state_keys(&self) -> Vec<String>;

    // === Operation Cache ===
    pub fn get_cached_operation(&self, name: &str) -> Option<&Value>;

    // === Accessors ===
    pub fn task_event_count(&self) -> usize;
    pub fn timer_event_count(&self) -> usize;
    // ... etc
}
```

- [ ] Create `core/src/workflow/replay_engine.rs` with struct definition
- [ ] Implement `ReplayEngine::new()` with event pre-filtering
- [ ] Implement sequence management methods
- [ ] Implement event lookup methods
- [ ] Implement terminal event lookup (delegate to `EventLookup`)
- [ ] Implement state management methods
- [ ] Update `core/src/workflow/mod.rs` to export `ReplayEngine`
- [ ] Verify: `cargo build -p flovyn-core`

#### Step 2.2: Add ReplayEngine Tests

Write comprehensive unit tests following the test-first approach.

**Event Filtering Tests:**
- [ ] `test_new_filters_task_events` - Only TaskScheduled events in task_events list
- [ ] `test_new_filters_timer_events` - Only TimerStarted events in timer_events list
- [ ] `test_new_filters_promise_events` - Only PromiseCreated events in promise_events list
- [ ] `test_new_filters_child_workflow_events` - Only ChildWorkflowInitiated events
- [ ] `test_new_filters_operation_events` - Only OperationCompleted events
- [ ] `test_new_filters_state_events` - Both StateSet and StateCleared events
- [ ] `test_new_preserves_event_order` - Events maintain original sequence order within type
- [ ] `test_new_with_empty_events` - Empty input produces empty lists

**Sequence Management Tests:**
- [ ] `test_next_task_seq_increments` - Counter starts at 0, increments on each call
- [ ] `test_next_timer_seq_increments` - Same for timers
- [ ] `test_next_promise_seq_increments` - Same for promises
- [ ] `test_sequence_counters_independent` - Each type has separate counter
- [ ] `test_sequence_counters_thread_safe` - Concurrent increments are safe (use multiple threads)

**Event Lookup Tests:**
- [ ] `test_get_task_event_returns_correct_event` - Returns event at sequence index
- [ ] `test_get_task_event_returns_none_beyond_count` - Returns None if seq >= count
- [ ] `test_get_timer_event_returns_correct_event` - Same for timers
- [ ] `test_is_replaying_task_true_when_seq_in_range` - seq < task_event_count
- [ ] `test_is_replaying_task_false_when_seq_beyond` - seq >= task_event_count

**Terminal Event Lookup Tests:**
- [ ] `test_find_terminal_task_event_completed` - Finds TaskCompleted by taskExecutionId
- [ ] `test_find_terminal_task_event_failed` - Finds TaskFailed by taskExecutionId
- [ ] `test_find_terminal_task_event_latest` - Returns latest if multiple (retries)
- [ ] `test_find_terminal_task_event_not_found` - Returns None if no terminal event
- [ ] `test_find_terminal_timer_event_fired` - Finds TimerFired by timerId
- [ ] `test_find_terminal_timer_event_cancelled` - Finds TimerCancelled by timerId
- [ ] `test_find_terminal_promise_event_resolved` - Finds PromiseResolved by promiseName
- [ ] `test_find_terminal_promise_event_rejected` - Finds PromiseRejected
- [ ] `test_find_terminal_promise_event_timeout` - Finds PromiseTimeout
- [ ] `test_find_terminal_child_workflow_completed` - Finds ChildWorkflowCompleted
- [ ] `test_find_terminal_child_workflow_failed` - Finds ChildWorkflowFailed

**State Management Tests:**
- [ ] `test_initial_state_from_events` - StateSet events populate initial state
- [ ] `test_initial_state_cleared_events` - StateCleared removes from initial state
- [ ] `test_initial_state_latest_wins` - Later StateSet overwrites earlier
- [ ] `test_get_state_returns_value` - Returns Some(value) if key exists
- [ ] `test_get_state_returns_none` - Returns None if key doesn't exist
- [ ] `test_set_state_adds_new_key` - Adds key-value pair
- [ ] `test_set_state_overwrites_existing` - Overwrites existing key
- [ ] `test_clear_state_removes_key` - Removes key from state
- [ ] `test_clear_state_nonexistent_noop` - No error if key doesn't exist
- [ ] `test_state_keys_returns_all_keys` - Returns all current keys

**Operation Cache Tests:**
- [ ] `test_operation_cache_from_events` - OperationCompleted populates cache
- [ ] `test_get_cached_operation_returns_value` - Returns cached result
- [ ] `test_get_cached_operation_returns_none` - Returns None if not cached
- [ ] `test_operation_cache_latest_wins` - Later OperationCompleted overwrites

**Integration Tests:**
- [ ] `test_replay_scenario_task_completed` - Full replay of task schedule → complete
- [ ] `test_replay_scenario_task_pending` - Replay stops when no terminal event
- [ ] `test_replay_scenario_mixed_events` - Multiple event types interleaved
- [ ] `test_replay_scenario_determinism` - Same events produce same sequence behavior

- [ ] Verify: `cargo test -p flovyn-core`

#### Step 2.3: Phase 2 Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`

### Phase 3: Update SDK to Use ReplayEngine

#### Step 3.1: Refactor WorkflowContextImpl

- [ ] Update `sdk/src/workflow/context_impl.rs`:
  - Add import: `use flovyn_core::workflow::ReplayEngine;`
  - Replace inline event lists with `ReplayEngine`:
    ```rust
    pub struct WorkflowContextImpl<R: CommandRecorder> {
        replay_engine: ReplayEngine,  // NEW
        recorder: RwLock<R>,
        random: SeededRandom,         // From core
        // Remove: task_events, timer_events, etc.
        // Remove: next_task_seq, next_timer_seq, etc.
        // Remove: operation_cache, state (now in ReplayEngine)
        // ... keep other fields
    }
    ```
- [ ] Update constructor to use `ReplayEngine::new(existing_events)`
- [ ] Remove inline cache/state building (now in `ReplayEngine`)
- [ ] Verify: `cargo build -p flovyn-sdk`

#### Step 3.2: Update SDK Context Methods

- [ ] Update `schedule_task_raw()` to use `replay_engine.next_task_seq()` and `replay_engine.get_task_event()`
- [ ] Update `schedule_timer()` to use `replay_engine` methods
- [ ] Update `create_promise()` to use `replay_engine` methods
- [ ] Update `schedule_child_workflow()` to use `replay_engine` methods
- [ ] Update `run_operation()` to use `replay_engine` methods
- [ ] Update state methods to delegate to `replay_engine`
- [ ] Remove private `find_terminal_*` methods (use `replay_engine` instead)
- [ ] Verify: `cargo test -p flovyn-sdk`

#### Step 3.3: Phase 3 Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] All examples compile and run

### Phase 4: Update FFI to Use ReplayEngine

#### Step 4.1: Add Event Conversion

FFI uses `FfiReplayEvent` (uniffi type) but `ReplayEngine` uses `ReplayEvent` (core type).
Add conversion at the FFI boundary.

- [ ] Update `ffi/src/types.rs`:
  - Add `impl From<FfiReplayEvent> for ReplayEvent` conversion
  - Or add helper function `fn convert_events(events: Vec<FfiReplayEvent>) -> Vec<ReplayEvent>`
- [ ] Verify: `cargo build -p flovyn-ffi`

#### Step 4.2: Refactor FfiWorkflowContext

- [ ] Update `ffi/src/context.rs`:
  - Add import: `use flovyn_core::workflow::ReplayEngine;`
  - Replace inline event lists with `ReplayEngine`:
    ```rust
    pub struct FfiWorkflowContext {
        replay_engine: ReplayEngine,  // NEW
        random: SeededRandom,         // From core
        commands: Mutex<Vec<FfiWorkflowCommand>>,
        // Remove: task_events, promise_events, etc.
        // Remove: next_task_seq, next_promise_seq, etc.
        // Remove: completed_tasks, resolved_promises, etc. (eager maps)
        // ... keep FFI-specific fields
    }
    ```
- [ ] Update constructor to convert events and use `ReplayEngine::new()`
- [ ] Verify: `cargo build -p flovyn-ffi`

#### Step 4.3: Update FFI Context Methods

Note: FFI currently uses **eager result maps** (e.g., `completed_tasks`). With `ReplayEngine`,
we switch to **lazy lookup** (call `find_terminal_task_event()` when needed). This is a behavioral
change but should be equivalent.

- [ ] Update `schedule_task()` to use `replay_engine.next_task_seq()` and lazy terminal lookup
- [ ] Update `create_promise()` to use `replay_engine` methods
- [ ] Update `start_timer()` to use `replay_engine` methods
- [ ] Update `schedule_child_workflow()` to use `replay_engine` methods
- [ ] Update `run_operation()` to use `replay_engine` methods
- [ ] Remove eager result map building methods (`build_task_results()`, etc.)
- [ ] Verify: `cargo test -p flovyn-ffi`

#### Step 4.4: Phase 4 Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] FFI bindings generate correctly

### Phase 5: Final Verification and Documentation

#### Step 5.1: Run Full Test Suite

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo doc --no-deps --workspace`

#### Step 5.2: E2E Tests

- [ ] Run E2E tests: `cargo test --test e2e -p flovyn-sdk`
- [ ] Verify deterministic replay works correctly
- [ ] Verify random number sequences match between original and replay

#### Step 5.3: Kotlin SDK E2E Tests (if available)

- [ ] Run Kotlin SDK E2E tests to verify FFI changes
- [ ] Verify replay behavior is consistent

#### Step 5.4: Update Documentation

- [ ] Add doc comments to `ReplayEngine` explaining its purpose and usage
- [ ] Update `core/src/workflow/mod.rs` module docs
- [ ] Document that `DeterministicRandom`, `SeededRandom`, and `ReplayEngine` come from core

## Intentional Non-Changes

The following remain **intentionally separate** and should NOT be consolidated:

### FFI Command and Event Types

FFI must define its own `FfiWorkflowCommand` and `FfiEventType` for uniffi compatibility. These are structural requirements, not technical debt.

### FFI-to-Core Event Conversion

FFI uses `FfiReplayEvent` (uniffi-compatible) while core uses `ReplayEvent`. The conversion happens once at context creation. This is acceptable overhead for the consolidation benefit.

### ParsedEvent (FFI Internal Type)

FFI's `ParsedEvent` is removed in favor of core's `ReplayEvent`. The conversion cost is one-time at context creation.

## Expected Outcome

| Metric | Before | After |
|--------|--------|-------|
| `SeededRandom` copies | 3 | 1 (in core) |
| `DeterministicRandom` copies | 2 | 1 (in core) |
| Event pre-filtering implementations | 2 | 1 (in ReplayEngine) |
| Terminal event lookup implementations | 2 | 1 (in ReplayEngine) |
| State management implementations | 2 | 1 (in ReplayEngine) |
| Total LOC reduction | - | ~400-500 lines |
| Replay logic maintenance points | 3 | 1 |

## Architecture After Consolidation

```
┌─────────────────────────────────────────────────────────────────────┐
│  User Code (Rust)          │   User Code (Kotlin/Swift/Python)     │
└──────────────┬──────────────┴──────────────────┬────────────────────┘
               │                                  │
               ▼                                  ▼
┌──────────────────────────┐       ┌───────────────────────────────────┐
│     flovyn-sdk           │       │          flovyn-ffi                │
│                          │       │                                    │
│  - WorkflowContextImpl   │       │  - FfiWorkflowContext              │
│    (uses ReplayEngine)   │       │    (uses ReplayEngine)             │
│  - Async futures         │       │  - uniffi bindings                 │
│  - Rust traits           │       │  - FfiWorkflowCommand              │
└──────────────┬───────────┘       └──────────────┬────────────────────┘
               │                                   │
               └───────────────┬───────────────────┘
                               ▼
               ┌───────────────────────────────────┐
               │          flovyn-core              │
               │                                   │
               │  - ReplayEngine         [NEW]     │
               │  - SeededRandom                   │
               │  - DeterministicRandom trait      │
               │  - EventLookup                    │
               │  - ReplayEvent, WorkflowCommand   │
               │  - gRPC clients                   │
               └───────────────────────────────────┘
```

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Breaking deterministic replay | Run E2E tests verifying replay produces same results |
| FFI event conversion overhead | One-time cost at context creation; acceptable |
| Behavioral change (eager → lazy in FFI) | Terminal lookup is semantically equivalent; test thoroughly |
| uniffi compatibility with core types | FFI converts at boundary; core types not exposed via uniffi |
| Complex refactoring in SDK | Phase 3 is incremental; verify after each method update |

## Rollback Strategy

If issues arise during implementation:

1. **Phase 1** is low-risk; can be completed independently
2. **Phase 2** adds `ReplayEngine` without changing SDK/FFI; safe to pause
3. **Phase 3** (SDK refactor) can be reverted by restoring `context_impl.rs`
4. **Phase 4** (FFI refactor) can be reverted by restoring `context.rs`

Each phase is independently testable and deployable.

## References

- [SDK and FFI Module Consolidation Analysis](../design/sdk-ffi-core-consolidation.md)
- [Phase 1: Extract Core](./phase1-extract-core.md)
- [Core SDK in Rust for Multiple Languages](../design/core-sdk-in-rust-for-multiple-language.md)
