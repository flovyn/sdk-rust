# Design: FFI-Level Replay Handling

## Overview

Move replay logic from language SDKs to the Rust FFI layer. This ensures consistent replay behavior across all language SDKs (Kotlin, Python, Go, etc.) with a single implementation.

## Current Architecture (Problem)

```
┌─────────────────────────────────────────────────────┐
│ Kotlin SDK                                          │
│  WorkflowContextImpl                                │
│  - ctx.promise() → always generates CreatePromise   │
│  - ctx.scheduleTask() → always generates command    │
│  - No replay awareness                              │
└─────────────────────┬───────────────────────────────┘
                      │ commands (including duplicates)
┌─────────────────────▼───────────────────────────────┐
│ Rust FFI                                            │
│  - Receives all commands                            │
│  - Submits all to server                            │
│  - Server gets duplicate commands during replay     │
└─────────────────────────────────────────────────────┘
```

**Problems:**
1. Kotlin SDK generates commands even during replay
2. Server receives duplicate commands (e.g., CreatePromise twice)
3. Each language SDK would need to implement replay logic
4. Determinism validation is inconsistent

## Proposed Architecture

```
┌─────────────────────────────────────────────────────┐
│ Kotlin SDK                                          │
│  WorkflowContextImpl (thin wrapper)                 │
│  - ctx.promise() → calls FFI.createPromise()        │
│  - ctx.scheduleTask() → calls FFI.scheduleTask()    │
│  - Receives results from FFI                        │
└─────────────────────┬───────────────────────────────┘
                      │ FFI method calls
┌─────────────────────▼───────────────────────────────┐
│ Rust FFI (WorkflowContext)                          │
│  - Stores replay events per workflow                │
│  - Checks events before generating commands         │
│  - Validates determinism (name/type matching)       │
│  - Only generates NEW commands                      │
│  - Returns cached results for replayed operations   │
└─────────────────────────────────────────────────────┘
```

## FFI API Changes

### New: WorkflowContext Object

Instead of Kotlin managing commands, the FFI exposes a `WorkflowContext` that handles replay:

```rust
#[derive(uniffi::Object)]
pub struct FfiWorkflowContext {
    workflow_execution_id: Uuid,

    // Replay state
    replay_events: Vec<ReplayEvent>,
    task_events: Vec<ReplayEvent>,
    promise_events: Vec<ReplayEvent>,
    timer_events: Vec<ReplayEvent>,
    child_workflow_events: Vec<ReplayEvent>,

    // Per-type sequence counters
    next_task_seq: AtomicU32,
    next_promise_seq: AtomicU32,
    next_timer_seq: AtomicU32,
    next_child_workflow_seq: AtomicU32,

    // Generated commands (only NEW ones)
    commands: Mutex<Vec<FfiWorkflowCommand>>,

    // Resolved promises/tasks from replay
    resolved_promises: HashMap<String, ResolvedValue>,
    completed_tasks: HashMap<Uuid, TaskResult>,
}

#[uniffi::export]
impl FfiWorkflowContext {
    /// Create a durable promise.
    /// Returns immediately if promise was already resolved during replay.
    /// Returns None if promise is pending (workflow should suspend).
    pub fn create_promise(&self, name: &str, timeout_ms: Option<i64>) -> FfiPromiseResult;

    /// Schedule a task.
    /// Returns immediately if task was already completed during replay.
    /// Returns None if task is pending (workflow should suspend).
    pub fn schedule_task(
        &self,
        task_type: &str,
        input: Vec<u8>,
        options: TaskOptions,
    ) -> FfiTaskResult;

    /// Start a timer.
    /// Returns immediately if timer already fired during replay.
    /// Returns None if timer is pending (workflow should suspend).
    pub fn start_timer(&self, timer_id: &str, duration_ms: i64) -> FfiTimerResult;

    /// Schedule a child workflow.
    pub fn schedule_child_workflow(
        &self,
        name: &str,
        kind: Option<&str>,
        input: Vec<u8>,
        options: ChildWorkflowOptions,
    ) -> FfiChildWorkflowResult;

    /// Get current time (deterministic).
    pub fn current_time_millis(&self) -> i64;

    /// Generate deterministic random.
    pub fn random(&self) -> f64;

    /// Generate deterministic UUID.
    pub fn random_uuid(&self) -> String;

    /// Set workflow state.
    pub fn set_state(&self, key: &str, value: Vec<u8>);

    /// Get workflow state.
    pub fn get_state(&self, key: &str) -> Option<Vec<u8>>;

    /// Get the commands generated (only new ones, not replayed).
    pub fn take_commands(&self) -> Vec<FfiWorkflowCommand>;
}
```

### Result Types

```rust
#[derive(uniffi::Enum)]
pub enum FfiPromiseResult {
    /// Promise was resolved during replay
    Resolved { value: Vec<u8> },
    /// Promise was rejected during replay
    Rejected { error: String },
    /// Promise is pending - workflow should suspend
    Pending { promise_id: String },
}

#[derive(uniffi::Enum)]
pub enum FfiTaskResult {
    /// Task completed during replay
    Completed { output: Vec<u8> },
    /// Task failed during replay
    Failed { error: String },
    /// Task is pending - workflow should suspend
    Pending { task_execution_id: String },
}

#[derive(uniffi::Enum)]
pub enum FfiTimerResult {
    /// Timer fired during replay
    Fired,
    /// Timer is pending - workflow should suspend
    Pending { timer_id: String },
}
```

### Updated CoreWorker

```rust
impl CoreWorker {
    /// Poll for workflow activation.
    /// Returns a WorkflowActivation with an FfiWorkflowContext.
    pub fn poll_workflow_activation(&self) -> Result<Option<WorkflowActivationWithContext>, FfiError>;

    /// Complete workflow activation.
    /// Commands are taken from the context automatically.
    pub fn complete_workflow_activation(
        &self,
        context: &FfiWorkflowContext,
        status: WorkflowCompletionStatus,
    ) -> Result<(), FfiError>;
}

#[derive(uniffi::Record)]
pub struct WorkflowActivationWithContext {
    pub context: Arc<FfiWorkflowContext>,
    pub workflow_kind: String,
    pub input: Vec<u8>,
    // ... other fields
}
```

## Kotlin SDK Changes

### WorkflowContextImpl (Simplified)

```kotlin
class WorkflowContextImpl(
    private val ffiContext: FfiWorkflowContext,
    private val serializer: JsonSerializer,
) : WorkflowContext {

    override suspend fun <T> promise(name: String, timeout: Duration?): DurablePromise<T> {
        return when (val result = ffiContext.createPromise(name, timeout?.toMillis())) {
            is FfiPromiseResult.Resolved -> {
                ResolvedDurablePromise(serializer.deserialize(result.value))
            }
            is FfiPromiseResult.Rejected -> {
                RejectedDurablePromise(result.error)
            }
            is FfiPromiseResult.Pending -> {
                // Suspend workflow - will be resumed when promise is resolved
                throw WorkflowSuspendedException("Waiting for promise: ${result.promiseId}")
            }
        }
    }

    override suspend fun <I, O> scheduleTask(
        taskType: String,
        input: I,
        options: TaskOptions,
    ): O {
        val inputBytes = serializer.serialize(input)
        return when (val result = ffiContext.scheduleTask(taskType, inputBytes, options.toFfi())) {
            is FfiTaskResult.Completed -> {
                serializer.deserialize(result.output)
            }
            is FfiTaskResult.Failed -> {
                throw TaskFailedException(result.error)
            }
            is FfiTaskResult.Pending -> {
                throw WorkflowSuspendedException("Waiting for task: ${result.taskExecutionId}")
            }
        }
    }

    override suspend fun sleep(duration: Duration) {
        val timerId = "timer-${ffiContext.nextTimerSeq()}"
        when (val result = ffiContext.startTimer(timerId, duration.toMillis())) {
            is FfiTimerResult.Fired -> {
                // Timer already fired during replay, continue
            }
            is FfiTimerResult.Pending -> {
                throw WorkflowSuspendedException("Waiting for timer: ${result.timerId}")
            }
        }
    }

    // No more commands list - FFI handles it all
}
```

### WorkflowWorker (Simplified)

```kotlin
class WorkflowWorker(...) {
    private suspend fun processActivation(activationWithContext: WorkflowActivationWithContext) {
        val ffiContext = activationWithContext.context
        val workflow = registry.get(activationWithContext.workflowKind)

        val context = WorkflowContextImpl(ffiContext, serializer)

        try {
            val result = workflow.execute(context, input)

            // Complete - FFI takes commands from context automatically
            coreBridge.completeWorkflowActivation(
                ffiContext,
                WorkflowCompletionStatus.Completed(serializer.serialize(result))
            )
        } catch (e: WorkflowSuspendedException) {
            coreBridge.completeWorkflowActivation(
                ffiContext,
                WorkflowCompletionStatus.Suspended
            )
        } catch (e: Exception) {
            coreBridge.completeWorkflowActivation(
                ffiContext,
                WorkflowCompletionStatus.Failed(e.message ?: "Unknown error")
            )
        }
    }
}
```

## Implementation Plan

### Phase 1: Create FfiWorkflowContext
1. Add `FfiWorkflowContext` struct with replay state
2. Implement `create_promise()` with replay checking
3. Implement `schedule_task()` with replay checking
4. Implement `start_timer()` with replay checking
5. Implement `take_commands()` to get only new commands

### Phase 2: Update CoreWorker
1. Change `poll_workflow_activation()` to return context
2. Change `complete_workflow_activation()` to use context
3. Add proper replay event parsing

### Phase 3: Update Kotlin SDK
1. Update `WorkflowContextImpl` to use FFI context
2. Update `WorkflowWorker` to use new API
3. Remove command generation from Kotlin

### Phase 4: Testing
1. Verify replay works correctly
2. Verify no duplicate commands
3. Run all E2E tests

## Benefits

1. **Single replay implementation** - Only Rust code handles replay
2. **Determinism validation** - FFI can validate operation names match
3. **Simpler language SDKs** - Just call FFI methods, no replay logic
4. **Future SDKs benefit** - Python, Go, etc. get replay handling automatically
5. **No duplicate commands** - FFI only generates commands for new operations

## Risks

1. **Breaking change** - Kotlin SDK API changes significantly
2. **FFI complexity** - More logic in FFI layer
3. **Debugging** - Harder to debug from Kotlin side

## Migration

Since this is a development SDK, we can make breaking changes. The Kotlin SDK public API (`WorkflowContext`) stays the same - only internal implementation changes.
