# Edge Case Tests for Event Sourcing/Replaying

This document explores edge cases in workflow replay and how the SDK handles deterministic execution.

> **Design Document:** See [Sequence-Based Replay Design](../design/sequence-based-replay.md) for the proposed solution.

---

## 1. Multiple Child Workflows with Same Type in Loops

**Question:** How does the system handle launching multiple sub-workflows of the same type (e.g., in a loop) with potentially different inputs/outputs?

**Current SDK Behavior:**

The SDK uses a **name-based matching** strategy for child workflows:

- Each child workflow is identified by a unique `name` parameter (not just the workflow type)
- During replay, the SDK matches `ChildWorkflowCompleted`/`ChildWorkflowFailed` events by the `childExecutionName` field
- Child execution IDs are generated deterministically using `ctx.random_uuid()` (seeded by parent workflow execution ID)

**Example:**
```rust
// In a loop, each child must have a unique name
for i in 0..3 {
    let name = format!("process-item-{}", i);
    ctx.schedule_workflow::<ProcessItem>(&name, item[i]).await?;
}
```

**Test Cases Needed:**
- [ ] Loop spawning 3 child workflows of same type with unique names
- [ ] Loop spawning child workflows with same name (should fail determinism on replay)
- [ ] Nested child workflows (parent spawns child, child spawns grandchild)
- [ ] Child workflow with same name but different inputs on separate executions

---

## 2. Multiple Tasks with Same Type in Loops

**Question:** How does the system handle launching multiple tasks of the same type (e.g., in a loop) with potentially different inputs?

**Solution: Per-Type Sequence Matching**

The SDK uses **per-type sequence counters** for replay matching:

- Each command type (Task, Timer, ChildWorkflow, etc.) has its own sequence counter
- During replay, the Nth `schedule()` call matches the Nth `TaskScheduled` event
- Terminal events (TaskCompleted/TaskFailed) are matched by `taskExecutionId`, not position

**Algorithm:**
```
1. Get per-type sequence: task_seq = next_task_seq.fetch_add(1)
2. Look up task_events[task_seq]
3. If event exists: validate taskType matches, find terminal event by taskExecutionId
4. If no event: new command, submit to server
```

**Example:**
```rust
// Each schedule() call gets the next per-type sequence
for item in items {
    let result = ctx.schedule::<ProcessTask>(item).await?;
    // 1st call → Task(0) → task_events[0]
    // 2nd call → Task(1) → task_events[1]
    // etc.
}
```

**Test Cases Needed:**
- [ ] Loop scheduling 5 tasks of same type - verify per-type sequence matching
- [ ] Replay after adding one more iteration to loop (should extend, not violate)
- [ ] Replay after removing one iteration from loop (behavior depends on implementation)
- [ ] Mixed task types in a loop (alternating between TaskA and TaskB)
- [ ] Task scheduling inside conditional branches

---

## 3. Parallel Command Execution

**Question:** How does the system handle parallel execution of commands (e.g., `awaitAll`) where multiple tasks/timers run concurrently and may complete in different orders?

**Challenge:**

With parallel execution, the completion order of commands can vary between executions:
```rust
let a = ctx.schedule_async::<TaskA>(input1);  // Might complete 2nd
let b = ctx.schedule_async::<TaskB>(input2);  // Might complete 1st
let c = ctx.sleep_async(Duration::from_secs(5)); // Might complete 3rd
let results = ctx.await_all(a, b, c).await?;
```

If completion order determined replay matching, the same code could fail replay due to non-deterministic timing.

**Solution: Per-Type Sequence at Creation Time**

The SDK assigns per-type sequences at **command creation time**, not completion time:

```rust
schedule_async::<TaskA>()  // Task(0) - assigned immediately
schedule_async::<TaskB>()  // Task(1) - assigned immediately
sleep_async()              // Timer(0) - assigned immediately
```

During replay:
- First `schedule_async::<TaskA>()` always matches `task_events[0]` (Task(0))
- First `schedule_async::<TaskB>()` always matches `task_events[1]` (Task(1))
- First `sleep_async()` always matches `timer_events[0]` (Timer(0))

Terminal events (TaskCompleted, TimerFired) are matched by their ID (taskExecutionId, timerId), not by completion order.

**Test Cases Needed:**
- [ ] `awaitAll` with 3 tasks - verify replay works regardless of completion order
- [ ] `awaitAll` with mixed commands (tasks + timer) - verify independent type matching
- [ ] Parallel tasks where faster one completes first - verify deterministic replay
- [ ] Parallel child workflows - verify name-based matching still works
- [ ] Nested parallel execution (parallel tasks inside parallel workflows)

---

## Deferred: Investigation Mode

> **Status:** Deferred for future implementation

The server could have an investigation/validation mode that:
- Tracks ongoing history during execution
- Validates that state changes are legal
- Rejects illegal state transitions
- Enabled by default in development mode to detect SDK implementation bugs

This would complement the client-side determinism validation with server-side verification.

---

## Solution

The [Sequence-Based Replay Design](../design/sequence-based-replay.md) addresses these edge cases by:

1. **Per-type sequence matching**: Each command type has its own sequence counter
2. **Creation-time assignment**: Sequences assigned when command is created, not completed
3. **Strict validation**: Task type, child workflow name/kind must match at each position
4. **Clear violations**: Determinism violations include CommandID (e.g., "Task(2)") and field mismatch details
5. **Parallel support**: Parallel commands work because sequences are assigned at creation time, not completion time

**Key insight:** The per-type sequence approach solves the parallel execution problem because completion order doesn't affect matching - only creation order matters.

See also: [Sequence-Based Replay Testing](../design/sequence-based-replay-testing.md) for the comprehensive test strategy.
