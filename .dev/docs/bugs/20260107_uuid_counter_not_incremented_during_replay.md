# Bug Report: UUID Counter Not Incremented During Replay

**Date:** 2026-01-07
**Status:** Fixed & Verified
**Component:** `sdk/src/workflow/context_impl.rs`
**Severity:** Critical

## Problem

When a workflow schedules multiple child workflows sequentially (e.g., batch-processing-workflow), the second and subsequent child workflows receive the **same UUID** as the first child workflow. This causes:
1. The second child workflow to be treated as a duplicate of the first
2. The parent workflow to remain stuck in WAITING state indefinitely
3. Only the first child workflow actually gets created

## Reproduction

1. Run the `batch-processing-workflow` with input:
   ```json
   {"batch_id": "TEST-001", "items": ["apple", "banana", "cherry"]}
   ```

2. Observe the workflow events in the database:
   ```
   #5: CHILD_WORKFLOW_INITIATED - childWorkflowExecutionId: 17ce4999-99d4-5e59-88c7-3192674333e1
   #7: CHILD_WORKFLOW_COMPLETED - (first child completes)
   #9: CHILD_WORKFLOW_INITIATED - childWorkflowExecutionId: 17ce4999-99d4-5e59-88c7-3192674333e1  <-- SAME ID!
   ```

3. Only 1 child workflow exists in the database, parent is stuck in WAITING.

## Root Cause

In `sdk/src/workflow/context_impl.rs`, the `uuid_counter` is always initialized to 0 when a new `WorkflowContextImpl` is created (line 178):

```rust
uuid_counter: AtomicI64::new(0),
```

The `schedule_workflow_raw` method has two code paths:

1. **Replay path** (line 651-736): When a child workflow is found in replay events, it returns the child execution ID from the event **without calling `random_uuid()`**. The `uuid_counter` is NOT incremented.

2. **New command path** (line 738-760): When scheduling a new child workflow, it calls `random_uuid()` which increments the counter.

**The bug flow:**
1. First execution: `uuid_counter = 0`
   - Call `random_uuid()` → counter becomes 1, returns UUID with counter=0
   - Workflow suspends waiting for child

2. Resume execution: `uuid_counter` is **reset to 0** (new context created)
   - Replay: first child found in events → returns ID from event (counter stays 0)
   - New command: call `random_uuid()` → counter becomes 1, returns UUID with counter=0
   - **Same UUID generated for second child!**

## Solution

In the replay path of `schedule_workflow_raw`, increment the `uuid_counter` to keep it synchronized with the UUIDs that were generated during previous executions:

```rust
// In the replay path, after getting child_execution_id from replay event:
// Increment uuid_counter to stay in sync (even though we don't use the value)
let _ = self.uuid_counter.fetch_add(1, Ordering::SeqCst);
```

This ensures that when a new child workflow is scheduled after replay, the counter is at the correct value to generate a unique UUID.

## Affected Workflows

Any workflow that:
- Schedules multiple child workflows sequentially
- Uses `ctx.schedule_workflow_raw()` or `ctx.schedule_workflow()`
- Suspends between child workflow calls (which triggers replay on resume)

Examples:
- `batch-processing-workflow`
- `controlled-parallel-workflow`
- Any workflow with a loop that schedules child workflows

## Fix Applied

Added `uuid_counter` increment in the replay path of `schedule_workflow_raw()`:

```rust
// Get child execution ID
let child_execution_id = initiated_event
    .get_string("childExecutionId")
    .map(|s| Uuid::parse_str(s).unwrap_or(Uuid::nil()))
    .unwrap_or(Uuid::nil());

// IMPORTANT: Increment uuid_counter to stay in sync with UUIDs generated
// in the original execution. Without this, subsequent schedule_workflow_raw
// calls after replay would generate the same UUID as the first child.
let _ = self.uuid_counter.fetch_add(1, Ordering::SeqCst);
```

## Files Changed

- `sdk/src/workflow/context_impl.rs` - Added `uuid_counter` increment in replay path (line 696)
- `sdk/src/workflow/context_impl.rs` - Added regression test `test_multiple_child_workflows_after_replay_get_unique_uuids`

## Why E2E Tests Didn't Catch This

The existing e2e tests in `sdk/tests/e2e/child_workflow_tests.rs` only test:
1. `test_child_workflow_success` - Parent schedules **ONE** child
2. `test_child_workflow_failure` - Parent schedules **ONE** failing child
3. `test_nested_child_workflows` - Chain of grandparent -> parent -> child (**ONE** at each level)

None of these test the scenario of a parent scheduling **multiple children sequentially** with suspension between them. The bug only manifests when:
1. Parent schedules child #1, suspends
2. Child #1 completes, parent resumes (replay happens)
3. Parent schedules child #2 → gets same UUID due to counter reset

## Testing

Add a test that:
1. Creates a context with replay events for one completed child workflow
2. Replays the first child workflow (should increment counter during replay)
3. Schedules a second child workflow
4. Verifies the second child has a different UUID than the first

Also add an e2e test:
1. Parent workflow schedules multiple children in a loop
2. Each child should have unique execution ID
3. Parent should complete successfully after all children complete
