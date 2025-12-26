# Design: Sequence-Based Replay

## Status
**Proposed** - Breaking change, MVP phase

## Problem Statement

The current Flovyn SDK uses different strategies for matching commands to events during replay:

1. **Tasks**: Uses a `consumed_task_execution_ids: HashSet<String>` to track consumed task events, matching by `taskType` and finding the first unconsumed event.

2. **Child Workflows**: Uses name-based matching via `childExecutionName` field.

These approaches have issues:

- **Non-deterministic edge cases**: The consumed set approach can produce unexpected behavior when workflow code changes (adding/removing loop iterations).
- **Implicit matching**: Events are matched by semantic fields rather than position, making determinism violations harder to detect.
- **Difficult debugging**: When violations occur, it's hard to pinpoint which command caused the issue.

## Goals

1. Use strict position-based matching for all commands
2. Detect determinism violations early and clearly
3. Support loops with multiple tasks/child workflows of the same type
4. **Support parallel command execution** (e.g., `awaitAll(schedule("A"), schedule("B"), sleep(1s))`)
5. Maintain correctness during workflow code changes

## Non-Goals

- Backwards compatibility with existing event histories (MVP allows breaking changes)
- Supporting dynamic task/workflow routing at runtime

## Design

### Core Principle: Per-Type Position-Based Matching

Commands are matched to events using a **compound key** of `(command_type, per_type_sequence)`:

```
CommandID::Task(0) ↔ First TaskScheduled event
CommandID::Task(1) ↔ Second TaskScheduled event
CommandID::Timer(0) ↔ First TimerStarted event
CommandID::ChildWorkflow(0) ↔ First ChildWorkflowInitiated event
```

**Why per-type sequences instead of a global sequence?**

A global sequence breaks with parallel commands. Consider:
```rust
// Parallel execution
let a = schedule_async("TaskA");  // Would get global seq=1
let b = schedule_async("TaskB");  // Would get global seq=2
let c = sleep_async(1s);          // Would get global seq=3
await_all(a, b, c);
```

If we used a global cursor on all events, parallel completion order could vary between executions, causing determinism violations during replay.

**Per-type sequences solve this** because:
1. Commands are **created** (and assigned sequences) in deterministic order
2. Each command type has its own counter that increments at creation time
3. Matching uses `(type, per_type_seq)`, not completion order

```rust
// Same parallel execution with per-type sequences
let a = schedule_async("TaskA");  // Task(0)
let b = schedule_async("TaskB");  // Task(1)
let c = sleep_async(1s);          // Timer(0)
await_all(a, b, c);
```

During replay, regardless of which task/timer completed first:
- First `schedule_async("TaskA")` → matches first TaskScheduled (by per-type index 0)
- Second `schedule_async("TaskB")` → matches second TaskScheduled (by per-type index 1)
- First `sleep_async()` → matches first TimerStarted (by per-type index 0)

### CommandID Enum

```rust
/// Identifies a command for replay matching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CommandID {
    Task(u32),           // per-type sequence for tasks
    ChildWorkflow(u32),  // per-type sequence for child workflows
    Timer(u32),          // per-type sequence for timers
    Operation(u32),      // per-type sequence for run() operations
    Promise(u32),        // per-type sequence for promises
    State(u32),          // per-type sequence for state operations
}
```

### Replay Matching Algorithm

During replay:
1. SDK generates commands in the same order as the original execution
2. For each command, SDK computes its `CommandID` using per-type counter
3. SDK looks up the matching event by filtering `existing_events` by type and taking the Nth one
4. If event exists, validate its fields match the command
5. If validation fails → **Determinism Violation**

### SDK Changes

#### 1. Add Per-Type Sequence Counters

Replace global sequence counter with per-type counters:

**Before:**
```rust
struct WorkflowContextImpl {
    sequence_number: AtomicI32,
    consumed_task_execution_ids: HashSet<String>,
    // ...
}
```

**After:**
```rust
struct WorkflowContextImpl {
    // Per-type counters for replay matching
    next_task_seq: AtomicU32,
    next_child_workflow_seq: AtomicU32,
    next_timer_seq: AtomicU32,
    next_operation_seq: AtomicU32,
    next_promise_seq: AtomicU32,
    next_state_seq: AtomicU32,

    // Pre-filtered event lists for O(1) lookup by index
    task_events: Vec<ReplayEvent>,      // TaskScheduled events only
    child_workflow_events: Vec<ReplayEvent>, // ChildWorkflowInitiated events only
    timer_events: Vec<ReplayEvent>,     // TimerStarted events only
    operation_events: Vec<ReplayEvent>, // OperationCompleted events only
    promise_events: Vec<ReplayEvent>,   // PromiseCreated events only
    state_events: Vec<ReplayEvent>,     // StateSet/StateCleared events only
    // ...
}
```

#### 2. Pre-filter Events in Constructor

```rust
impl WorkflowContextImpl {
    fn new(all_events: Vec<ReplayEvent>) -> Self {
        Self {
            task_events: all_events.iter()
                .filter(|e| e.event_type() == EventType::TaskScheduled)
                .cloned().collect(),
            child_workflow_events: all_events.iter()
                .filter(|e| e.event_type() == EventType::ChildWorkflowInitiated)
                .cloned().collect(),
            timer_events: all_events.iter()
                .filter(|e| e.event_type() == EventType::TimerStarted)
                .cloned().collect(),
            operation_events: all_events.iter()
                .filter(|e| e.event_type() == EventType::OperationCompleted)
                .cloned().collect(),
            promise_events: all_events.iter()
                .filter(|e| e.event_type() == EventType::PromiseCreated)
                .cloned().collect(),
            state_events: all_events.iter()
                .filter(|e| matches!(e.event_type(), EventType::StateSet | EventType::StateCleared))
                .cloned().collect(),
            next_task_seq: AtomicU32::new(0),
            next_child_workflow_seq: AtomicU32::new(0),
            // ... initialize all counters to 0
        }
    }
}
```

#### 3. New Matching Logic for Tasks

```rust
async fn schedule_with_options_raw(&self, task_type: &str, input: Value, options: ScheduleTaskOptions) -> Result<Value> {
    // Get per-type sequence and increment
    let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst) as usize;

    // Look for event at this per-type index
    let event = self.task_events.get(task_seq);

    match event {
        Some(e) => {
            // Validate task type matches
            let event_task_type = e.get_string("taskType").unwrap_or_default();
            if event_task_type != task_type {
                return Err(FlovynError::DeterminismViolation {
                    message: format!(
                        "Task type mismatch at Task({}): expected '{}', got '{}'",
                        task_seq, task_type, event_task_type
                    ),
                });
            }

            // Get task execution ID from the scheduled event
            let task_execution_id = e.get_string("taskExecutionId")
                .ok_or_else(|| FlovynError::Other("Missing taskExecutionId".into()))?;

            // Look for terminal event (completed/failed) for this task
            let terminal = self.find_terminal_task_event(task_execution_id);

            match terminal {
                Some(t) if t.event_type() == EventType::TaskCompleted => {
                    Ok(t.get("result").cloned().unwrap_or(Value::Null))
                }
                Some(t) if t.event_type() == EventType::TaskFailed => {
                    Err(FlovynError::TaskFailed(t.get_string("error").unwrap_or("").into()))
                }
                None => Err(FlovynError::Suspended { reason: "Task running".into() }),
                _ => unreachable!(),
            }
        }
        None => {
            // New command - submit task and record
            let task_execution_id = self.submit_task(task_type, input.clone(), options).await?;
            self.record_command(WorkflowCommand::ScheduleTask {
                sequence_number: self.next_global_sequence(),
                task_type: task_type.to_string(),
                task_execution_id,
                input,
                priority_seconds: options.priority_seconds,
            })?;
            Err(FlovynError::Suspended { reason: "Task scheduled".into() })
        }
    }
}
```

**Note:** The `sequence_number` in the command is still a global sequence for the server's event ordering. The `task_seq` (per-type index) is only used for replay matching.

#### 4. New Matching Logic for Child Workflows

```rust
async fn schedule_workflow_raw(&self, name: &str, kind: &str, input: Value) -> Result<Value> {
    // Get per-type sequence and increment
    let cw_seq = self.next_child_workflow_seq.fetch_add(1, Ordering::SeqCst) as usize;

    let event = self.child_workflow_events.get(cw_seq);

    match event {
        Some(e) => {
            // Validate child workflow name and kind match
            let event_name = e.get_string("childExecutionName").unwrap_or_default();
            let event_kind = e.get_string("childWorkflowKind").unwrap_or_default();

            if event_name != name {
                return Err(FlovynError::DeterminismViolation {
                    message: format!(
                        "Child workflow name mismatch at ChildWorkflow({}): expected '{}', got '{}'",
                        cw_seq, name, event_name
                    ),
                });
            }

            if event_kind != kind {
                return Err(FlovynError::DeterminismViolation {
                    message: format!(
                        "Child workflow kind mismatch at ChildWorkflow({}): expected '{}', got '{}'",
                        cw_seq, kind, event_kind
                    ),
                });
            }

            // Look for terminal event by childExecutionName
            let terminal = self.find_terminal_child_workflow_event(name);

            match terminal {
                Some(t) if t.event_type() == EventType::ChildWorkflowCompleted => {
                    Ok(t.get("output").cloned().unwrap_or(Value::Null))
                }
                Some(t) if t.event_type() == EventType::ChildWorkflowFailed => {
                    Err(FlovynError::ChildWorkflowFailed { /* ... */ })
                }
                None => Err(FlovynError::Suspended { reason: "Child workflow running".into() }),
                _ => unreachable!(),
            }
        }
        None => {
            // New command - record and suspend
            let child_execution_id = self.random_uuid();
            self.record_command(WorkflowCommand::ScheduleChildWorkflow {
                sequence_number: self.next_global_sequence(),
                name: name.to_string(),
                kind: Some(kind.to_string()),
                definition_id: None,
                child_execution_id,
                input,
                task_queue: String::new(),
                priority_seconds: 0,
            })?;
            Err(FlovynError::Suspended { reason: "Child workflow initiated".into() })
        }
    }
}
```

#### 5. Helper Methods

Add to `WorkflowContextImpl`:

```rust
impl WorkflowContextImpl {
    /// Get event at a specific sequence number
    fn get_event_at_sequence(&self, seq: i32) -> Option<&ReplayEvent> {
        self.existing_events.iter().find(|e| e.sequence_number() == seq)
    }

    /// Find terminal event for a task by taskExecutionId
    fn find_terminal_task_event(&self, task_execution_id: &str) -> Option<&ReplayEvent> {
        self.existing_events.iter()
            .filter(|e| {
                let id = e.get_string("taskExecutionId").or_else(|| e.get_string("taskId"));
                id == Some(task_execution_id) && e.event_type().is_task_terminal()
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Find terminal event for a child workflow by name
    fn find_terminal_child_workflow_event(&self, name: &str) -> Option<&ReplayEvent> {
        self.existing_events.iter()
            .filter(|e| {
                e.get_string("childExecutionName") == Some(name)
                    && e.event_type().is_child_workflow_terminal()
            })
            .max_by_key(|e| e.sequence_number())
    }
}
```

### Server Changes

#### 1. Event Sequence Numbers Must Be Globally Unique Per Execution

The server already stores events with sequence numbers. Verify that:

- `TaskScheduled` events get sequence numbers assigned by the SDK (from command)
- `ChildWorkflowInitiated` events get sequence numbers assigned by the SDK (from command)
- Terminal events (`TaskCompleted`, `ChildWorkflowCompleted`, etc.) use their own sequence numbers (not matching the initiating event)

**No schema changes required** - the current `WorkflowEvent.sequence_number` field is sufficient.

#### 2. Determinism Violation Handling

When SDK reports a `DETERMINISM_VIOLATION` failure type:

```protobuf
message FailWorkflowCommand {
  string error = 1;
  string stack_trace = 2;
  string failure_type = 3; // "DETERMINISM_VIOLATION"
}
```

The server should:
1. Mark the workflow execution as failed with `DETERMINISM_VIOLATION` status
2. **Do not retry** - this is a non-recoverable error
3. Log the violation for debugging

**Current behavior is already correct** - `DETERMINISM_VIOLATION` is already a defined failure type.

### Determinism Validation Matrix

| Command Type | Event Type | Validated Fields |
|--------------|------------|------------------|
| `ScheduleTask` | `TaskScheduled` | `taskType` |
| `ScheduleChildWorkflow` | `ChildWorkflowInitiated` | `childExecutionName`, `childWorkflowKind` |
| `RecordOperation` | `OperationCompleted` | `operationName` |
| `SetState` | `StateSet` | `key` |
| `ClearState` | `StateCleared` | `key` |
| `StartTimer` | `TimerStarted` | `timerId` |
| `CancelTimer` | `TimerCancelled` | `timerId` |
| `CreatePromise` | `PromiseCreated` | `promiseId` |
| `ResolvePromise` | `PromiseResolved` | `promiseId` |

### Loop Handling Example

```rust
// Workflow code
for i in 0..3 {
    ctx.schedule::<ProcessTask>(items[i]).await?;
}
```

**First execution** (no events):
```
Task(0): ScheduleTask{taskType="ProcessTask"} → TaskScheduled
Task(1): ScheduleTask{taskType="ProcessTask"} → TaskScheduled
Task(2): ScheduleTask{taskType="ProcessTask"} → TaskScheduled
```

**Replay** (events exist):
```
Task(0): Check task_events[0] has taskType="ProcessTask" ✓
Task(1): Check task_events[1] has taskType="ProcessTask" ✓
Task(2): Check task_events[2] has taskType="ProcessTask" ✓
```

**Workflow extended** (code changed to 4 iterations):
```
Task(0): Match ✓
Task(1): Match ✓
Task(2): Match ✓
Task(3): No event at task_events[3] → New command (OK, workflow extended)
```

**Workflow shortened** (code changed to 2 iterations):
```
Task(0): Match ✓
Task(1): Match ✓
// Workflow completes, but task_events[2] was never replayed
// Server may detect orphaned commands, or workflow completes differently
```

### Parallel Execution Example

```rust
// Parallel task scheduling with awaitAll
let a = ctx.schedule_async::<TaskA>(input1);  // Task(0)
let b = ctx.schedule_async::<TaskB>(input2);  // Task(1)
let c = ctx.sleep_async(Duration::from_secs(5)); // Timer(0)
let results = ctx.await_all(a, b, c).await?;
```

**First execution** (commands created in code order, completed in any order):
```
Event history (creation order):
  TaskScheduled{seq=1, taskType="TaskA"}      ← Task(0)
  TaskScheduled{seq=2, taskType="TaskB"}      ← Task(1)
  TimerStarted{seq=3, timerId="sleep-1"}      ← Timer(0)

Event history (completion order varies - tasks might complete in any order):
  TaskCompleted{seq=4, taskId=..., output=...}
  TaskCompleted{seq=5, taskId=..., output=...}
  TimerFired{seq=6, timerId="sleep-1"}
```

**Replay** (completion order might differ, but matching still works):
```
schedule_async::<TaskA>() → Task(0) → task_events[0] ✓ (taskType="TaskA")
schedule_async::<TaskB>() → Task(1) → task_events[1] ✓ (taskType="TaskB")
sleep_async()             → Timer(0) → timer_events[0] ✓ (timerId="sleep-1")

Terminal events matched by taskExecutionId/timerId, not by completion order.
```

**Why this works:**
1. Per-type sequences are assigned at **command creation time**, not completion time
2. `Task(0)` always refers to the first `schedule_async` call, regardless of which task completes first
3. Terminal events (TaskCompleted, TimerFired) are matched by their ID (taskExecutionId, timerId), not position

### Migration Path

Since we're at MVP stage with breaking changes allowed:

1. **Delete existing event histories** or mark them as incompatible
2. **Update SDK** with new matching logic
3. **Update server** to expect new behavior
4. All new workflow executions use sequence-based matching

### Test Cases

See [Sequence-Based Replay Testing](./sequence-based-replay-testing.md) for the comprehensive test strategy. Key tests:

1. **Loop scheduling same task type** - verify position-based matching
2. **Task type mismatch** - verify determinism violation is raised
3. **Child workflow name mismatch** - verify determinism violation is raised
4. **Workflow code extension** - verify new commands are allowed
5. **Workflow code reduction** - verify behavior is correct

## Alternatives Considered

### 1. Keep Current Approach with Better Documentation

**Rejected**: The consumed-set approach is fundamentally non-positional, making it harder to reason about and debug.

### 2. Hybrid: Position + Fallback to Type Matching

**Rejected**: Adds complexity without clear benefit. Pure position-based approach is simpler and more predictable.

### 3. Content-Addressable Events (Hash-Based)

**Rejected**: Over-engineered for MVP. Could be considered for future versioning support.

## References

- [Flovyn Edge Case Tests](./../research/edge-case-tests.md)
- [Sequence-Based Replay Testing](./sequence-based-replay-testing.md)
