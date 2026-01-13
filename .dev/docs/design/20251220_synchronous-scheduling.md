# Design: Synchronous Scheduling

## Status
**Implemented** - SDK and Server changes complete

## Implementation Notes (2024-12)

Both SDK and server implementations are complete. Key implementation details:

### UUID Counter Synchronization
A critical issue was identified and addressed: during replay, the `random_uuid()` counter must always advance even when using IDs from events. This ensures deterministic UUID generation when extending a replay.

```rust
fn schedule_with_options_raw(&self, task_type: &str, input: Value, options: ScheduleTaskOptions) -> TaskFutureRaw {
    let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

    // CRITICAL: Always generate UUID to keep counter synchronized
    let generated_id = self.random_uuid();

    if let Some(event) = self.task_events.get(task_seq as usize) {
        // Replay: use ID from event (ignore generated_id, but counter advanced)
        let event_id = event.get_string("taskExecutionId")...;
        return TaskFuture::from_replay(...);
    }

    // New command: use generated_id
    ...
}
```

### Files Changed
- `sdk/src/workflow/context.rs` - Made `schedule_raw` and `schedule_with_options_raw` synchronous
- `sdk/src/workflow/context_impl.rs` - Removed `task_submitter`, added client-side ID generation
- `sdk/src/workflow/command.rs` - Added `max_retries`, `timeout_ms`, `queue` fields to `ScheduleTask`
- Deleted: `sdk/src/workflow/task_submitter.rs`, `sdk/src/client/grpc_task_submitter.rs`

### Test Corpus Updates
The test corpus files were updated to use valid UUIDs for `taskExecutionId` fields (e.g., `"11111111-1111-1111-1111-000000000001"`) instead of string IDs like `"task-1"`.

## Breaking Changes

### API Changes

| Before | After |
|--------|-------|
| `ctx.schedule_raw(task, input).await.await?` | `ctx.schedule_raw(task, input).await?` |
| `async fn schedule_raw(...)` | `fn schedule_raw(...)` |
| `join!(ctx.schedule_raw(...), ctx.schedule_raw(...)).await` | Direct calls: `let a = ctx.schedule_raw(...); let b = ctx.schedule_raw(...);` |

### Migration Guide

1. **Double-await to single-await**: Change all `.await.await?` patterns to `.await?`
2. **Remove tokio::join! on schedule calls**: Since scheduling is now synchronous, `join!` is not needed to collect futures
3. **Remove futures::future::join_all on schedule calls**: Collect futures directly into a Vec

**Example migration:**
```rust
// Before
let (task1, task2) = tokio::join!(
    ctx.schedule_raw("task-a", input1),
    ctx.schedule_raw("task-b", input2),
);
let (result1, result2) = tokio::join!(task1, task2);

// After
let task1 = ctx.schedule_raw("task-a", input1);
let task2 = ctx.schedule_raw("task-b", input2);
let (result1, result2) = tokio::join!(task1, task2);
```

## Problem Statement

The current Flovyn SDK uses **async scheduling** where the `schedule_raw` method makes a gRPC call to the server to obtain a server-generated task execution ID:

```rust
// Current: Async scheduling
async fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value> {
    // 1. Get sequence number (sync)
    let seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

    // 2. Call server to get task_execution_id (ASYNC - blocks here)
    let task_id = self.task_submitter.submit_task(...).await?;

    // 3. Record command and return future
    self.record_command(ScheduleTask { task_execution_id: task_id, ... });
    TaskFuture::new(seq, task_id, ...)
}
```

This creates several problems:

### 1. Awkward Double-Await Pattern
```rust
// User must await twice: once to schedule, once to get result
let task = ctx.schedule_raw("my-task", input).await;  // First await: get TaskFuture
let result = task.await?;  // Second await: get result
```

### 2. Parallel Scheduling Requires Extra Step
```rust
// Must await all schedules first, then await all results
let futures = join!(
    ctx.schedule_raw("task-a", input1),  // await to get TaskFuture
    ctx.schedule_raw("task-b", input2),
).await;
let results = join!(futures.0, futures.1).await;  // await to get results
```

### 3. Network Call During Command Creation
The gRPC call to get the task execution ID happens during scheduling, not deferred to when the workflow suspends. This is inefficient for parallel operations.

## Goals

1. Synchronous scheduling API that returns Futures immediately
2. Enable single-await usage: `ctx.schedule::<Task>(input).await`
3. Enable natural parallel patterns: `join!(a, b, c).await`
4. Client-side ID generation for deterministic replay
5. Batch command submission when workflow suspends

## Non-Goals

- Backwards compatibility with existing server (breaking changes allowed)
- Backwards compatibility with existing event histories

## Design Overview

### Target Pattern

```rust
// Synchronous scheduling, returns Future immediately
pub fn schedule(&self, opts: TaskOptions) -> impl Future<Output = Result<T>> {
    let seq = self.seq_nums.write().next_task_seq();  // Sync
    let cmd = self.create_command(opts, seq);          // Sync
    self.queue_command(cmd);                           // Sync (internal queue)
    TaskFuture::new(seq)                               // Returns immediately
}
```

**Key insight**: No network call during scheduling. Commands are queued internally and sent in batch when the workflow yields.

### Proposed Implementation

```rust
// Proposed: Synchronous scheduling with client-generated IDs
fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value> {
    // 1. Get per-type sequence (atomic)
    let seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

    // 2. Check replay events
    if let Some(event) = self.task_events.get(seq) {
        // Replay: validate and return pre-populated future
        return self.handle_replay_task(seq, event, task_type);
    }

    // 3. New command: generate ID deterministically
    let task_execution_id = self.random_uuid();  // Deterministic (seeded by workflow execution ID)

    // 4. Queue command internally (no network call)
    self.pending_commands.push(WorkflowCommand::ScheduleTask {
        task_type: task_type.to_string(),
        task_execution_id,
        input,
        ...
    });

    // 5. Return future immediately
    TaskFuture::new(seq, task_execution_id, self.weak_self())
}
```

## Detailed Design

### 1. Client-Side Task Execution ID Generation

#### Current Flow (Server-Generated IDs)
```
SDK                                    Server
 |                                       |
 |--submitTask(taskType, input)--------->|
 |                                       | Create TaskExecution
 |                                       | Generate UUID
 |<--taskExecutionId--------------------|
 |                                       |
 |--ScheduleTask{taskExecutionId}------->|
 |                                       | Create TASK_SCHEDULED event
```

#### Proposed Flow (Client-Generated IDs)
```
SDK                                    Server
 |                                       |
 | Generate UUID (deterministic)         |
 | Queue ScheduleTask command            |
 |                                       |
 | ... workflow continues ...            |
 |                                       |
 | Workflow suspends                     |
 |                                       |
 |--BatchCommands[ScheduleTask,...]----->|
 |                                       | For each ScheduleTask:
 |                                       |   Create TaskExecution with provided ID
 |                                       |   Create TASK_SCHEDULED event
```

#### Deterministic UUID Generation

The SDK already has `ctx.random_uuid()` which generates deterministic UUIDs seeded by the workflow execution ID:

```rust
impl WorkflowContextImpl {
    fn random_uuid(&self) -> Uuid {
        let mut rng = self.rng.lock();
        Uuid::from_u128(rng.gen())
    }
}
```

For task execution IDs, we use this same mechanism:
```rust
fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value> {
    let task_execution_id = self.random_uuid();  // Deterministic!
    // ...
}
```

**Why this is deterministic:**
1. `self.rng` is a `StdRng` seeded by workflow execution ID
2. Same workflow code → same sequence of `random_uuid()` calls
3. Same sequence → same UUIDs generated
4. Replay produces identical IDs

### 2. Command Queuing (No Immediate Network)

Replace immediate gRPC calls with internal command queue:

```rust
struct WorkflowContextImpl {
    // Pending commands to be sent when workflow suspends
    pending_commands: RefCell<Vec<WorkflowCommand>>,

    // ... other fields
}

impl WorkflowContextImpl {
    fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value> {
        // ... validation and sequence assignment ...

        // Queue command (no network call)
        self.pending_commands.borrow_mut().push(WorkflowCommand::ScheduleTask {
            sequence_number: self.next_global_sequence(),
            task_type: task_type.to_string(),
            task_execution_id: task_execution_id.clone(),
            input,
            priority_seconds: 0,
        });

        TaskFuture::new(seq, task_execution_id, self.weak_self())
    }

    /// Called when workflow suspends - sends all pending commands
    fn flush_commands(&self) -> Vec<WorkflowCommand> {
        self.pending_commands.borrow_mut().drain(..).collect()
    }
}
```

### 3. Batch Command Submission

The workflow executor collects commands and sends them in batch:

```rust
impl WorkflowExecutor {
    async fn execute_workflow(&self, ctx: &WorkflowContextImpl) -> ExecutionResult {
        loop {
            match self.run_workflow_step(ctx).await {
                StepResult::Completed(output) => {
                    let commands = ctx.flush_commands();
                    return ExecutionResult::Completed { output, commands };
                }
                StepResult::Suspended { reason } => {
                    let commands = ctx.flush_commands();
                    // Send batch to server
                    self.send_commands_batch(commands).await?;
                    return ExecutionResult::Suspended { reason };
                }
                StepResult::Failed(error) => {
                    let commands = ctx.flush_commands();
                    return ExecutionResult::Failed { error, commands };
                }
            }
        }
    }
}
```

### 4. Updated WorkflowContext Trait

Remove async from scheduling methods:

```rust
pub trait WorkflowContext: Send + Sync {
    // ===== Synchronous scheduling (returns Future immediately) =====

    /// Schedule a typed task. Returns a future that resolves when the task completes.
    fn schedule<T: TaskDefinition>(&self, input: T::Input) -> TaskFuture<T::Output>;

    /// Schedule a raw task by type name.
    fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value>;

    /// Schedule with options.
    fn schedule_with_options<T: TaskDefinition>(
        &self,
        input: T::Input,
        options: ScheduleTaskOptions,
    ) -> TaskFuture<T::Output>;

    fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> TaskFuture<Value>;

    /// Start a timer. Returns a future that resolves when the timer fires.
    fn sleep(&self, duration: Duration) -> TimerFuture;

    /// Schedule a child workflow.
    fn schedule_workflow<W: WorkflowDefinition>(
        &self,
        name: &str,
        input: W::Input,
    ) -> ChildWorkflowFuture<W::Output>;

    fn schedule_workflow_raw(
        &self,
        name: &str,
        kind: &str,
        input: Value,
    ) -> ChildWorkflowFuture<Value>;

    /// Create a promise that can be resolved externally.
    fn promise<T: DeserializeOwned>(&self, name: &str) -> PromiseFuture<T>;
    fn promise_raw(&self, name: &str) -> PromiseFuture<Value>;

    /// Run a side-effect operation (results are memoized).
    fn run<F, T>(&self, name: &str, f: F) -> OperationFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Serialize + DeserializeOwned + Send + 'static;

    // ===== Immediate operations (no future returned) =====

    fn current_time_millis(&self) -> i64;
    fn random_uuid(&self) -> Uuid;
    fn random(&self) -> f64;
    fn get_state<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>;
    fn set_state<T: Serialize>(&self, key: &str, value: &T) -> Result<()>;
    fn clear_state(&self, key: &str) -> Result<()>;
}
```

### 5. Usage Examples

#### Single Task (Clean Single Await)
```rust
// Before: Double await
let task = ctx.schedule_raw("my-task", input).await;
let result = task.await?;

// After: Single await
let result = ctx.schedule_raw("my-task", input).await?;
```

#### Parallel Tasks (Natural join!)
```rust
// Before: Awkward nested awaits
let (t1, t2, t3) = join!(
    ctx.schedule_raw("task-1", input1),
    ctx.schedule_raw("task-2", input2),
    ctx.schedule_raw("task-3", input3),
).await;
let (r1, r2, r3) = join!(t1, t2, t3).await;

// After: Natural pattern
let (r1, r2, r3) = join!(
    ctx.schedule_raw("task-1", input1),
    ctx.schedule_raw("task-2", input2),
    ctx.schedule_raw("task-3", input3),
).await;
```

#### Dynamic Parallelism
```rust
// Works naturally with iterators
let futures: Vec<_> = items
    .iter()
    .map(|item| ctx.schedule::<ProcessItem>(item.clone()))
    .collect();

let results = futures::future::join_all(futures).await;
```

## Server Changes Required

### 1. Accept Client-Provided Task Execution IDs

#### Current: Server generates ID in submitTask
```protobuf
message SubmitTaskRequest {
    string org_id = 1;
    string task_type = 2;
    bytes input = 3;
    optional string workflow_execution_id = 4;
    // Server generates task_execution_id
}

message SubmitTaskResponse {
    string task_execution_id = 1;  // Server-generated
}
```

#### Proposed: Client provides ID in ScheduleTask command
```protobuf
message ScheduleTaskCommand {
    int32 sequence_number = 1;
    string task_type = 2;
    string task_execution_id = 3;  // Client-generated (required)
    bytes input = 4;
    int32 priority_seconds = 5;
    optional int32 max_retries = 6;
    optional int64 timeout_ms = 7;
    optional string queue = 8;
}
```

### 2. Remove submitTask gRPC Method

The `submitTask` gRPC method becomes unnecessary. Task creation happens when processing the `ScheduleTask` command.

**Before (two-step):**
1. SDK calls `submitTask()` → server creates TaskExecution, returns ID
2. SDK sends `ScheduleTask` command → server creates event

**After (single-step):**
1. SDK sends `ScheduleTask` command with client ID → server creates TaskExecution + event

### 3. TaskExecution Creation from Command

When server processes `ScheduleTask` command:

```kotlin
// Server: WorkflowCommandProcessor.kt
fun processScheduleTask(command: ScheduleTaskCommand, workflowExecution: WorkflowExecution) {
    // Validate task_execution_id is valid UUID
    val taskExecutionId = UUID.fromString(command.taskExecutionId)

    // Check for duplicate (idempotency)
    if (taskExecutionRepository.existsById(taskExecutionId)) {
        // Already created (replay or retry) - skip creation
        return
    }

    // Create TaskExecution with client-provided ID
    val taskExecution = TaskExecution(
        id = taskExecutionId,  // Use client-provided ID
        orgId = workflowExecution.orgId,
        taskType = command.taskType,
        input = command.input,
        status = TaskExecutionStatus.PENDING,
        workflowExecutionId = workflowExecution.id,
        queue = command.queue ?: workflowExecution.taskQueue,
        maxRetries = command.maxRetries ?: 3,
        timeout = command.timeoutMs?.let { Duration.ofMillis(it) },
        createdAt = Instant.now(),
    )

    taskExecutionRepository.save(taskExecution)

    // Create TASK_SCHEDULED event
    workflowEventRepository.save(WorkflowEvent(
        workflowExecutionId = workflowExecution.id,
        sequenceNumber = command.sequenceNumber,
        eventType = EventType.TASK_SCHEDULED,
        payload = mapOf(
            "taskType" to command.taskType,
            "taskExecutionId" to command.taskExecutionId,
        ),
        timestamp = Instant.now(),
    ))
}
```

### 4. Idempotency with Client IDs

Since clients generate IDs, server must handle duplicates:

```kotlin
fun processScheduleTask(command: ScheduleTaskCommand, ...) {
    val taskExecutionId = UUID.fromString(command.taskExecutionId)

    // Idempotency check
    val existing = taskExecutionRepository.findById(taskExecutionId)
    if (existing.isPresent) {
        // Validate it matches (same task type, workflow, etc.)
        val task = existing.get()
        if (task.taskType != command.taskType) {
            throw DeterminismViolationException(
                "Task execution ID collision: ${command.taskExecutionId} " +
                "already exists with different task type"
            )
        }
        // Already created - idempotent, skip
        return
    }

    // Create new TaskExecution...
}
```

### 5. Batch Command Processing

Server should process multiple commands in a single transaction:

```kotlin
// Server: WorkflowDispatchService.kt
@Transactional
fun processWorkflowCommands(
    workflowExecutionId: UUID,
    commands: List<WorkflowCommand>
): ProcessCommandsResponse {
    val workflowExecution = workflowExecutionRepository.findById(workflowExecutionId)
        .orElseThrow { NotFoundException("Workflow not found") }

    for (command in commands) {
        when (command) {
            is ScheduleTaskCommand -> processScheduleTask(command, workflowExecution)
            is StartTimerCommand -> processStartTimer(command, workflowExecution)
            is ScheduleChildWorkflowCommand -> processScheduleChildWorkflow(command, workflowExecution)
            // ... other command types
        }
    }

    return ProcessCommandsResponse(success = true)
}
```

### 6. Remove TaskExecution gRPC Service (Optional)

With client-generated IDs, the `TaskExecutionService.submitTask` method is no longer needed:

```protobuf
// BEFORE
service TaskExecutionService {
    rpc SubmitTask(SubmitTaskRequest) returns (SubmitTaskResponse);
    rpc CompleteTask(CompleteTaskRequest) returns (CompleteTaskResponse);
    rpc FailTask(FailTaskRequest) returns (FailTaskResponse);
    // ...
}

// AFTER
service TaskExecutionService {
    // SubmitTask removed - tasks created via ScheduleTask command
    rpc CompleteTask(CompleteTaskRequest) returns (CompleteTaskResponse);
    rpc FailTask(FailTaskRequest) returns (FailTaskResponse);
    // ...
}
```

## SDK Implementation Changes

### 1. Remove TaskSubmitter

```rust
// REMOVE this trait and implementations
pub trait TaskSubmitter: Send + Sync {
    async fn submit_task(...) -> Result<Uuid>;
}

pub struct GrpcTaskSubmitter { ... }
```

### 2. Update WorkflowContextImpl

```rust
pub struct WorkflowContextImpl {
    // REMOVE
    // task_submitter: Option<Arc<dyn TaskSubmitter>>,

    // ADD
    pending_commands: RefCell<Vec<WorkflowCommand>>,

    // Keep existing
    next_task_seq: AtomicU32,
    task_events: Vec<ReplayEvent>,
    rng: Mutex<StdRng>,
    // ...
}

impl WorkflowContextImpl {
    // CHANGE: Synchronous, no async
    pub fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> TaskFuture<Value> {
        let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

        // Check for replay
        if let Some(event) = self.task_events.get(task_seq as usize) {
            // Validate and return replay future
            return self.handle_task_replay(task_seq, event, task_type);
        }

        // New command: generate deterministic ID
        let task_execution_id = self.random_uuid();

        // Queue command (no network)
        self.pending_commands.borrow_mut().push(WorkflowCommand::ScheduleTask {
            sequence_number: self.next_global_sequence(),
            task_type: task_type.to_string(),
            task_execution_id,
            input,
            priority_seconds: options.priority_seconds,
            max_retries: options.max_retries,
            timeout_ms: options.timeout.map(|d| d.as_millis() as i64),
            queue: options.queue,
        });

        TaskFuture::new(task_seq, task_execution_id, self.weak_self())
    }
}
```

### 3. Update FlovynClient Builder

```rust
impl FlovynClientBuilder {
    // REMOVE
    // fn task_submitter(self, submitter: impl TaskSubmitter) -> Self;

    // Keep
    pub async fn build(self) -> Result<FlovynClient> {
        // No longer needs to create GrpcTaskSubmitter
        // ...
    }
}
```

## Migration Path

Since breaking changes are allowed:

### Phase 1: Server Changes
1. Add client-provided `task_execution_id` to `ScheduleTask` command schema
2. Update command processor to create TaskExecution from command
3. Add idempotency checks for client IDs
4. Keep `submitTask` gRPC temporarily for backwards compatibility

### Phase 2: SDK Changes
1. Change `schedule_raw` to synchronous
2. Implement command queuing
3. Use `random_uuid()` for task execution IDs
4. Remove `TaskSubmitter` and `GrpcTaskSubmitter`
5. Update all examples and tests

### Phase 3: Cleanup
1. Remove `submitTask` gRPC method from server
2. Remove any deprecated code paths

## Testing Strategy

### Unit Tests
- Verify sequence numbers assigned synchronously
- Verify deterministic UUID generation
- Verify command queuing
- Verify replay matching works with client IDs

### Integration Tests
- Single task scheduling and completion
- Parallel task scheduling (join!)
- Dynamic parallelism (join_all)
- Task cancellation
- Replay with client-generated IDs

### E2E Tests
- Full workflow with parallel tasks
- Workflow resume after suspension
- Server restart during parallel execution

## Appendix: Sequence Diagram

### Before (Async Scheduling)
```
Workflow          SDK                     Server
   |               |                        |
   |--schedule()-->|                        |
   |               |--submitTask()--------->|
   |               |                        | Create TaskExecution
   |               |<--taskExecutionId------|
   |               |                        |
   |<--await-------|                        |
   |               |                        |
   | (later)       |                        |
   |--await------->|                        |
   |               | Check events           |
   |               |--ScheduleTask cmd----->|
   |<--Suspended---|                        |
```

### After (Synchronous Scheduling)
```
Workflow          SDK                     Server
   |               |                        |
   |--schedule()-->|                        |
   |               | Gen UUID               |
   |               | Queue command          |
   |<--Future------|                        |
   |               |                        |
   |--await------->|                        |
   |               | Check events           |
   |               | None found             |
   |<--Suspended---|                        |
   |               |                        |
   |               |--Batch commands------->|
   |               |                        | For each ScheduleTask:
   |               |                        |   Create TaskExecution
   |               |                        |   Create event
```

## References

- [Sequence-Based Replay](./sequence-based-replay.md)
- [Parallel Execution](./parallel-execution.md) (deprecated)
