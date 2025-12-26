# Implementation Plan: Sequence-Based Replay

## Overview

Implement position-based command-event matching to replace the current consumed-set approach. This is a breaking change that will improve determinism detection and debugging.

**Design Documents:**
- [Sequence-Based Replay Design](../design/sequence-based-replay.md)
- [Sequence-Based Replay Testing](../design/sequence-based-replay-testing.md)

---

## Phase 0: Design Decision - Server as Source of Truth

### 0.1 Current Behavior

Server ignores SDK's `command.sequence_number` and auto-increments its own counter.

**Location:** `/flovyn-server/src/api/grpc/workflow_dispatch.rs` (lines 965-976)

```rust
let mut next_sequence = (existing_events.len() as i32) + 1;
```

### 0.2 Adopted Approach: Server is Source of Truth

**The server's sequence numbers are authoritative.** The SDK matches commands to events by **position in the event list**, not by predicting sequence numbers.

**How it works:**

1. SDK sends commands (sequence_number in command is ignored by server)
2. Server assigns sequence_number when creating each event
3. When workflow resumes, SDK receives events with server-assigned sequence_numbers
4. During replay, SDK matches the Nth command call to the Nth matching event type

**Example:**
```
Events from server:
  [0] WorkflowStarted{seq=1}
  [1] TaskScheduled{seq=2, taskType="A"}
  [2] TaskCompleted{seq=3}
  [3] TaskScheduled{seq=4, taskType="A"}
  [4] TaskCompleted{seq=5}

SDK replay:
  1st schedule("A") → matches events[1] (TaskScheduled at index 1)
  2nd schedule("A") → matches events[3] (TaskScheduled at index 3)
```

**Key change in SDK:**

Use **per-type replay cursors** to support parallel command execution:

```rust
struct WorkflowContextImpl {
    // Pre-filtered event lists by type for O(1) lookup
    task_events: Vec<ReplayEvent>,           // TaskScheduled events only
    child_workflow_events: Vec<ReplayEvent>, // ChildWorkflowInitiated events only
    timer_events: Vec<ReplayEvent>,          // TimerStarted events only
    operation_events: Vec<ReplayEvent>,      // OperationCompleted events only
    promise_events: Vec<ReplayEvent>,        // PromiseCreated events only
    state_events: Vec<ReplayEvent>,          // StateSet/StateCleared events only

    // Per-type cursors for replay matching
    next_task_seq: AtomicU32,
    next_child_workflow_seq: AtomicU32,
    next_timer_seq: AtomicU32,
    next_operation_seq: AtomicU32,
    next_promise_seq: AtomicU32,
    next_state_seq: AtomicU32,
}

impl WorkflowContextImpl {
    fn new(all_events: Vec<ReplayEvent>) -> Self {
        Self {
            task_events: all_events.iter()
                .filter(|e| e.event_type() == EventType::TaskScheduled)
                .cloned().collect(),
            timer_events: all_events.iter()
                .filter(|e| e.event_type() == EventType::TimerStarted)
                .cloned().collect(),
            // ... filter other event types
            next_task_seq: AtomicU32::new(0),
            next_timer_seq: AtomicU32::new(0),
            // ... initialize other counters
        }
    }
}
```

**Matching algorithm:**

```rust
fn schedule_raw(&self, task_type: &str, input: Value) -> Result<Value> {
    // Get per-type sequence and increment
    let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst) as usize;

    if task_seq < self.task_events.len() {
        let event = &self.task_events[task_seq];

        // Validate task type matches (event type already correct by filtering)
        let event_task_type = event.get_string("taskType");
        if event_task_type != Some(task_type) {
            return Err(DeterminismViolation {
                message: format!(
                    "Task type mismatch at Task({}): expected '{}', got '{}'",
                    task_seq, task_type, event_task_type.unwrap_or("none")
                )
            });
        }

        // Find terminal event by taskExecutionId
        let task_id = event.get_string("taskExecutionId").unwrap();
        let terminal = self.find_terminal_task_event(task_id);
        // ...
    } else {
        // New command - submit to server
    }
}
```

**Why per-type cursors:**
- Supports parallel command execution (e.g., `awaitAll(schedule("A"), schedule("B"), sleep(1s))`)
- Commands are matched by creation order within their type, not global order
- Terminal events are matched by ID (taskExecutionId, timerId), not position
- See design document for detailed parallel execution examples

### 0.3 Why This Is Safe

1. **Single source of truth**: Server controls sequence numbers
2. **No prediction needed**: SDK doesn't guess what sequence to use
3. **Race-proof**: External events (TaskCompleted) don't break matching
4. **Simpler logic**: Match by position in filtered event list

### 0.4 No Server Change Needed

The current server behavior is correct. Only SDK needs to change its matching strategy.

---

## Phase 1: Core SDK Changes

### 1.1 Add Helper Methods to WorkflowContextImpl

Add `get_event_at_sequence()` and terminal event finders.

**File:** `sdk/src/workflow/context_impl.rs`

```rust
fn get_event_at_sequence(&self, seq: i32) -> Option<&ReplayEvent>
fn find_terminal_task_event(&self, task_execution_id: &str) -> Option<&ReplayEvent>
fn find_terminal_child_workflow_event(&self, name: &str) -> Option<&ReplayEvent>
```

### 1.2 Refactor Task Scheduling

Replace consumed-set matching with sequence-based matching in `schedule_with_options_raw()`.

**File:** `sdk/src/workflow/context_impl.rs`

### 1.3 Refactor Child Workflow Scheduling

Update `schedule_workflow_raw()` to use sequence-based matching with name/kind validation.

**File:** `sdk/src/workflow/context_impl.rs`

### 1.4 Remove Consumed ID Tracking

Remove `consumed_task_execution_ids: HashSet<String>` field and related imports.

**File:** `sdk/src/workflow/context_impl.rs`

### 1.5 Update Error Messages

Ensure `DeterminismViolation` errors include:
- Sequence number
- Expected value (from event)
- Actual value (from command)
- Field name

**File:** `sdk/src/error.rs`

---

## Phase 2: Unit Tests

### 2.1 Context Matching Tests

Add tests for sequence-based matching logic.

**File:** `sdk/src/workflow/context_impl.rs` (module tests)

Tests:
- `task_matches_at_correct_sequence`
- `task_type_mismatch_raises_determinism_violation`
- `multiple_tasks_same_type_match_by_sequence`
- `wrong_event_type_at_sequence_raises_violation`
- `child_workflow_matches_by_sequence_and_name`
- `child_workflow_name_mismatch_raises_violation`
- `child_workflow_kind_mismatch_raises_violation`
- `loop_extended_is_valid`
- `loop_shortened_behavior`
- `mixed_task_types_in_loop_must_match_order`
- `mixed_task_types_wrong_order_raises_violation`

### 2.2 Validator Tests

Add/update tests for `DeterminismValidator`.

**File:** `sdk/src/worker/determinism.rs` (module tests)

---

## Phase 3: TCK Corpus Tests

### 3.1 Create Determinism Corpus Files

**Directory:** `sdk/tests/shared/replay-corpus/`

Files to create:
- `determinism-task-loop.json` - Tasks in loop, successful replay
- `determinism-violation-task-type.json` - Task type mismatch
- `determinism-violation-task-order.json` - Task order changed
- `determinism-child-workflow-loop.json` - Child workflows in loop
- `determinism-violation-child-name.json` - Child workflow name mismatch
- `determinism-violation-child-kind.json` - Child workflow kind mismatch
- `determinism-mixed-commands.json` - Mixed tasks and child workflows

### 3.2 Create Determinism Corpus Test Runner

**File:** `sdk/tests/tck/determinism_corpus.rs`

Test runner that:
- Loads all `determinism-*.json` files
- Runs each scenario
- Validates expected behavior (success or violation)
- Checks error messages contain expected text

### 3.3 Update TCK mod.rs

**File:** `sdk/tests/tck/mod.rs`

---

## Phase 4: E2E Replay Tests

### 4.1 Create Replay Test Fixtures

**File:** `sdk/tests/e2e/fixtures/replay_workflows.rs`

Workflows:
- `TaskLoopWorkflow` - Schedules N tasks in a loop
- `ChildWorkflowLoopWorkflow` - Spawns N child workflows
- `MixedCommandWorkflow` - Tasks, timers, child workflows
- `OriginalWorkflow` / `ChangedWorkflow` - For violation testing

### 4.2 Create Replay Test Module

**File:** `sdk/tests/e2e/replay_tests.rs`

Tests:
- `test_e2e_task_loop_replay` - Execute + replay identical code
- `test_e2e_child_workflow_loop_replay`
- `test_e2e_determinism_violation_on_task_type_change`
- `test_e2e_determinism_violation_on_child_name_change`
- `test_e2e_workflow_extension_allowed`
- `test_e2e_server_reports_determinism_failure`

### 4.3 Add Replay Helper to Test Harness

**File:** `sdk/tests/e2e/harness.rs`

Add methods:
- `get_workflow_events(workflow_execution_id) -> Vec<ReplayEvent>`
- `replay_workflow<W>(events) -> Result<Value>`

### 4.4 Update E2E mod.rs

**File:** `sdk/tests/e2e/mod.rs`

---

## Phase 5: Documentation & Cleanup

### 5.1 Update CLAUDE.md

Add note about sequence-based replay if needed.

### 5.2 Remove Old Test Patterns

Remove any tests that rely on consumed-set behavior.

### 5.3 Verify All Tests Pass

```bash
cargo test --workspace
cargo test --test tck -p flovyn-sdk
FLOVYN_E2E_USE_DEV_INFRA=1 cargo test --test e2e -p flovyn-sdk -- --ignored
```

---

## TODO List

### Phase 0: Server Verification ✅
- [x] Check server code for `SubmitWorkflowCommandsRequest` handler
- [x] Verify server is source of truth for sequence numbers
- [x] No server change needed

### Phase 1: Core SDK Changes ✅
- [x] Add per-type event lists: `task_events`, `child_workflow_events`, `timer_events`, `operation_events`, `promise_events`, `state_events`
- [x] Add per-type counters: `next_task_seq`, `next_child_workflow_seq`, `next_timer_seq`, `next_operation_seq`, `next_promise_seq`, `next_state_seq`
- [x] Pre-filter events by type in `WorkflowContextImpl::new()`
- [x] Add `find_terminal_task_event()` helper
- [x] Add `find_terminal_child_workflow_event()` helper
- [x] Refactor `schedule_with_options_raw()` to use per-type cursor matching
- [x] Refactor `schedule_workflow_raw()` to use per-type cursor matching
- [x] Refactor `run()` to use per-type cursor matching
- [x] Refactor `sleep()` to use per-type cursor matching
- [x] Refactor `set()`, `clear()` to use per-type cursor matching
- [x] Refactor `create_promise()` to use per-type cursor matching
- [x] Remove `consumed_task_execution_ids` field
- [x] Update `DeterminismViolation` error messages with CommandID (e.g., "Task(0)", "Timer(1)")

### Phase 2: Unit Tests ✅

**Sequential execution tests:**
- [x] Add `task_type_mismatch_raises_determinism_violation` test (`test_sequence_based_task_type_mismatch_violation`)
- [x] Add `multiple_tasks_same_type_match_by_sequence` test (`test_multiple_tasks_same_type_each_consume_one_event`)
- [x] Add `child_workflow_name_mismatch_raises_violation` test (`test_sequence_based_child_workflow_name_mismatch_violation`)
- [x] Add `operation_name_mismatch_raises_violation` test (`test_sequence_based_operation_name_mismatch_violation`)
- [x] Add `timer_id_mismatch_raises_violation` test (`test_sequence_based_timer_id_mismatch_violation`)
- [x] Add `promise_name_mismatch_raises_violation` test (`test_sequence_based_promise_name_mismatch_violation`)
- [x] Add `state_key_mismatch_raises_violation` test (`test_sequence_based_state_key_mismatch_violation`)
- [x] Add `interleaved_operations_replay_correctly` test (`test_sequence_based_interleaved_operations_replay_correctly`)
- [x] Add `new_commands_beyond_replay_history` test (`test_sequence_based_new_commands_beyond_replay_history`)
- [x] Add `task_matches_at_correct_sequence` test (`test_sequence_based_task_matches_at_correct_sequence`)
- [x] Add `child_workflow_matches_by_sequence_and_name` test (`test_sequence_based_child_workflow_matches_by_sequence_and_name`)
- [x] Add `child_workflow_kind_mismatch_raises_violation` test (`test_sequence_based_child_workflow_kind_mismatch_violation`)
- [x] Add `loop_extended_is_valid` test (`test_sequence_based_loop_extended_is_valid`)
- [x] Add `loop_shortened_is_valid` test (`test_sequence_based_loop_shortened_is_valid`)
- [x] Add `mixed_task_types_in_loop_must_match_order` test (`test_sequence_based_mixed_task_types_in_loop_must_match_order`)
- [x] Add `mixed_task_types_wrong_order_raises_violation` test (`test_sequence_based_mixed_task_types_wrong_order_raises_violation`)

**Parallel execution tests:** (deferred - requires parallel execution support)
- [ ] Add `parallel_tasks_match_by_per_type_sequence` test
- [ ] Add `parallel_tasks_and_timer_independent_matching` test
- [ ] Add `parallel_completion_order_does_not_affect_replay` test
- [ ] Add `await_all_with_mixed_command_types` test

### Phase 3: TCK Corpus Tests ✅
- [x] Create `determinism-task-loop.json`
- [x] Create `determinism-violation-task-type.json`
- [x] Create `determinism-violation-task-order.json`
- [x] Create `determinism-child-workflow-loop.json`
- [x] Create `determinism-violation-child-name.json`
- [x] Create `determinism-violation-child-kind.json`
- [x] Create `determinism-mixed-commands.json`
- [x] Create `determinism-loop-shortened.json`
- [x] Create `sdk/tests/tck/determinism_corpus.rs` test runner
- [x] Update `sdk/tests/tck/mod.rs`

### Phase 4: E2E Replay Tests ✅

**Test fixtures:** ✅
- [x] Create `TaskLoopWorkflow` fixture
- [x] Create `ChildWorkflowLoopWorkflow` fixture
- [x] Create `MixedCommandWorkflow` fixture
- [x] Create `ProcessChildWorkflow` fixture
- [x] Create `OriginalTaskOrderWorkflow` / `ChangedTaskOrderWorkflow` fixtures

**Test harness:** ✅
- [x] Add `get_workflow_events()` to test harness
- [x] Add `create_replay_context()` to replay_utils
- [x] Add `to_replay_events()` helper functions

**Sequential execution E2E tests:** ✅
- [x] Add `test_e2e_task_loop_replay` test
- [x] Add `test_e2e_child_workflow_loop_replay` test
- [x] Add `test_e2e_determinism_violation_on_task_type_change` test
- [x] Add `test_e2e_determinism_violation_on_child_name_change` test
- [x] Add `test_e2e_workflow_extension_allowed` test
- [x] Add `test_e2e_mixed_commands_replay` test
- [x] Add `test_e2e_operation_name_mismatch` test

**Parallel execution E2E tests:** (deferred - requires parallel execution support)
- [ ] Add `test_e2e_parallel_tasks_replay` test
- [ ] Add `test_e2e_parallel_tasks_with_timer_replay` test
- [ ] Add `test_e2e_await_all_replay` test

- [x] Update `sdk/tests/e2e/mod.rs`

### Phase 5: Documentation & Cleanup (Partial ✅)
- [x] Verify all unit tests pass
- [x] Verify all TCK tests pass
- [ ] Verify all E2E tests pass
- [x] Remove any obsolete tests relying on consumed-set behavior

---

## Verification Checklist

After implementation:

- [x] `cargo build --workspace` succeeds
- [x] `cargo test --workspace` passes
- [x] `cargo test --test tck -p flovyn-sdk` passes
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] `cargo fmt --all -- --check` passes
- [ ] E2E tests pass with server running

---

## Estimated Effort

| Phase | Complexity | Description |
|-------|------------|-------------|
| Phase 1 | Medium | Core logic refactoring |
| Phase 2 | Medium | Unit test implementation |
| Phase 3 | Low | JSON corpus + test runner |
| Phase 4 | Medium | E2E infrastructure + tests |
| Phase 5 | Low | Cleanup and verification |
