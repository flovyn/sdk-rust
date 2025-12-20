# Bug: join_all Not Completing When All Tasks Are Done

**Date**: 2025-12-20
**Severity**: High
**Component**: Server/SDK Integration
**Status**: FIXED

## Summary

When a workflow uses `join_all` to wait for multiple parallel tasks, the workflow may never complete because the server does not resume the workflow after a task completes while the workflow is suspended.

## Root Cause

The server's check for new events after setting a workflow to WAITING state was comparing `latest_seq > final_sequence`, but this didn't catch events that arrived BETWEEN when the server got `existing_events` and when it called `append_batch`.

**Timeline of the bug:**
1. SDK gets `existing_events` (seq 1-5)
2. 2nd task completes, inserts TASK_COMPLETED (seq 6)
3. SDK's `append_batch` inserts WORKFLOW_SUSPENDED (seq 7)
4. `final_sequence = 7`, `latest_seq = 7`
5. `7 > 7` is false → no resume!

The 2nd task's TASK_COMPLETED (seq 6) was inserted after we got existing_events but before our append_batch, so it got a sequence number BEFORE our events. The comparison didn't detect this.

## Fix

Changed the resume check to compare against **expected** final sequence instead of actual:

```rust
// Calculate what we expected the final sequence to be
let expected_final_seq = existing_events
    .last()
    .map(|e| e.sequence_number)
    .unwrap_or(0)
    + events.len() as i32;

// If actual > expected, there are events we missed
if final_sequence > expected_final_seq {
    // Resume the workflow
}
```

Now if we expected seq 6 (5 existing + 1 new) but got seq 7, we know there's an extra event (seq 6) that we didn't know about, and we resume the workflow.

## Files Changed

- `src/api/grpc/workflow_dispatch.rs`: Fixed resume check logic in `submit_workflow_commands`

## Testing

- All 16 integration tests pass
- `test_parallel_task_large_batch` (10 tasks with join_all) passes
- `test_parallel_task_completion_race_condition` (3 tasks with join_all) passes

## Related

- Server bug #004: Fixed - atomic sequence assignment with advisory locks
- Server bug #005: Fixed - this bug was the remaining issue
