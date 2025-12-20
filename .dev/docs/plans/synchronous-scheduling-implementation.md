# Implementation Plan: Synchronous Scheduling

## Critical Analysis

### Issue 1: UUID Counter Synchronization During Replay

**Problem**: The current `random_uuid()` uses a counter that increments on each call:
```rust
fn random_uuid(&self) -> Uuid {
    let counter = self.uuid_counter.fetch_add(1, Ordering::SeqCst);
    let name = format!("{}:{}", self.workflow_execution_id, counter);
    Uuid::new_v5(&self.workflow_execution_id, name.as_bytes())
}
```

During replay, if we skip calling `random_uuid()` when we find an existing event, the counter won't advance. This breaks determinism for subsequent NEW commands.

**Example of the bug**:
```
Original execution:
  schedule("task-a") → random_uuid() returns UUID-1, counter=1
  schedule("task-b") → random_uuid() returns UUID-2, counter=2
  schedule("task-c") → random_uuid() returns UUID-3, counter=3

Replay with 2 events, then extend:
  schedule("task-a") → event found, skip random_uuid()?, counter=0
  schedule("task-b") → event found, skip random_uuid()?, counter=0
  schedule("task-c") → no event, random_uuid() returns UUID-1, counter=1  // WRONG!
```

**Solution**: Always call `random_uuid()` to advance the counter, but use the event's ID during replay:
```rust
fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFuture<Value> {
    let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

    // ALWAYS generate UUID to keep counter synchronized
    let generated_id = self.random_uuid();

    if let Some(event) = self.task_events.get(task_seq as usize) {
        // Replay: use ID from event (ignore generated_id)
        let event_id = event.get_uuid("taskExecutionId")?;
        return TaskFuture::from_replay(task_seq, event_id, ...);
    }

    // New command: use generated_id
    self.pending_commands.push(ScheduleTask { task_execution_id: generated_id, ... });
    TaskFuture::new(task_seq, generated_id, ...)
}
```

### Issue 2: Transaction Boundaries on Server

**Problem**: When processing batch commands, server must:
1. Create all TaskExecution records
2. Create all workflow events
3. Update workflow state

If any step fails, the whole batch must roll back. Otherwise we get inconsistent state.

**Solution**: Server processes batch in single database transaction.

### Issue 3: Idempotency with Client-Generated IDs

**Problem**: Network retries could send the same commands twice.

**Solution**: Server checks if TaskExecution with client ID already exists:
```kotlin
fun processScheduleTask(command: ScheduleTaskCommand) {
    val existing = taskExecutionRepository.findById(command.taskExecutionId)
    if (existing.isPresent) {
        // Validate matches, then skip (idempotent)
        return
    }
    // Create new TaskExecution
}
```

### Issue 4: Implementation Order Dependencies

The SDK cannot send client-generated IDs until the server accepts them. Order matters:

```
Phase 1: Server adds support for client IDs (backwards compatible)
    ↓
Phase 2: SDK switches to synchronous scheduling with client IDs
    ↓
Phase 3: Remove deprecated code (submitTask, TaskSubmitter)
```

### Issue 5: Async Trait Removal

Currently `WorkflowContext` uses `#[async_trait]` because `schedule_raw` is async. Removing async from these methods requires:
1. Remove `async` from method signatures
2. Remove `#[async_trait]` from trait (if no async methods remain)
3. Update all implementations (main impl, mock impl)

### Issue 6: Breaking Change to Public API

The return type changes:
```rust
// Before
async fn schedule_raw(&self, ...) -> TaskFuture<Value>;  // Must await to get TaskFuture

// After
fn schedule_raw(&self, ...) -> TaskFuture<Value>;  // Returns TaskFuture directly
```

All user code calling `ctx.schedule_raw(...).await` must change to `ctx.schedule_raw(...)`.

---

## Implementation Plan

### Phase 1: Server Changes (Prerequisite)

#### 1.1 Update ScheduleTask Command Schema

**File**: `flovyn-server/src/api/grpc/proto/workflow.proto`

```protobuf
message ScheduleTaskCommand {
    int32 sequence_number = 1;
    string task_type = 2;
    string task_execution_id = 3;  // Now required, client-generated
    bytes input = 4;
    int32 priority_seconds = 5;
    optional int32 max_retries = 6;
    optional int64 timeout_ms = 7;
    optional string queue = 8;
}
```

#### 1.2 Update Command Processor

**File**: `flovyn-server/src/service/WorkflowCommandProcessor.kt`

```kotlin
@Transactional
fun processScheduleTask(
    command: ScheduleTaskCommand,
    workflowExecution: WorkflowExecution
) {
    val taskExecutionId = UUID.fromString(command.taskExecutionId)

    // Idempotency check
    if (taskExecutionRepository.existsById(taskExecutionId)) {
        val existing = taskExecutionRepository.findById(taskExecutionId).get()
        if (existing.taskType != command.taskType) {
            throw DeterminismViolationException(
                "Task ID collision: ${command.taskExecutionId}"
            )
        }
        // Already exists - idempotent, skip
        return
    }

    // Create TaskExecution with client-provided ID
    val taskExecution = TaskExecution(
        id = taskExecutionId,
        tenantId = workflowExecution.tenantId,
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
    createWorkflowEvent(
        workflowExecution = workflowExecution,
        sequenceNumber = command.sequenceNumber,
        eventType = EventType.TASK_SCHEDULED,
        payload = mapOf(
            "taskType" to command.taskType,
            "taskExecutionId" to command.taskExecutionId,
        )
    )
}
```

#### 1.3 Keep submitTask for Backwards Compatibility (Temporarily)

Don't remove `submitTask` gRPC yet. This allows gradual migration.

---

### Phase 2: SDK Changes

#### 2.1 Remove Async from WorkflowContext Trait

**File**: `sdk/src/workflow/context.rs`

```rust
// Remove #[async_trait] if no async methods remain
pub trait WorkflowContext: Send + Sync {
    // Change from async to sync
    fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFutureRaw;

    fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> TaskFutureRaw;

    // Keep other methods...
    fn sleep(&self, duration: Duration) -> TimerFuture;  // Already sync in new design
    fn schedule_workflow_raw(&self, name: &str, kind: &str, input: Value) -> ChildWorkflowFutureRaw;
    fn promise_raw(&self, name: &str) -> PromiseFutureRaw;
    fn run_raw(&self, name: &str, result: Value) -> OperationFutureRaw;

    // These remain sync
    fn current_time_millis(&self) -> i64;
    fn random_uuid(&self) -> Uuid;
    fn random(&self) -> &dyn DeterministicRandom;
    fn get_raw(&self, key: &str) -> Result<Option<Value>>;  // Make sync
    fn set_raw(&self, key: &str, value: Value) -> Result<()>;  // Make sync
    fn clear(&self, key: &str) -> Result<()>;  // Make sync
}
```

#### 2.2 Update WorkflowContextImpl

**File**: `sdk/src/workflow/context_impl.rs`

Remove `task_submitter` field and update implementation:

```rust
pub struct WorkflowContextImpl<R: CommandRecorder> {
    // REMOVE:
    // task_submitter: Option<Arc<dyn TaskSubmitter>>,

    // Keep existing fields...
}

impl<R: CommandRecorder> WorkflowContext for WorkflowContextImpl<R> {
    fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> TaskFutureRaw {
        let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

        // ALWAYS generate UUID to keep counter synchronized
        let generated_id = self.random_uuid();

        // Check for replay
        if let Some(event) = self.task_events.get(task_seq as usize) {
            return self.handle_task_replay(task_seq, event, task_type, &generated_id);
        }

        // New command: record with client-generated ID
        if let Err(e) = self.record_command(WorkflowCommand::ScheduleTask {
            sequence_number: self.next_sequence(),
            task_type: task_type.to_string(),
            task_execution_id: generated_id,
            input: input.clone(),
            priority_seconds: options.priority_seconds.unwrap_or(0),
            max_retries: options.max_retries,
            timeout_ms: options.timeout.map(|d| d.as_millis() as i64),
            queue: options.queue.clone(),
        }) {
            return TaskFuture::with_error(e.into());
        }

        TaskFuture::new(task_seq, generated_id, self.weak_self())
    }

    fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFutureRaw {
        self.schedule_with_options_raw(task_type, input, ScheduleTaskOptions::default())
    }
}
```

#### 2.3 Remove TaskSubmitter

**Files to delete/modify**:
- `sdk/src/workflow/task_submitter.rs` - Delete entire file
- `sdk/src/client/grpc_task_submitter.rs` - Delete entire file
- `sdk/src/client/mod.rs` - Remove exports
- `sdk/src/workflow/mod.rs` - Remove export

#### 2.4 Update FlovynClient Builder

**File**: `sdk/src/client/builder.rs`

Remove task_submitter configuration.

#### 2.5 Update MockWorkflowContext

**File**: `sdk/src/testing/mock_workflow_context.rs`

Update mock to match new sync API.

#### 2.6 Update All Examples

**Files**: `examples/**/*.rs`

Change from:
```rust
let task = ctx.schedule::<MyTask>(input).await;
let result = task.await?;
```

To:
```rust
let result = ctx.schedule::<MyTask>(input).await?;
```

---

### Phase 3: Testing

#### 3.1 Unit Tests

Add/update tests for:
- UUID counter synchronization during replay
- Deterministic UUID generation across executions
- Command recording only for new commands
- Parallel scheduling sequence numbers

#### 3.2 Integration Tests

- Single task scheduling with client ID
- Multiple parallel tasks with client IDs
- Replay with existing events
- Idempotency on server (duplicate commands)

#### 3.3 E2E Tests

- Full workflow with parallel tasks
- Workflow resume after server restart
- Error handling (task failure, timeout)

---

### Phase 4: Cleanup (Optional)

#### 4.1 Remove submitTask gRPC

**File**: `flovyn-server/src/api/grpc/task_execution.proto`

Remove `SubmitTask` RPC and related messages.

#### 4.2 Update Documentation

- Update SDK README
- Update API documentation
- Add migration guide for breaking changes

---

## TODO List

### Server Changes
- [x] 1.1 Add `task_execution_id` field to ScheduleTaskCommand proto
- [x] 1.2 Update command processor to create TaskExecution from ScheduleTask
- [x] 1.3 Add idempotency check for duplicate task IDs
- [x] 1.4 Add transaction handling for batch commands
- [x] 1.5 Write server unit tests for new behavior
- [x] 1.6 Write server integration tests

### SDK Changes - Core
- [x] 2.1 Remove `async` from `schedule_raw` in WorkflowContext trait
- [x] 2.2 Remove `async` from `schedule_with_options_raw` in WorkflowContext trait
- [x] 2.3 Update WorkflowContextImpl to generate client-side task IDs
- [x] 2.4 Ensure UUID counter advances during replay (critical fix)
- [x] 2.5 Remove `task_submitter` field from WorkflowContextImpl
- [x] 2.6 Update `new_with_task_submitter` constructor (remove submitter param)

### SDK Changes - Cleanup
- [x] 3.1 Delete `sdk/src/workflow/task_submitter.rs`
- [x] 3.2 Delete `sdk/src/client/grpc_task_submitter.rs`
- [x] 3.3 Update `sdk/src/workflow/mod.rs` exports
- [x] 3.4 Update `sdk/src/client/mod.rs` exports
- [x] 3.5 Update FlovynClient builder (remove task_submitter config)

### SDK Changes - Testing Support
- [x] 4.1 Update MockWorkflowContext to sync API
- [x] 4.2 Update test helpers

### SDK Changes - Tests
- [x] 5.1 Fix existing unit tests for sync API
- [x] 5.2 Add test: UUID counter advances during replay
- [x] 5.3 Add test: deterministic UUIDs across executions
- [x] 5.4 Add test: parallel scheduling assigns correct sequences
- [x] 5.5 Update E2E tests for new API
- [x] 5.6 Fix test corpus with valid UUIDs for taskExecutionId fields

### Examples
- [x] 6.1 Update hello-world example
- [x] 6.2 Update ecommerce example
- [x] 6.3 Update data-pipeline example
- [x] 6.4 Update patterns example

### Documentation
- [x] 7.1 Update design doc with implementation notes
- [x] 7.2 Add breaking change notes

---

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Server doesn't accept client IDs | Blocks SDK work | Implement server first, verify with tests |
| UUID counter desync during replay | Silent determinism violations | Add explicit test, always call random_uuid() |
| Breaking changes to user code | User migration effort | Clear error messages, migration guide |
| Transaction failures on batch | Partial state corruption | Single transaction, rollback on failure |
| Idempotency edge cases | Duplicate tasks created | Unique constraint on task_execution_id |

---

## Estimated Effort

| Phase | Effort |
|-------|--------|
| Phase 1: Server Changes | 1-2 days |
| Phase 2: SDK Changes | 2-3 days |
| Phase 3: Testing | 1-2 days |
| Phase 4: Cleanup | 0.5 day |
| **Total** | **5-8 days** |
