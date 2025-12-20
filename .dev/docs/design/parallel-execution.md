# Design: Parallel Execution Support

## Status
**DEPRECATED** - Superseded by [Synchronous Scheduling](./synchronous-scheduling.md)

> **Note**: This document described an approach using async scheduling methods (`schedule_async_raw`)
> that required awkward double-await patterns. The new [Synchronous Scheduling](./synchronous-scheduling.md)
> design uses synchronous scheduling methods that return Futures immediately,
> enabling natural `join!(a, b, c).await` patterns. Please refer to that document for the current design.

---

## Original Design (Deprecated)

*The content below is preserved for historical reference only.*

---

## Original Status
~~Proposed~~ - Builds on [Sequence-Based Replay](./sequence-based-replay.md)

## Problem Statement

The current Flovyn SDK supports only sequential execution of workflow operations. Each operation (task, timer, child workflow) must be awaited before starting the next one:

```rust
// Current: Sequential only
let result1 = ctx.schedule::<TaskA>(input1).await?;  // Blocks
let result2 = ctx.schedule::<TaskB>(input2).await?;  // Starts after TaskA completes
let result3 = ctx.sleep(Duration::from_secs(5)).await?;  // Starts after TaskB completes
```

This limits workflow expressiveness and efficiency. Many real-world workflows need:
- **Fan-out/Fan-in**: Process multiple items in parallel, then aggregate
- **Racing/Timeout**: Start multiple alternatives, use first to complete
- **Concurrent operations**: Schedule tasks while waiting for timers

## Goals

1. Enable parallel scheduling and awaiting of multiple operations
2. Support common parallel patterns: join_all, select/race, timeout
3. Support dynamic parallelism (runtime-determined number of concurrent operations)
4. Maintain deterministic replay semantics
5. Provide idiomatic Rust async/await experience
6. Support cancellation of pending operations

## Non-Goals

- Thread-based parallelism (workflows run on single-threaded executor)
- Automatic parallelization of sequential code
- Fire-and-forget spawning (all futures must be awaited)

## Design

### Core Insight: Per-Type Sequences Enable Parallelism

The [sequence-based replay design](./sequence-based-replay.md) already provides the foundation for parallel execution:

1. **Commands are numbered at creation time**, not completion time
2. **Each command type has independent sequence counters**
3. **Matching uses `(type, per_type_seq)`**, not global order or completion order

This means parallel execution is already deterministic:

```rust
// These get deterministic sequence numbers regardless of completion order
let a = ctx.schedule_async::<TaskA>(input1);  // Task(0)
let b = ctx.schedule_async::<TaskB>(input2);  // Task(1)
let c = ctx.sleep_async(Duration::from_secs(5)); // Timer(0)

// Completion order may vary, but replay always matches correctly
let (r1, r2, r3) = join!(a, b, c).await?;
```

### Async Operation API

Add non-blocking variants of all operations that return futures instead of awaiting immediately.

#### WorkflowContext Trait Additions

```rust
pub trait WorkflowContext {
    // Existing blocking methods (unchanged)
    async fn schedule<T: TaskDefinition>(&self, input: T::Input) -> Result<T::Output>;
    async fn sleep(&self, duration: Duration) -> Result<()>;
    async fn schedule_workflow<W: WorkflowDefinition>(&self, input: W::Input) -> Result<W::Output>;

    // New: Non-blocking async variants
    fn schedule_async<T: TaskDefinition>(&self, input: T::Input) -> TaskFuture<T::Output>;
    fn schedule_async_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value>;

    fn sleep_async(&self, duration: Duration) -> TimerFuture;

    fn schedule_workflow_async<W: WorkflowDefinition>(
        &self,
        name: &str,
        input: W::Input
    ) -> ChildWorkflowFuture<W::Output>;
    fn schedule_workflow_async_raw(
        &self,
        name: &str,
        kind: &str,
        input: Value
    ) -> ChildWorkflowFuture<Value>;

    fn promise_async<T: DeserializeOwned>(&self, name: &str) -> PromiseFuture<T>;
    fn promise_async_raw(&self, name: &str) -> PromiseFuture<Value>;

    fn run_async<F, T>(&self, name: &str, f: F) -> OperationFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Serialize + DeserializeOwned + Send + 'static;
}
```

### Future Types

Each operation type returns a specific future type that implements both `Future` and `CancellableFuture`:

```rust
/// Trait for futures that can be cancelled
pub trait CancellableFuture: Future {
    /// Cancel this operation. Returns immediately, cancellation is async.
    fn cancel(&self);

    /// Check if this future has been cancelled
    fn is_cancelled(&self) -> bool;
}

/// Future for a scheduled task
pub struct TaskFuture<T> {
    task_seq: u32,
    task_execution_id: String,
    context: Weak<WorkflowContextImpl>,
    _marker: PhantomData<T>,
}

impl<T: DeserializeOwned> Future for TaskFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check if task completed/failed in event history
        // Return Poll::Ready with result or Poll::Pending
    }
}

impl<T> CancellableFuture for TaskFuture<T> {
    fn cancel(&self) {
        // Record RequestCancelTask command
    }

    fn is_cancelled(&self) -> bool {
        // Check if TaskCancelled event exists
    }
}

/// Future for a timer/sleep
pub struct TimerFuture {
    timer_seq: u32,
    timer_id: String,
    context: Weak<WorkflowContextImpl>,
}

/// Future for a child workflow
pub struct ChildWorkflowFuture<T> {
    child_workflow_seq: u32,
    child_execution_id: String,
    context: Weak<WorkflowContextImpl>,
    _marker: PhantomData<T>,
}

/// Future for a promise/signal
pub struct PromiseFuture<T> {
    promise_seq: u32,
    promise_id: String,
    context: Weak<WorkflowContextImpl>,
    _marker: PhantomData<T>,
}

/// Future for a run() operation
pub struct OperationFuture<T> {
    operation_seq: u32,
    operation_name: String,
    context: Weak<WorkflowContextImpl>,
    _marker: PhantomData<T>,
}
```

### Parallel Combinators

Provide workflow-safe combinators that work with `CancellableFuture`:

```rust
/// Extension trait for parallel workflow operations
pub trait WorkflowFutureExt: WorkflowContext {
    /// Wait for all futures to complete. Returns error if any fails.
    async fn join_all<T, I>(&self, futures: I) -> Result<Vec<T>>
    where
        I: IntoIterator,
        I::Item: CancellableFuture<Output = Result<T>>;

    /// Wait for first future to complete. Cancels remaining futures.
    async fn select<T, I>(&self, futures: I) -> Result<T>
    where
        I: IntoIterator,
        I::Item: CancellableFuture<Output = Result<T>>;

    /// Wait for future with timeout. Cancels future if timeout expires.
    async fn with_timeout<T, F>(&self, future: F, timeout: Duration) -> Result<T>
    where
        F: CancellableFuture<Output = Result<T>>;

    /// Wait for any N futures to complete. Cancels remaining.
    async fn join_n<T, I>(&self, futures: I, n: usize) -> Result<Vec<T>>
    where
        I: IntoIterator,
        I::Item: CancellableFuture<Output = Result<T>>;
}
```

### Usage Examples

#### Fan-out/Fan-in Pattern

```rust
async fn process_batch(ctx: &impl WorkflowContext, items: Vec<Item>) -> Result<Summary> {
    // Schedule all tasks in parallel
    let futures: Vec<_> = items
        .into_iter()
        .map(|item| ctx.schedule_async::<ProcessItem>(item))
        .collect();

    // Wait for all to complete
    let results = ctx.join_all(futures).await?;

    // Aggregate results
    Ok(aggregate(results))
}
```

#### Racing/First-Wins Pattern

```rust
async fn fetch_with_fallback(ctx: &impl WorkflowContext, id: String) -> Result<Data> {
    let primary = ctx.schedule_async::<FetchFromPrimary>(id.clone());
    let fallback = ctx.schedule_async::<FetchFromFallback>(id);

    // First one to complete wins, other is cancelled
    ctx.select([primary, fallback]).await
}
```

#### Timeout Pattern

```rust
async fn fetch_with_timeout(ctx: &impl WorkflowContext, id: String) -> Result<Data> {
    let fetch = ctx.schedule_async::<FetchData>(id);

    ctx.with_timeout(fetch, Duration::from_secs(30)).await
        .map_err(|_| FlovynError::Timeout("Fetch timed out".into()))
}
```

#### Mixed Parallel Operations

```rust
async fn complex_workflow(ctx: &impl WorkflowContext) -> Result<Output> {
    // Schedule task and timer concurrently
    let task = ctx.schedule_async::<LongRunningTask>(input);
    let heartbeat = ctx.sleep_async(Duration::from_secs(30));

    // Use tokio::select! for fine-grained control
    tokio::select! {
        result = task => {
            // Task completed first
            result
        }
        _ = heartbeat => {
            // Heartbeat fired, task still running
            // Cancel task or take other action
            task.cancel();
            Err(FlovynError::Timeout("Task exceeded heartbeat".into()))
        }
    }
}
```

### Dynamic Parallelism

Dynamic parallelism (spawning a runtime-determined number of concurrent operations) is fully supported through the standard pattern of collecting futures into a `Vec` and joining them:

```rust
async fn process_dynamic_batch(ctx: &impl WorkflowContext, input: Input) -> Result<Output> {
    // Number of items determined at runtime - could be 1 or 1000
    let items = ctx.schedule::<FetchItems>(input.source).await?;

    // Dynamically create futures based on runtime data
    let futures: Vec<_> = items
        .into_iter()
        .map(|item| ctx.schedule_async::<ProcessItem>(item))
        .collect();

    // Wait for all - works with any Vec size
    let results = futures::future::join_all(futures).await;

    // Handle results
    let successes: Vec<_> = results.into_iter().filter_map(|r| r.ok()).collect();
    Ok(Output { processed: successes.len() })
}
```

**Why this works:**

1. **Per-type sequence numbers** - Each `schedule_async()` call gets the next sequence number, regardless of how many are called
2. **Deterministic creation order** - Iterating over `items` produces futures in the same order on replay
3. **Standard combinators** - `join_all()`, `FuturesUnordered`, `select_all()` all work naturally
4. **No spawn primitive needed** - Unlike Go/Temporal's `workflow.Go()`, Rust's async model handles this natively

**Scalable patterns:**

```rust
// Process in batches with controlled concurrency
for chunk in items.chunks(10) {
    let batch: Vec<_> = chunk.iter()
        .map(|item| ctx.schedule_async::<ProcessItem>(item.clone()))
        .collect();

    // Process 10 at a time
    let _ = futures::future::join_all(batch).await;
}

// Or use FuturesUnordered for streaming results
use futures::stream::{FuturesUnordered, StreamExt};

let mut futures: FuturesUnordered<_> = items
    .into_iter()
    .map(|item| ctx.schedule_async::<ProcessItem>(item))
    .collect();

while let Some(result) = futures.next().await {
    // Process results as they complete
    handle_result(result?);
}
```

**Limitation: No fire-and-forget**

All futures must eventually be awaited. You cannot "spawn and forget":

```rust
// NOT supported - future is dropped without awaiting
async fn bad_pattern(ctx: &impl WorkflowContext) {
    let _ = ctx.schedule_async::<BackgroundTask>(input);  // Dropped!
    // Task command recorded but result never collected
}
```

### Replay Semantics

#### Command Recording

When an async operation is created, the command is immediately recorded:

```rust
impl WorkflowContextImpl {
    fn schedule_async_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value> {
        // Get per-type sequence and increment atomically
        let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

        // Check if already in event history (replay case)
        if let Some(event) = self.task_events.get(task_seq as usize) {
            // Validate determinism
            let event_task_type = event.get_string("taskType").unwrap_or_default();
            if event_task_type != task_type {
                // Store error to be returned when future is polled
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

        // New command - record it
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
}
```

#### Future Polling

When a future is polled, it checks for completion in the event history:

```rust
impl<T: DeserializeOwned> Future for TaskFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ctx = self.context.upgrade()
            .ok_or_else(|| FlovynError::Other("Context dropped".into()))?;

        // Look for terminal event for this task
        match ctx.find_terminal_task_event(&self.task_execution_id) {
            Some(event) if event.is_task_completed() => {
                let result: T = event.get_result()?;
                Poll::Ready(Ok(result))
            }
            Some(event) if event.is_task_failed() => {
                Poll::Ready(Err(FlovynError::TaskFailed(event.get_error())))
            }
            Some(event) if event.is_task_cancelled() => {
                Poll::Ready(Err(FlovynError::Cancelled))
            }
            None => Poll::Pending,
            _ => unreachable!(),
        }
    }
}
```

#### Determinism Guarantee

Parallel execution is deterministic because:

1. **Sequence numbers assigned at creation**: `schedule_async` immediately assigns the next per-type sequence number, regardless of when the future is awaited
2. **Creation order is deterministic**: Workflow code runs deterministically, so the order of `schedule_async` calls is always the same
3. **Completion order doesn't matter**: Terminal events are matched by `task_execution_id`/`timer_id`, not by position or completion order
4. **Replay validation**: Each command validates its expected fields against the historical event at its sequence position

### Cancellation

#### Command Types

Add cancellation commands to the command enum:

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

#### Cancellation Semantics

| Operation Type | Cancellation Behavior |
|---------------|----------------------|
| **Task** | Sends RequestCancelTask. Task may complete, fail, or be cancelled. |
| **Timer** | Immediately cancelled. Future resolves with `Err(Cancelled)`. |
| **Child Workflow** | Sends RequestCancelChildWorkflow. Child may complete, fail, or be cancelled. |
| **Promise** | Cannot be cancelled (external signal required). |
| **Operation** | Cannot be cancelled (already completed or waiting for result). |

#### Timer Cancellation (Synchronous)

Timers can be cancelled synchronously because they don't involve external systems:

```rust
impl CancellableFuture for TimerFuture {
    fn cancel(&self) {
        if let Some(ctx) = self.context.upgrade() {
            // Record cancellation command
            ctx.record_command(WorkflowCommand::CancelTimer {
                sequence_number: ctx.next_global_sequence(),
                timer_id: self.timer_id.clone(),
            }).unwrap();

            // Immediately unblock the future
            ctx.unblock_timer(&self.timer_id, TimerResult::Cancelled);
        }
    }
}
```

### Workflow Suspension and Resumption

#### When All Parallel Operations Are Pending

When the workflow has scheduled parallel operations but none have completed:

```rust
// Workflow code
let a = ctx.schedule_async::<TaskA>(input1);  // Task(0)
let b = ctx.schedule_async::<TaskB>(input2);  // Task(1)
let (r1, r2) = tokio::join!(a, b).await?;     // Both pending, workflow suspends
```

The workflow executor detects that no progress can be made:
1. All futures returned `Poll::Pending`
2. Commands have been recorded for all operations
3. Workflow suspends and returns commands to server

#### When Some Operations Complete

When events arrive for completed operations:
1. Server sends workflow task with new events
2. Workflow re-executes from beginning (deterministic replay)
3. `schedule_async` returns futures pre-populated with results
4. `join!` completes for finished operations, continues waiting for others

### Standard Library Compatibility

The async futures work with standard Rust async patterns:

```rust
// Works with tokio::join!
let (a, b, c) = tokio::join!(
    ctx.schedule_async::<TaskA>(input1),
    ctx.schedule_async::<TaskB>(input2),
    ctx.sleep_async(Duration::from_secs(5))
).await;

// Works with tokio::select!
tokio::select! {
    result = ctx.schedule_async::<TaskA>(input) => handle_result(result),
    _ = ctx.sleep_async(Duration::from_secs(30)) => handle_timeout(),
}

// Works with futures::future::join_all
let futures: Vec<_> = items.iter()
    .map(|item| ctx.schedule_async::<ProcessItem>(item.clone()))
    .collect();
let results = futures::future::join_all(futures).await;
```

### Implementation Notes

#### Context Reference Handling

Futures need a reference to the workflow context to check event history during polling. Use `Weak<WorkflowContextImpl>` to avoid reference cycles:

```rust
pub struct TaskFuture<T> {
    context: Weak<WorkflowContextImpl>,
    // ...
}
```

#### Waker Integration

For proper async runtime integration, futures should register wakers:

```rust
impl<T: DeserializeOwned> Future for TaskFuture<T> {
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Register waker for notification when events arrive
        if let Some(ctx) = self.context.upgrade() {
            ctx.register_waker(&self.task_execution_id, cx.waker().clone());
        }
        // ... rest of polling logic
    }
}
```

When new events arrive, the executor notifies all registered wakers.

### Validation Matrix

| Scenario | Expected Behavior |
|----------|------------------|
| Create futures in deterministic order | Sequence numbers assigned correctly |
| Await futures in different order | Results match by ID, not await order |
| Task completes before timeout | Result returned, timer cancelled |
| Timeout fires before task | Timeout error, task cancellation requested |
| All parallel tasks succeed | All results returned |
| One parallel task fails | Error propagated, others may be cancelled |
| Replay with different completion order | Matching still works by ID |
| Code adds new parallel operation | New command at next sequence |
| Code removes parallel operation | Orphaned event not matched |

## Alternatives Considered

### 1. Explicit Parallel Block

```rust
ctx.parallel(|p| {
    p.schedule::<TaskA>(input1);
    p.schedule::<TaskB>(input2);
}).await?
```

**Rejected**: Less flexible, doesn't compose with standard Rust async patterns.

### 2. Actor-Style Message Passing

```rust
let task_handle = ctx.spawn_task::<TaskA>(input);
let result = task_handle.recv().await?;
```

**Rejected**: More complex, introduces channel-like semantics that don't fit workflow model.

### 3. Callback-Based API

```rust
ctx.schedule_with_callback::<TaskA>(input, |result| {
    // handle result
});
```

**Rejected**: Not idiomatic Rust, harder to compose and reason about.

## References

- [Sequence-Based Replay](./sequence-based-replay.md) - Foundation for deterministic matching

