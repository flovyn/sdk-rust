# Bug Report: Rust SDK Replay Sequence Numbering Issue

**Date Identified**: 2025-12-14
**Status**: FIXED

## Summary

The Rust SDK's `WorkflowContextImpl` had incorrect sequence numbering during workflow replay/resume, causing workflow executions to fail after suspending for timers, tasks, or promises.

## Error Observed

When a workflow suspended (e.g., for a timer) and was resumed after the event occurred, the server returned:

```
Event validation failed for workflow: Sequence gap: expected 5, got 1
```

## Root Cause

The `WorkflowContextImpl` always started the `sequence_number` counter at 1, regardless of existing replay events. This caused:

1. **Sequence mismatch**: On resume, the first command sent had sequence 1 instead of continuing from where replay left off
2. **Timer ID mismatch**: Timer IDs were generated using sequence numbers (`timer-{sequence}`), so they wouldn't match during replay
3. **Command duplication**: Commands were recorded even during replay when they should have been skipped

## Comparison with Kotlin SDK

The Kotlin SDK correctly handles this:

```kotlin
class WorkflowContextImpl(...) {
    private var nextSequence = existingEvents.size  // Start from replay count
    private var sleepCallCounter: Int = 0  // Separate counter for timer IDs

    override suspend fun sleep(duration: Duration) {
        val timerId = "sleep-${++sleepCallCounter}"  // Not based on sequence

        // Check if timer already fired (replay)
        val firedEvent = existingEvents.find { it.type == TIMER_FIRED && it.data["timerId"] == timerId }
        if (firedEvent != null) {
            return  // No new command - just return
        }

        // Only record command if NOT replaying
        if (existingEvents.none { it.type == TIMER_STARTED && it.data["timerId"] == timerId }) {
            commandRecorder.recordCommand(...)
        }

        throw WorkflowSuspendException(...)
    }
}
```

## Fix Applied

1. **Sequence numbering**: Initialize `sequence_number` to `existingEvents.len() + 1`
   ```rust
   let initial_sequence = (existing_events.len() as i32) + 1;
   sequence_number: AtomicI32::new(initial_sequence),
   ```

2. **Separate timer counter**: Added `sleep_call_counter` for deterministic timer IDs
   ```rust
   sleep_call_counter: AtomicI32::new(0),
   ```

3. **Timer replay logic**: Find events by timer ID, not sequence; skip command recording during replay
   ```rust
   async fn sleep(&self, duration: Duration) -> Result<()> {
       let sleep_count = self.sleep_call_counter.fetch_add(1, Ordering::SeqCst) + 1;
       let timer_id = format!("sleep-{}", sleep_count);

       // Check for TIMER_FIRED event (already completed)
       if self.find_event_by_field(EventType::TimerFired, "timerId", &timer_id).is_some() {
           return Ok(());  // No command, just return
       }

       // Only record command if not already started
       if self.find_event_by_field(EventType::TimerStarted, "timerId", &timer_id).is_none() {
           self.record_command(...)?;
       }

       Err(FlovynError::Suspended { ... })
   }
   ```

4. **Task scheduling replay**: Added `consumed_task_execution_ids` tracking for handling multiple tasks with the same type

## Files Changed

- `sdk/src/workflow/context_impl.rs` - Main fix location
- `sdk/src/error.rs` - Added `TaskFailed` error variant

## Tests Added

- `test_sleep_returns_immediately_when_timer_fired` - Verifies timer replay works
- `test_multiple_tasks_same_type_each_consume_one_event` - Verifies task tracking
- `test_task_still_running_suspends` - Verifies pending task handling
- `test_task_failed_returns_error` - Verifies task failure propagation
- `test_schedule_raw_returns_completed_from_replay` - Verifies task replay

## Promise Replay Fix (Applied 2025-12-14)

The original fix addressed timers and tasks. The promise replay fix was later applied using the same pattern:

### Changes Made

1. **Look up events by promise name, not sequence**: Changed `find_event(sequence, ...)` to `find_event_by_field(EventType::*, "promiseName", name)`

2. **Check for resolved/rejected events first**: Before recording commands, check if the promise was already resolved or rejected during replay

3. **Skip command recording during replay**: Only record `CREATE_PROMISE` command if no `PROMISE_CREATED` event exists

4. **Added `PromiseRejected` error variant**: New error type for handling rejected promises during replay

### New Unit Tests Added

- `test_promise_returns_immediately_when_resolved_during_replay`
- `test_promise_returns_error_when_rejected_during_replay`
- `test_promise_suspends_when_created_but_not_resolved_during_replay`

## Child Workflow Replay Fix (Applied 2025-12-14)

The child workflow scheduling had the same replay issue. Fixed using the same pattern:

### Changes Made

1. **Look up events by childExecutionName, not sequence**: Changed `find_event(sequence, ...)` to `find_event_by_field(EventType::*, "childExecutionName", name)`
   - Server uses `childExecutionName` field (not `name`) in all child workflow events
   - Server uses `output` field (not `result`) for completed child workflow results
   - Server uses `childworkflowExecutionId` field for execution IDs

2. **Use correct event types**: Use `ChildWorkflowInitiated` (not `ChildWorkflowStarted`) to check if a child workflow was already scheduled

3. **Check for completed/failed events first**: Before recording commands, check if the child workflow was already completed or failed during replay

4. **Skip command recording during replay**: Only record `SCHEDULE_CHILD_WORKFLOW` command if no `CHILD_WORKFLOW_INITIATED` event exists

### New Unit Tests Added

- `test_schedule_workflow_returns_immediately_when_completed_during_replay`
- `test_schedule_workflow_returns_error_when_failed_during_replay`
- `test_schedule_workflow_suspends_when_initiated_but_not_completed_during_replay`

## Related Files

- Kotlin SDK reference: `sdk/kotlin/src/main/kotlin/ai/flovyn/sdk/kotlin/workflow/WorkflowContextImpl.kt`
- Server E2E tests: `server/app/src/test/kotlin/ai/flovyn/e2e/DurableSleepE2ETest.kt`
