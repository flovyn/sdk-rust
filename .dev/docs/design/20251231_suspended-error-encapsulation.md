# Design: Encapsulating Suspended Error

**Date**: 2025-12-31
**Status**: Proposal
**Related**: Bug discovered in `streaming_tests::test_workflow_failed_event_published`

## Problem Statement

The `FlovynError::Suspended` variant is a **system control flow mechanism** used internally by the SDK to signal that a workflow needs to suspend (e.g., waiting for a task, timer, or child workflow to complete). However, it's exposed in the public `FlovynError` enum, which allows developers to catch and handle it incorrectly.

### Current Implementation

```rust
// sdk/src/error.rs
pub enum FlovynError {
    /// Workflow is suspended waiting for external event
    #[error("Workflow suspended: {reason}")]
    Suspended { reason: String },  // ← PUBLIC AND CATCHABLE

    TaskFailed(String),
    TaskCancelled,
    // ... other application errors
}
```

```rust
// sdk/src/workflow/future.rs - TaskFuture::poll()
fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
    // ... check for result ...

    // Not ready yet - signal workflow suspension
    Poll::Ready(Err(FlovynError::Suspended {
        reason: format!("Waiting for task {} to complete", self.task_execution_id),
    }))
}
```

### The Danger

A developer can write:

```rust
async fn execute(&self, ctx: &dyn WorkflowContext, input: Input) -> Result<Output> {
    let result = ctx.schedule("some-task", input).await;

    match result {
        Ok(v) => Ok(v),
        Err(FlovynError::Suspended { .. }) => {
            // DANGEROUS: Developer catches suspension and returns a default
            // The workflow completes prematurely without waiting for the task!
            Ok(Default::default())
        }
        Err(e) => Err(e),
    }
}
```

This caused the flaky `test_workflow_failed_event_published` test - the test workflow was catching `Suspended` thinking it was a task error.

## Design Goals

1. **Prevent user code from catching `Suspended`** - It should be impossible or extremely difficult
2. **Maintain ergonomic API** - Developers should still be able to handle real errors (`TaskFailed`, etc.)
3. **Backward compatibility** - Minimize breaking changes if possible
4. **Type safety** - The compiler should enforce correct behavior

## Options Analysis

### Option 1: Separate Error Hierarchies (Recommended)

Split errors into control flow (internal) and application errors (public):

```rust
// Internal - not exposed to users
pub(crate) enum WorkflowControlFlow {
    Suspended { reason: String },
}

// Public - what users can handle
#[derive(Debug, thiserror::Error)]
pub enum FlovynError {
    #[error("Task failed: {0}")]
    TaskFailed(String),

    #[error("Task cancelled")]
    TaskCancelled,

    // ... other application errors
}

// Context methods return a combined type
pub(crate) enum InternalResult<T> {
    Ok(T),
    Err(FlovynError),
    Suspended { reason: String },
}

// But user-facing trait uses Result<T, FlovynError>
// The executor handles InternalResult internally
```

**Pros:**
- Clean separation of concerns
- Type-safe - users literally cannot match on `Suspended`
- Clear API contract

**Cons:**
- Breaking change
- Requires internal refactoring

### Option 2: Non-Exhaustive + Hidden Variant

```rust
#[non_exhaustive]
pub enum FlovynError {
    #[doc(hidden)]
    Suspended { reason: String },

    TaskFailed(String),
    // ...
}
```

**Pros:**
- Minimal code change
- `#[doc(hidden)]` discourages use

**Cons:**
- Users can still match with `_` wildcard and act on it
- Not truly enforced by the type system

### Option 3: Use Proper `Poll::Pending`

Refactor futures to return `Poll::Pending` instead of `Poll::Ready(Err(Suspended))`:

```rust
impl<T: DeserializeOwned> Future for TaskFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check for result...
        if let Some(result) = self.find_result() {
            return Poll::Ready(result);
        }

        // Not ready - return Pending (standard async pattern)
        // Register waker for when result becomes available
        self.register_waker(cx.waker().clone());
        Poll::Pending
    }
}
```

The executor would then need to handle waking futures when events arrive.

**Pros:**
- Standard Rust async pattern
- No `Suspended` error needed at all
- Natural integration with async runtimes

**Cons:**
- Significant refactoring
- Requires waker management infrastructure
- May conflict with deterministic replay requirements (workflows must execute synchronously during replay)

### Option 4: Sealed Error Trait

Use a sealed trait to prevent external implementations:

```rust
mod private {
    pub trait Sealed {}
}

pub trait WorkflowError: private::Sealed + std::error::Error {}

// Only FlovynError implements Sealed
impl private::Sealed for FlovynError {}
impl WorkflowError for FlovynError {}
```

This doesn't directly solve the problem but could be part of a larger solution.

### Option 5: Return Type Transformation

Change the return type of context methods:

```rust
// Instead of Result<T, FlovynError>, return:
pub enum TaskOutcome<T> {
    Completed(T),
    Failed(TaskError),  // TaskError ⊂ FlovynError
}

// The impl Future for TaskFuture returns TaskOutcome
// Suspended is never exposed to user code
```

**Pros:**
- Type-safe
- Clear semantics

**Cons:**
- Different return type for different operations
- More complex API

## Recommendation

**Option 1 (Separate Error Hierarchies)** is the cleanest long-term solution.

However, for a quick fix with minimal breaking changes, consider **Option 3** if the deterministic replay can be preserved, or **Option 2** as a temporary measure with clear documentation.

## Implementation Plan

### Phase 1: Immediate Fix (Low Risk)
1. Add `#[doc(hidden)]` to `Suspended` variant
2. Add strong documentation warning against catching it
3. Update test workflows to NOT catch `Suspended`

### Phase 2: Proper Fix (SDK 2.0)
1. Refactor to separate control flow from application errors
2. Update all internal usages
3. Provide migration guide

## Migration Guide (for Phase 2)

```rust
// Before (SDK 1.x) - WRONG but possible
match ctx.schedule(...).await {
    Err(FlovynError::Suspended { .. }) => { /* user could catch this */ }
    _ => {}
}

// After (SDK 2.0) - Suspended is not in FlovynError
match ctx.schedule(...).await {
    Err(FlovynError::TaskFailed(msg)) => { /* handle task failure */ }
    Err(FlovynError::TaskCancelled) => { /* handle cancellation */ }
    Ok(result) => { /* success */ }
}
// Suspended is handled internally by the SDK - user code never sees it
```

## Open Questions

1. Should we also hide `DeterminismViolation`? It's also a system error, but developers might want to log/handle it.
2. How does this affect the FFI layer (flovyn-ffi)?
3. Should `WorkflowCancelled` be user-catchable or system-only?
