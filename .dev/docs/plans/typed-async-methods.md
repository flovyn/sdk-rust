# Implementation Plan: Typed Async Method Wrappers

## Overview

Add typed convenience wrappers for the raw async methods in `WorkflowContext`. These provide compile-time type safety by leveraging `TaskDefinition` and `WorkflowDefinition` traits.

**Related:** [Parallel Execution Implementation](parallel-execution-implementation.md) - Phase 2

## Methods to Implement

| Method | Input | Output | Description |
|--------|-------|--------|-------------|
| `schedule_async<T>` | `T::Input` | `TaskFuture<T::Output>` | Schedule typed task |
| `schedule_async_with_options<T>` | `T::Input`, options | `TaskFuture<T::Output>` | Schedule typed task with options |
| `schedule_workflow_async<W>` | `name`, `W::Input` | `ChildWorkflowFuture<W::Output>` | Schedule typed child workflow |
| `promise_async<T>` | `name` | `PromiseFuture<T>` | Create typed promise |

---

## Phase 1: Add Method Signatures to WorkflowContext Trait

**File:** `sdk/src/workflow/context.rs`

Add the typed method signatures:

```rust
/// Schedule a typed task asynchronously
fn schedule_async<T: TaskDefinition>(&self, input: T::Input) -> TaskFuture<T::Output>
where
    T::Input: Serialize,
    T::Output: DeserializeOwned;

/// Schedule a typed task with options
fn schedule_async_with_options<T: TaskDefinition>(
    &self,
    input: T::Input,
    options: ScheduleTaskOptions,
) -> TaskFuture<T::Output>
where
    T::Input: Serialize,
    T::Output: DeserializeOwned;

/// Schedule a typed child workflow asynchronously
fn schedule_workflow_async<W: WorkflowDefinition>(
    &self,
    name: &str,
    input: W::Input,
) -> ChildWorkflowFuture<W::Output>
where
    W::Input: Serialize,
    W::Output: DeserializeOwned;

/// Create a typed promise
fn promise_async<T: DeserializeOwned>(&self, name: &str) -> PromiseFuture<T>;
```

---

## Phase 2: Implement in WorkflowContextImpl

**File:** `sdk/src/workflow/context_impl.rs`

Each typed method wraps the raw version with serialization:

```rust
fn schedule_async<T: TaskDefinition>(&self, input: T::Input) -> TaskFuture<T::Output>
where
    T::Input: Serialize,
    T::Output: DeserializeOwned,
{
    let input_value = match serde_json::to_value(&input) {
        Ok(v) => v,
        Err(e) => return TaskFuture::with_error(FlovynError::Serialization(e)),
    };

    // Get raw future and convert output type
    let raw_future = self.schedule_async_raw(T::task_type(), input_value);

    // TaskFuture<Value> needs to become TaskFuture<T::Output>
    // This is handled by the generic parameter - both use same underlying state
    TaskFuture {
        task_seq: raw_future.task_seq,
        task_execution_id: raw_future.task_execution_id,
        context: raw_future.context,
        state: raw_future.state,
        _marker: PhantomData,
    }
}
```

**Note:** The `TaskFuture<T>` already handles deserialization in its `poll()` implementation via `serde_json::from_value()`.

---

## Phase 3: Implement in MockWorkflowContext

**File:** `sdk/src/testing/mock_workflow_context.rs`

Mirror the implementations for the mock context:

```rust
fn schedule_async<T: TaskDefinition>(&self, input: T::Input) -> TaskFuture<T::Output>
where
    T::Input: Serialize,
    T::Output: DeserializeOwned,
{
    let input_value = match serde_json::to_value(&input) {
        Ok(v) => v,
        Err(e) => return TaskFuture::with_error(FlovynError::Serialization(e)),
    };

    let raw_future = self.schedule_async_raw(T::task_type(), input_value);
    TaskFuture {
        task_seq: raw_future.task_seq,
        task_execution_id: raw_future.task_execution_id,
        context: raw_future.context,
        state: raw_future.state,
        _marker: PhantomData,
    }
}
```

---

## Phase 4: Unit Tests

**File:** `sdk/src/workflow/context_impl.rs` (module tests)

```rust
#[test]
fn test_schedule_async_typed_serializes_input() {
    // Verify typed method correctly serializes input
}

#[test]
fn test_schedule_async_typed_returns_typed_future() {
    // Verify return type is correctly typed
}

#[test]
fn test_schedule_async_serialization_error_returns_error_future() {
    // Verify serialization errors are handled gracefully
}

#[test]
fn test_schedule_workflow_async_typed() {
    // Verify typed child workflow scheduling
}

#[test]
fn test_promise_async_typed() {
    // Verify typed promise creation
}
```

---

## TODO List

### Phase 1: Trait Signatures
- [x] Add `schedule_async<T>` signature to `WorkflowContextExt` trait
- [x] Add `schedule_async_with_options<T>` signature to `WorkflowContextExt` trait
- [x] Add `schedule_workflow_async<W>` signature to `WorkflowContextExt` trait
- [x] Add `promise_async<T>` signature to `WorkflowContextExt` trait
- [x] Add required trait bounds imports

**Note:** Methods were added to `WorkflowContextExt` (extension trait) instead of `WorkflowContext` to maintain dyn-compatibility, since generic methods cannot be in object-safe traits.

### Phase 2: WorkflowContextImpl
- [x] Implement via `WorkflowContextExt` blanket impl (automatically available)

**Note:** Since methods are default implementations in the extension trait with a blanket impl, `WorkflowContextImpl` automatically inherits them.

### Phase 3: MockWorkflowContext
- [x] Implement via `WorkflowContextExt` blanket impl (automatically available)

**Note:** Same as above - `MockWorkflowContext` automatically inherits the typed methods.

### Phase 4: Tests
- [x] Add test for typed task scheduling (`test_schedule_async_typed_serializes_input`)
- [x] Add test for typed task with options (`test_schedule_async_with_options_typed`)
- [x] Add test for typed child workflow scheduling (`test_schedule_workflow_async_typed`)
- [x] Add test for typed promise creation (`test_promise_async_typed`)
- [x] Add test for error handling (`test_schedule_async_without_mock_result_returns_error`)
- [x] Add async tests (`test_schedule_async_typed_returns_typed_future`, `test_promise_async_typed_returns_result`, etc.)

### Phase 5: Update Documentation
- [x] Update parallel-execution-implementation.md to mark Phase 2 typed methods as complete
- [x] Add examples to doc comments (in `context.rs`)

---

## Verification Checklist

- [x] `cargo build --workspace` succeeds
- [x] `cargo test --workspace` passes
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] Existing tests still pass (no regressions)

---

## Implementation Notes

### Design Decision: Extension Trait

The typed methods were implemented in `WorkflowContextExt` rather than directly in `WorkflowContext` because:

1. **Object Safety**: `WorkflowContext` is used as `&dyn WorkflowContext` throughout the codebase. Generic methods (like `schedule_async<T>`) make a trait non-object-safe.

2. **Blanket Implementation**: `WorkflowContextExt` has a blanket impl for all `WorkflowContext` implementors:
   ```rust
   impl<C: WorkflowContext + ?Sized> WorkflowContextExt for C {}
   ```
   This means the typed methods are automatically available on any `WorkflowContext` implementation.

3. **`Default` Bound**: Task and Workflow definitions require the `Default` trait to access `kind()` without an instance:
   ```rust
   fn schedule_async<T: TaskDefinition + Default>(&self, input: T::Input) -> TaskFuture<T::Output>
   ```
   This works well with unit struct tasks/workflows which derive `Default` trivially.
