# Implementation Plan: Parallel Execution Support

## Overview

Implement parallel execution support for workflow operations, enabling patterns like fan-out/fan-in, racing/timeout, and concurrent task scheduling.

**Design Document:**
- [Parallel Execution Design](../design/parallel-execution.md)

**Prerequisites:**
- [Sequence-Based Replay](../design/sequence-based-replay.md) (implemented)

---

## Phase 1: Future Types and CancellableFuture Trait

### 1.1 Define CancellableFuture Trait

**File:** `sdk/src/workflow/future.rs` (new file)

```rust
/// Trait for workflow futures that can be cancelled
pub trait CancellableFuture: Future {
    /// Cancel this operation. Returns immediately, cancellation is async.
    fn cancel(&self);

    /// Check if this future has been cancelled
    fn is_cancelled(&self) -> bool;
}
```

### 1.2 Create TaskFuture

**File:** `sdk/src/workflow/future.rs`

```rust
pub struct TaskFuture<T> {
    task_seq: u32,
    task_execution_id: String,
    context: Weak<WorkflowContextImpl>,
    cancelled: AtomicBool,
    error: Option<FlovynError>,
    _marker: PhantomData<T>,
}
```

Implement:
- `Future<Output = Result<T>>` - polls for task completion
- `CancellableFuture` - sends RequestCancelTask command

### 1.3 Create TimerFuture

**File:** `sdk/src/workflow/future.rs`

```rust
pub struct TimerFuture {
    timer_seq: u32,
    timer_id: String,
    context: Weak<WorkflowContextImpl>,
    cancelled: AtomicBool,
}
```

Timer cancellation is synchronous - immediately unblocks with `Cancelled` result.

### 1.4 Create ChildWorkflowFuture

**File:** `sdk/src/workflow/future.rs`

```rust
pub struct ChildWorkflowFuture<T> {
    child_workflow_seq: u32,
    child_execution_id: String,
    child_execution_name: String,
    context: Weak<WorkflowContextImpl>,
    cancelled: AtomicBool,
    _marker: PhantomData<T>,
}
```

### 1.5 Create PromiseFuture

**File:** `sdk/src/workflow/future.rs`

```rust
pub struct PromiseFuture<T> {
    promise_seq: u32,
    promise_id: String,
    context: Weak<WorkflowContextImpl>,
    _marker: PhantomData<T>,
}
```

Promises cannot be cancelled (require external signal).

### 1.6 Create OperationFuture

**File:** `sdk/src/workflow/future.rs`

```rust
pub struct OperationFuture<T> {
    operation_seq: u32,
    operation_name: String,
    context: Weak<WorkflowContextImpl>,
    _marker: PhantomData<T>,
}
```

Operations cannot be cancelled (result already computed or pending).

### 1.7 Export Future Types

**File:** `sdk/src/workflow/mod.rs`

Add `mod future;` and export all future types.

---

## Phase 2: Async Operation Methods

### 2.1 Add Async Methods to WorkflowContext Trait

**File:** `sdk/src/workflow/context.rs`

Add method signatures:

```rust
pub trait WorkflowContext {
    // ... existing methods ...

    /// Schedule a task asynchronously, returning a future
    fn schedule_async<T: TaskDefinition>(&self, input: T::Input) -> TaskFuture<T::Output>;

    /// Schedule a task asynchronously with options
    fn schedule_async_with_options<T: TaskDefinition>(
        &self,
        input: T::Input,
        options: ScheduleTaskOptions,
    ) -> TaskFuture<T::Output>;

    /// Schedule a task asynchronously (raw/untyped)
    fn schedule_async_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value>;

    /// Sleep asynchronously, returning a future
    fn sleep_async(&self, duration: Duration) -> TimerFuture;

    /// Schedule a child workflow asynchronously
    fn schedule_workflow_async<W: WorkflowDefinition>(
        &self,
        name: &str,
        input: W::Input,
    ) -> ChildWorkflowFuture<W::Output>;

    /// Schedule a child workflow asynchronously (raw/untyped)
    fn schedule_workflow_async_raw(
        &self,
        name: &str,
        kind: &str,
        input: Value,
    ) -> ChildWorkflowFuture<Value>;

    /// Create a promise asynchronously
    fn promise_async<T: DeserializeOwned>(&self, name: &str) -> PromiseFuture<T>;

    /// Create a promise asynchronously (raw/untyped)
    fn promise_async_raw(&self, name: &str) -> PromiseFuture<Value>;

    /// Run an operation asynchronously
    fn run_async<F, T>(&self, name: &str, f: F) -> OperationFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Serialize + DeserializeOwned + Send + 'static;
}
```

### 2.2 Implement Async Methods in WorkflowContextImpl

**File:** `sdk/src/workflow/context_impl.rs`

Implement each async method:

```rust
impl WorkflowContext for WorkflowContextImpl {
    fn schedule_async_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value> {
        let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

        // Check replay history
        if let Some(event) = self.task_events.get(task_seq as usize) {
            let event_task_type = event.get_string("taskType").unwrap_or_default();
            if event_task_type != task_type {
                return TaskFuture::with_error(FlovynError::DeterminismViolation {
                    message: format!(
                        "Task type mismatch at Task({}): expected '{}', got '{}'",
                        task_seq, task_type, event_task_type
                    ),
                });
            }
            let task_execution_id = event.get_string("taskExecutionId").unwrap();
            return TaskFuture::from_replay(task_seq, task_execution_id, self.weak_self());
        }

        // New command
        let task_execution_id = self.generate_task_execution_id();
        self.record_command(WorkflowCommand::ScheduleTask {
            sequence_number: self.next_global_sequence(),
            task_type: task_type.to_string(),
            task_execution_id: task_execution_id.clone(),
            input,
            priority_seconds: 0,
        }).unwrap();

        TaskFuture::new(task_seq, task_execution_id, self.weak_self())
    }

    // ... implement other async methods similarly
}
```

### 2.3 Context Internal Helpers

**File:** `sdk/src/workflow/context_impl.rs`

Add helper methods:

```rust
impl WorkflowContextImpl {
    /// Get a weak reference to self for futures
    fn weak_self(&self) -> Weak<Self>;

    /// Unblock a timer (for synchronous cancellation)
    fn unblock_timer(&self, timer_id: &str, result: TimerResult);

    /// Check if a task is completed
    fn is_task_completed(&self, task_execution_id: &str) -> Option<Result<Value>>;

    /// Check if a timer has fired
    fn is_timer_fired(&self, timer_id: &str) -> Option<Result<()>>;

    /// Check if a child workflow is completed
    fn is_child_workflow_completed(&self, name: &str) -> Option<Result<Value>>;

    /// Check if a promise is resolved
    fn is_promise_resolved(&self, promise_id: &str) -> Option<Result<Value>>;
}
```

---

## Phase 3: Cancellation Commands

### 3.1 Add Cancellation Command Variants

**File:** `sdk/src/workflow/command.rs`

```rust
pub enum WorkflowCommand {
    // ... existing commands ...

    /// Request cancellation of a scheduled task
    RequestCancelTask {
        sequence_number: i32,
        task_execution_id: String,
    },

    /// Cancel a timer
    CancelTimer {
        sequence_number: i32,
        timer_id: String,
    },

    /// Request cancellation of a child workflow
    RequestCancelChildWorkflow {
        sequence_number: i32,
        child_execution_id: String,
    },
}
```

### 3.2 Add Cancellation Event Types

**File:** `sdk/src/workflow/event.rs`

Ensure these event types are defined:
- `TaskCancelled`
- `TimerCancelled`
- `ChildWorkflowCancelled`

### 3.3 Convert Cancellation Commands to Proto

**File:** `sdk/src/workflow/command.rs`

Update `to_proto()` to handle new command variants.

---

## Phase 4: Parallel Combinator Methods

### 4.1 Create Combinator Module

**File:** `sdk/src/workflow/combinators.rs` (new file)

```rust
/// Wait for all futures to complete. Returns error if any fails.
pub async fn join_all<T, I>(futures: I) -> Result<Vec<T>>
where
    I: IntoIterator,
    I::Item: CancellableFuture<Output = Result<T>>;

/// Wait for first future to complete. Cancels remaining futures.
pub async fn select<T, I>(futures: I) -> Result<T>
where
    I: IntoIterator,
    I::Item: CancellableFuture<Output = Result<T>>;

/// Wait for any N futures to complete. Cancels remaining.
pub async fn join_n<T, I>(futures: I, n: usize) -> Result<Vec<T>>
where
    I: IntoIterator,
    I::Item: CancellableFuture<Output = Result<T>>;
```

### 4.2 Add Extension Trait for Context

**File:** `sdk/src/workflow/combinators.rs`

```rust
/// Extension trait for parallel workflow operations
pub trait WorkflowFutureExt: WorkflowContext {
    /// Wait for future with timeout. Cancels future if timeout expires.
    fn with_timeout<T, F>(&self, future: F, timeout: Duration) -> WithTimeout<F>
    where
        F: CancellableFuture<Output = Result<T>>;
}

pub struct WithTimeout<F> {
    future: F,
    timer: TimerFuture,
}

impl<T, F: CancellableFuture<Output = Result<T>>> Future for WithTimeout<F> {
    type Output = Result<T>;
    // Poll both, return first to complete, cancel the other
}
```

### 4.3 Export Combinators

**File:** `sdk/src/workflow/mod.rs`

Add `mod combinators;` and export.

---

## Phase 5: Unit Tests

### 5.1 Future Type Tests

**File:** `sdk/src/workflow/future.rs` (module tests)

Tests:
- `task_future_returns_pending_when_not_completed`
- `task_future_returns_ready_when_completed`
- `task_future_returns_error_when_failed`
- `task_future_cancel_records_command`
- `timer_future_cancel_is_synchronous`
- `child_workflow_future_returns_pending_when_running`
- `promise_future_cannot_be_cancelled`

### 5.2 Async Method Tests

**File:** `sdk/src/workflow/context_impl.rs` (module tests)

Tests:
- `schedule_async_assigns_sequence_at_creation`
- `schedule_async_multiple_tasks_get_sequential_ids`
- `sleep_async_returns_timer_future`
- `schedule_workflow_async_validates_name_on_replay`
- `async_methods_record_commands_immediately`

### 5.3 Parallel Replay Tests

**File:** `sdk/src/workflow/context_impl.rs` (module tests)

Tests:
- `parallel_tasks_match_by_per_type_sequence`
- `parallel_tasks_and_timer_independent_matching`
- `parallel_completion_order_does_not_affect_replay`
- `mixed_parallel_operations_replay_correctly`

### 5.4 Combinator Tests

**File:** `sdk/src/workflow/combinators.rs` (module tests)

Tests:
- `join_all_waits_for_all_futures`
- `join_all_returns_first_error`
- `select_returns_first_completed`
- `select_cancels_remaining_futures`
- `with_timeout_returns_result_if_fast`
- `with_timeout_returns_error_if_slow`
- `join_n_returns_first_n_results`

---

## Phase 6: TCK Corpus Tests

### 6.1 Create Parallel Execution Corpus Files

**Directory:** `sdk/tests/shared/replay-corpus/`

Files to create:
- `parallel-two-tasks.json` - Two tasks scheduled in parallel
- `parallel-task-and-timer.json` - Task + timer scheduled in parallel
- `parallel-three-tasks-one-fails.json` - Three tasks, one fails
- `parallel-select-first-wins.json` - Select pattern, first task wins
- `parallel-timeout-success.json` - Task completes before timeout
- `parallel-timeout-expires.json` - Timeout fires before task
- `parallel-mixed-operations.json` - Tasks + timers + child workflows

### 6.2 Update Determinism Corpus Test Runner

**File:** `sdk/tests/tck/determinism_corpus.rs`

Add test cases for parallel execution scenarios.

---

## Phase 7: E2E Tests

### 7.1 Create Parallel Test Workflows

**File:** `sdk/tests/e2e/fixtures/parallel_workflows.rs` (new file)

Workflows:
- `ParallelTasksWorkflow` - Schedules multiple tasks, awaits all
- `RacingTasksWorkflow` - Two tasks racing, first wins
- `TimeoutWorkflow` - Task with timeout
- `FanOutFanInWorkflow` - Process items in parallel, aggregate

### 7.2 Create Parallel E2E Test Module

**File:** `sdk/tests/e2e/parallel_tests.rs` (new file)

Tests:
- `test_e2e_parallel_tasks_execute_concurrently`
- `test_e2e_parallel_tasks_replay_correctly`
- `test_e2e_select_returns_first_result`
- `test_e2e_select_cancels_remaining`
- `test_e2e_timeout_success`
- `test_e2e_timeout_expires`
- `test_e2e_fan_out_fan_in`
- `test_e2e_mixed_parallel_operations`

### 7.3 Update E2E mod.rs

**File:** `sdk/tests/e2e/mod.rs`

Add `mod parallel_tests;` and `mod fixtures/parallel_workflows;`

---

## Phase 8: MockWorkflowContext Updates

### 8.1 Add Async Methods to MockWorkflowContext

**File:** `sdk/src/testing/mock_context.rs`

Implement all async methods for the mock context, returning mock futures that can be controlled in tests.

### 8.2 Add MockTaskFuture, MockTimerFuture, etc.

**File:** `sdk/src/testing/mock_future.rs` (new file)

Create mock future types that allow tests to:
- Control when futures complete
- Simulate failures
- Verify cancellation was called

---

## Phase 9: Examples and Documentation

### 9.1 Add Parallel Execution Example to Patterns Sample

**File:** `examples/patterns/src/parallel_workflow.rs` (new file)

Create workflows demonstrating parallel patterns:

```rust
/// 1. FanOutFanInWorkflow - Process items in parallel, aggregate results
pub struct FanOutFanInWorkflow;
// Input: { items: ["a", "b", "c", "d"] }
// Schedules ProcessItemTask for each item in parallel
// Waits for all with join_all()
// Aggregates results

/// 2. RacingWorkflow - First task to complete wins
pub struct RacingWorkflow;
// Schedules FetchFromPrimary and FetchFromFallback in parallel
// Uses select() to get first result
// Cancels the slower task

/// 3. TimeoutWorkflow - Task with timeout protection
pub struct TimeoutWorkflow;
// Schedules SlowTask
// Uses with_timeout() wrapper
// Returns timeout error if task exceeds duration

/// 4. BatchWithConcurrencyLimitWorkflow - Controlled parallelism
pub struct BatchWithConcurrencyLimitWorkflow;
// Processes items in batches (e.g., 3 at a time)
// Uses join_n() for controlled concurrency

/// 5. DynamicParallelismWorkflow - Runtime-determined concurrency
pub struct DynamicParallelismWorkflow;
// Fetches list of items from external source (unknown count)
// Dynamically creates futures based on fetched data
// Uses FuturesUnordered for streaming results as they complete
// Demonstrates that parallel count can be determined at runtime
```

**File:** `examples/patterns/src/parallel_tasks.rs` (new file)

Create task definitions for the parallel examples:

```rust
/// Task that processes a single item
pub struct ProcessItemTask;

/// Task that fetches from primary source
pub struct FetchFromPrimaryTask;

/// Task that fetches from fallback source
pub struct FetchFromFallbackTask;

/// Slow task for timeout demonstration
pub struct SlowProcessingTask;

/// Task that returns a dynamic list of items (for dynamic parallelism demo)
pub struct FetchItemsTask;
// Returns Vec<Item> with runtime-determined count
```

### 9.2 Update Patterns Sample Main

**File:** `examples/patterns/src/main.rs`

- Add `mod parallel_workflow;` and `mod parallel_tasks;`
- Register new workflows and tasks with the client
- Add parallel patterns to the help output

### 9.3 Add Integration Tests for Examples

**File:** `examples/patterns/src/parallel_workflow.rs` (module tests)

Tests using `MockWorkflowContext`:
- `test_fan_out_fan_in_executes_all_items`
- `test_racing_returns_first_result`
- `test_racing_cancels_slower_task`
- `test_timeout_succeeds_when_fast`
- `test_timeout_fails_when_slow`
- `test_batch_respects_concurrency_limit`

### 9.4 Update SDK Documentation

Add doc comments to all new public types and methods:
- `CancellableFuture` trait
- All future types (`TaskFuture`, `TimerFuture`, etc.)
- All async methods on `WorkflowContext`
- All combinators (`join_all`, `select`, `with_timeout`)

### 9.5 Update Examples README

**File:** `examples/README.md`

Add section documenting parallel execution patterns with code snippets.

---

## TODO List

### Phase 1: Future Types and CancellableFuture Trait
- [x] Create `sdk/src/workflow/future.rs`
- [x] Define `CancellableFuture` trait
- [x] Implement `TaskFuture<T>`
- [x] Implement `TimerFuture`
- [x] Implement `ChildWorkflowFuture<T>`
- [x] Implement `PromiseFuture<T>`
- [x] Implement `OperationFuture<T>`
- [x] Export from `sdk/src/workflow/mod.rs`

### Phase 2: Async Operation Methods
- [x] Add async method signatures to `WorkflowContext` trait
- [x] Implement `schedule_async_raw()` in `WorkflowContextImpl`
- [x] Implement `schedule_async<T>()` in `WorkflowContextExt` (typed version via extension trait)
- [x] Implement `schedule_async_with_options_raw()` in `WorkflowContextImpl`
- [x] Implement `schedule_async_with_options<T>()` in `WorkflowContextExt` (typed version via extension trait)
- [x] Implement `sleep_async()` in `WorkflowContextImpl`
- [x] Implement `schedule_workflow_async_raw()` in `WorkflowContextImpl`
- [x] Implement `schedule_workflow_async<W>()` in `WorkflowContextExt` (typed version via extension trait)
- [x] Implement `promise_async_raw()` in `WorkflowContextImpl`
- [x] Implement `promise_async<T>()` in `WorkflowContextExt` (typed version via extension trait)
- [x] Implement `run_async_raw()` in `WorkflowContextImpl`
- [x] Add context trait implementations for futures (dummy contexts)
- [x] Add completion check helpers via replay event matching

### Phase 3: Cancellation Commands
- [x] Add `RequestCancelTask` command variant
- [x] Add `CancelTimer` command variant
- [x] Add `RequestCancelChildWorkflow` command variant
- [x] Ensure `TaskCancelled` event type exists
- [x] Ensure `TimerCancelled` event type exists
- [x] Ensure `ChildWorkflowCancelled` event type exists
- [x] Update command handling in workflow_worker.rs

### Phase 4: Parallel Combinator Methods
- [x] Create `sdk/src/workflow/combinators.rs`
- [x] Implement `join_all()`
- [x] Implement `select()`
- [x] Implement `join_n()`
- [x] Implement `with_timeout()`
- [x] Export from `sdk/src/workflow/mod.rs`

### Phase 5: Unit Tests
- [x] Add `task_future_returns_pending_when_not_completed` test
- [x] Add `task_future_returns_ready_when_completed` test
- [x] Add `task_future_returns_error_when_failed` test
- [x] Add `task_future_cancel_records_command` test
- [x] Add `timer_future_cancel_is_synchronous` test
- [x] Add `child_workflow_future_returns_pending_when_running` test
- [x] Add `promise_future_cannot_be_cancelled` test
- [x] Add `schedule_async_assigns_sequence_at_creation` test
- [x] Add `schedule_async_multiple_tasks_get_sequential_ids` test
- [x] Add `sleep_async_returns_timer_future` test
- [x] Add `schedule_workflow_async_validates_name_on_replay` test (covered by sequence-based child workflow tests)
- [x] Add `async_methods_record_commands_immediately` test
- [x] Add `parallel_tasks_match_by_per_type_sequence` test
- [x] Add `parallel_tasks_and_timer_independent_matching` test
- [x] Add `parallel_completion_order_does_not_affect_replay` test
- [x] Add `mixed_parallel_operations_replay_correctly` test
- [x] Add `join_all_waits_for_all_futures` test (test_join_all_all_ready)
- [x] Add `join_all_returns_first_error` test (test_join_all_one_error)
- [x] Add `select_returns_first_completed` test (test_select_returns_first_ready)
- [x] Add `select_returns_error_on_first_failure` test
- [x] Add `with_timeout_returns_result_if_fast` test (test_with_timeout_returns_result_if_inner_completes_first)
- [x] Add `with_timeout_returns_error_if_slow` test (test_with_timeout_returns_timeout_error_if_timer_fires_first)
- [x] Add `join_n_returns_first_n_results` test (test_join_n_partial)
- [x] Add `join_n_with_error` test
- [x] Add `with_timeout_inner_error_propagates` test
- [x] Add `join_all_preserves_order` test
- [x] Add `join_all_single_future` test
- [x] Add `select_with_all_errors` test

### Phase 6: TCK Corpus Tests
- [x] Create `parallel-two-tasks.json`
- [x] Create `parallel-task-and-timer.json`
- [x] Create `parallel-three-tasks-one-fails.json`
- [x] Create `parallel-select-first-wins.json`
- [x] Create `parallel-timeout-success.json`
- [x] Create `parallel-timeout-expires.json`
- [x] Create `parallel-mixed-operations.json`
- [x] Update `determinism_corpus.rs` for parallel tests

### Phase 7: E2E Tests
- [ ] Create `sdk/tests/e2e/fixtures/parallel_workflows.rs`
- [ ] Implement `ParallelTasksWorkflow`
- [ ] Implement `RacingTasksWorkflow`
- [ ] Implement `TimeoutWorkflow`
- [ ] Implement `FanOutFanInWorkflow`
- [ ] Create `sdk/tests/e2e/parallel_tests.rs`
- [ ] Add `test_e2e_parallel_tasks_execute_concurrently`
- [ ] Add `test_e2e_parallel_tasks_replay_correctly`
- [ ] Add `test_e2e_select_returns_first_result`
- [ ] Add `test_e2e_select_cancels_remaining`
- [ ] Add `test_e2e_timeout_success`
- [ ] Add `test_e2e_timeout_expires`
- [ ] Add `test_e2e_fan_out_fan_in`
- [ ] Add `test_e2e_mixed_parallel_operations`
- [ ] Update `sdk/tests/e2e/mod.rs`

### Phase 8: MockWorkflowContext Updates
- [x] Add async methods to `MockWorkflowContext`
- [ ] Create `sdk/src/testing/mock_future.rs` (not needed - using actual futures with dummy contexts)
- [ ] Implement `MockTaskFuture` (not needed)
- [ ] Implement `MockTimerFuture` (not needed)
- [ ] Implement `MockChildWorkflowFuture` (not needed)
- [ ] Export mock futures (not needed)

### Phase 9: Examples and Documentation

**Parallel workflows:**
- [x] Create `examples/patterns/src/parallel_workflow.rs`
- [x] Implement `FanOutFanInWorkflow` (join_all pattern)
- [x] Implement `RacingWorkflow` (select pattern)
- [x] Implement `TimeoutWorkflow` (with_timeout pattern)
- [x] Implement `BatchWithConcurrencyLimitWorkflow` (batch processing pattern)
- [x] Implement `PartialCompletionWorkflow` (join_n pattern)
- [x] Implement `DynamicParallelismWorkflow` (runtime-determined parallelism)

**Parallel tasks:**
- [x] Create `examples/patterns/src/parallel_tasks.rs`
- [x] Implement `ProcessItemTask`
- [x] Implement `FetchDataTask` (for racing workflow)
- [x] Implement `SlowOperationTask` (for timeout workflow)
- [x] Implement `RunOperationTask` (for partial completion)
- [x] Implement `FetchItemsTask` (returns dynamic list)

**Integration:**
- [x] Update `examples/patterns/src/main.rs` with new modules
- [x] Register parallel workflows
- [x] Add parallel patterns to help output

**Tests:**
- [x] Add serialization tests for all input/output types
- [ ] Add `test_fan_out_fan_in_executes_all_items` test
- [ ] Add `test_racing_returns_first_result` test
- [ ] Add `test_racing_cancels_slower_task` test
- [ ] Add `test_timeout_succeeds_when_fast` test
- [ ] Add `test_timeout_fails_when_slow` test
- [ ] Add `test_batch_respects_concurrency_limit` test
- [ ] Add `test_dynamic_parallelism_handles_variable_count` test

**Documentation:**
- [x] Add doc comments to `CancellableFuture`
- [x] Add doc comments to all future types
- [x] Add doc comments to async methods
- [x] Add doc comments to combinators
- [x] Update `examples/README.md` with parallel patterns section

---

## Verification Checklist

After implementation:

- [x] `cargo build --workspace` succeeds
- [x] `cargo test --workspace` passes
- [x] `cargo test --test tck -p flovyn-sdk` passes
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] `cargo fmt --all -- --check` passes
- [ ] E2E tests pass with server running (E2E parallel tests not yet created)
- [x] Examples compile and run correctly

---

## Estimated Effort

| Phase | Complexity | Description |
|-------|------------|-------------|
| Phase 1 | Medium | Future types and trait definition |
| Phase 2 | High | Async methods in context |
| Phase 3 | Low | Cancellation commands |
| Phase 4 | Medium | Combinator implementation |
| Phase 5 | Medium | Unit tests |
| Phase 6 | Low | TCK corpus files |
| Phase 7 | Medium | E2E tests |
| Phase 8 | Medium | Mock context updates |
| Phase 9 | Medium | Examples and documentation |

---

## Dependencies

This implementation depends on:
1. **Sequence-Based Replay** (completed) - Per-type sequence counters are the foundation for parallel execution determinism

## Server Requirements

The server may need updates to handle:
1. `RequestCancelTask` command → should send cancellation to task worker
2. `CancelTimer` command → should cancel timer and emit `TimerCancelled` event
3. `RequestCancelChildWorkflow` command → should propagate cancellation to child

Verify server support before implementing E2E tests for cancellation.
