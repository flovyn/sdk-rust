# Implementation Plan: Complete Error Encapsulation (Option B)

**Date**: 2025-12-31
**Related**: `.dev/docs/design/20251231_suspended-error-encapsulation.md`
**Prerequisite**: `.dev/docs/plans/20251231_suspended-error-encapsulation.md` (completed)
**Status**: Draft

## Summary

This plan describes how to achieve **complete encapsulation** of system errors (`Suspended`, `DeterminismViolation`) by removing them entirely from `FlovynError`. This is a more invasive change than the current `#[doc(hidden)]` approach but provides stronger guarantees.

**Goal**: Make it impossible for users to match on system errors at the type level.

## Current State (After Phase 1 Implementation)

The current implementation uses:
- `#[doc(hidden)]` to hide variants from documentation
- `#[non_exhaustive]` to require `_ =>` catch-all
- `WorkflowOutcome<T>` internal type (not fully utilized)

**Limitation**: Users can still explicitly match if they know the variant names:
```rust
match error {
    FlovynError::Suspended { .. } => { /* still compiles */ }
    _ => {}
}
```

## Proposed Solution: Thread-Local Suspension Signaling

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        User Workflow Code                        │
│                                                                  │
│  async fn workflow(ctx, input) -> Result<Output, FlovynError> {  │
│      let result = ctx.schedule("task", data).await?;             │
│      Ok(result)                                                  │
│  }                                                               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ Returns Result<T, FlovynError>
                              │ (NO Suspended/DeterminismViolation)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     WorkflowFuture<T>                            │
│                                                                  │
│  impl Future for TaskFuture<T> {                                │
│      fn poll() -> Poll<Result<T, FlovynError>> {                │
│          match self.poll_outcome() {                            │
│              Ready(result) => Poll::Ready(result),              │
│              Suspended(reason) => {                             │
│                  SUSPENSION.set(reason);  // Thread-local       │
│                  Poll::Pending                                  │
│              }                                                  │
│              DeterminismViolation(e) => panic!("{}", e),        │
│          }                                                      │
│      }                                                          │
│  }                                                               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ Poll::Pending + thread-local flag
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Workflow Executor                            │
│                                                                  │
│  fn execute() -> WorkflowTaskResult {                           │
│      match workflow_future.poll() {                             │
│          Poll::Ready(Ok(v)) => completed(v),                    │
│          Poll::Ready(Err(e)) => failed(e),                      │
│          Poll::Pending => {                                     │
│              if let Some(reason) = SUSPENSION.take() {          │
│                  suspended(reason)  // Generate command         │
│              } else {                                           │
│                  panic!("Unexpected Pending")                   │
│              }                                                  │
│          }                                                      │
│      }                                                          │
│  }                                                               │
└─────────────────────────────────────────────────────────────────┘
```

### Key Design Decisions

1. **Suspension via Thread-Local**: When a future needs to suspend, it stores the reason in a thread-local and returns `Poll::Pending`. The executor checks this after each poll.

2. **DeterminismViolation as Panic**: These indicate bugs and should never be caught. Converting to panic ensures the workflow fails immediately.

3. **Single-Threaded Execution**: Workflow code runs on a single thread (per workflow), making thread-local state safe.

4. **Clean Public API**: `FlovynError` only contains errors users should handle.

## Phase 1: Add Suspension Context

### Goal
Create thread-local infrastructure for suspension signaling.

### TODO

- [ ] **1.1** Create `sdk/src/workflow/suspension.rs`:
  ```rust
  use std::cell::RefCell;

  thread_local! {
      static SUSPENSION_CONTEXT: RefCell<SuspensionContext> =
          RefCell::new(SuspensionContext::new());
  }

  #[derive(Debug, Default)]
  pub(crate) struct SuspensionContext {
      /// Suspension reason if workflow is suspended
      reason: Option<String>,
      /// Whether we're currently in workflow execution context
      in_workflow: bool,
  }

  impl SuspensionContext {
      pub fn new() -> Self { Default::default() }

      /// Enter workflow execution context
      pub fn enter(&mut self) {
          self.in_workflow = true;
          self.reason = None;
      }

      /// Exit workflow execution context
      pub fn exit(&mut self) {
          self.in_workflow = false;
          self.reason = None;
      }

      /// Signal suspension
      pub fn suspend(&mut self, reason: String) {
          assert!(self.in_workflow, "suspend() called outside workflow context");
          self.reason = Some(reason);
      }

      /// Take suspension reason if set
      pub fn take_suspension(&mut self) -> Option<String> {
          self.reason.take()
      }

      /// Check if currently in workflow context
      pub fn is_in_workflow(&self) -> bool {
          self.in_workflow
      }
  }

  /// RAII guard for workflow execution context
  pub(crate) struct WorkflowExecutionGuard;

  impl WorkflowExecutionGuard {
      pub fn enter() -> Self {
          SUSPENSION_CONTEXT.with(|ctx| ctx.borrow_mut().enter());
          WorkflowExecutionGuard
      }
  }

  impl Drop for WorkflowExecutionGuard {
      fn drop(&mut self) {
          SUSPENSION_CONTEXT.with(|ctx| ctx.borrow_mut().exit());
      }
  }

  /// Signal workflow suspension (called by futures)
  pub(crate) fn signal_suspension(reason: String) {
      SUSPENSION_CONTEXT.with(|ctx| ctx.borrow_mut().suspend(reason));
  }

  /// Take suspension reason (called by executor)
  pub(crate) fn take_suspension() -> Option<String> {
      SUSPENSION_CONTEXT.with(|ctx| ctx.borrow_mut().take_suspension())
  }
  ```

- [ ] **1.2** Add module to `sdk/src/workflow/mod.rs`

- [ ] **1.3** Add unit tests for suspension context

## Phase 2: Update Future Implementations

### Goal
Change futures to use thread-local suspension instead of returning `Err(Suspended)`.

### TODO

- [ ] **2.1** Update `TaskFuture::poll()`:
  ```rust
  impl<T: DeserializeOwned> Future for TaskFuture<T> {
      type Output = Result<T, FlovynError>;

      fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
          match self.poll_outcome(cx) {
              Poll::Ready(WorkflowOutcome::Ready(result)) => {
                  Poll::Ready(result)
              }
              Poll::Ready(WorkflowOutcome::Suspended { reason }) => {
                  signal_suspension(reason);
                  Poll::Pending
              }
              Poll::Ready(WorkflowOutcome::DeterminismViolation(e)) => {
                  panic!("Determinism violation: {}", e)
              }
              Poll::Pending => Poll::Pending,
          }
      }
  }
  ```

- [ ] **2.2** Update `TimerFuture::poll()` with same pattern

- [ ] **2.3** Update `ChildWorkflowFuture::poll()` with same pattern

- [ ] **2.4** Update `PromiseFuture::poll()` with same pattern

- [ ] **2.5** Update `OperationFuture::poll()` with same pattern

- [ ] **2.6** Add tests verifying:
  - Suspension sets thread-local correctly
  - DeterminismViolation panics
  - Normal completion returns result

## Phase 3: Update Combinators

### Goal
Update combinators to work with new suspension model.

### Approach
Combinators poll child futures. When a child suspends:
1. Child sets thread-local and returns `Pending`
2. Combinator sees `Pending`, checks thread-local
3. If suspended, combinator also returns `Pending` (suspension propagates)

### TODO

- [ ] **3.1** Update `JoinAll`:
  ```rust
  impl<F, T> Future for JoinAll<F, T>
  where
      F: Future<Output = Result<T, FlovynError>> + Unpin,
  {
      type Output = Result<Vec<T>, FlovynError>;

      fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
          let this = self.get_mut();
          let mut all_done = true;

          for i in 0..this.futures.len() {
              if this.results[i].is_some() {
                  continue;
              }

              if let Some(ref mut future) = this.futures[i] {
                  match Pin::new(future).poll(cx) {
                      Poll::Ready(Ok(result)) => {
                          this.results[i] = Some(result);
                          this.futures[i] = None;
                      }
                      Poll::Ready(Err(e)) => {
                          return Poll::Ready(Err(e));
                      }
                      Poll::Pending => {
                          // Check if it's a suspension
                          if take_suspension().is_some() {
                              // Re-signal for executor
                              signal_suspension(format!("JoinAll waiting for future {}", i));
                          }
                          all_done = false;
                      }
                  }
              }
          }

          if all_done {
              let results = this.results.iter_mut()
                  .map(|r| r.take().unwrap())
                  .collect();
              Poll::Ready(Ok(results))
          } else {
              Poll::Pending
          }
      }
  }
  ```

- [ ] **3.2** Update `Select` with same pattern

- [ ] **3.3** Update `JoinN` with same pattern

- [ ] **3.4** Update `WithTimeout` with same pattern

- [ ] **3.5** Add tests for combinator suspension handling

## Phase 4: Update Executor

### Goal
Modify executor to use suspension context instead of matching on error variants.

### TODO

- [ ] **4.1** Update `WorkflowExecutor::execute()`:
  ```rust
  pub async fn execute<F, Fut>(&self, workflow_fn: F) -> WorkflowTaskResult
  where
      F: FnOnce(Arc<WorkflowContextImpl>, Value) -> Fut,
      Fut: Future<Output = Result<Value, FlovynError>>,
  {
      let ctx = Arc::new(WorkflowContextImpl::new(...));

      // Enter workflow execution context
      let _guard = WorkflowExecutionGuard::enter();

      // Run the workflow
      let future = workflow_fn(Arc::clone(&ctx), self.input.clone());
      let future = std::pin::pin!(future);

      // Poll until completion or suspension
      let waker = futures::task::noop_waker();
      let mut cx = Context::from_waker(&waker);

      match future.poll(&mut cx) {
          Poll::Ready(Ok(output)) => {
              self.handle_completion(ctx, output)
          }
          Poll::Ready(Err(e)) => {
              self.handle_failure(ctx, e)
          }
          Poll::Pending => {
              // Check for suspension
              if let Some(reason) = take_suspension() {
                  self.handle_suspension(ctx, reason)
              } else {
                  // Unexpected Pending without suspension
                  panic!("Workflow returned Pending without suspension signal")
              }
          }
      }
  }

  fn handle_completion(&self, ctx: Arc<WorkflowContextImpl>, output: Value) -> WorkflowTaskResult {
      let mut commands = ctx.get_commands();
      commands.push(WorkflowCommand::CompleteWorkflow { output: output.clone(), ... });
      WorkflowTaskResult::completed(commands, output)
  }

  fn handle_suspension(&self, ctx: Arc<WorkflowContextImpl>, reason: String) -> WorkflowTaskResult {
      let mut commands = ctx.get_commands();
      commands.push(WorkflowCommand::SuspendWorkflow { reason, ... });
      WorkflowTaskResult::suspended(commands)
  }

  fn handle_failure(&self, ctx: Arc<WorkflowContextImpl>, error: FlovynError) -> WorkflowTaskResult {
      let mut commands = ctx.get_commands();
      commands.push(WorkflowCommand::FailWorkflow { error: error.to_string(), ... });
      WorkflowTaskResult::failed(commands, error.to_string(), ...)
  }
  ```

- [ ] **4.2** Update `WorkflowWorker` with same pattern

- [ ] **4.3** Update executor tests

## Phase 5: Remove System Error Variants

### Goal
Remove `Suspended` and `DeterminismViolation` from `FlovynError`.

### TODO

- [ ] **5.1** Remove `FlovynError::Suspended` variant

- [ ] **5.2** Remove `FlovynError::DeterminismViolation` variant
  - Keep `DeterminismViolationError` type for panic messages

- [ ] **5.3** Remove `#[doc(hidden)]` attributes (no longer needed)

- [ ] **5.4** Update `into_result_legacy()` in `WorkflowOutcome`:
  ```rust
  impl<T> WorkflowOutcome<T> {
      /// Convert to Result. Panics on system errors.
      ///
      /// This should only be called by internal code that has already
      /// handled suspension via thread-local signaling.
      pub fn into_result(self) -> Result<T, FlovynError> {
          match self {
              WorkflowOutcome::Ready(result) => result,
              WorkflowOutcome::Suspended { reason } => {
                  panic!("into_result() called on Suspended outcome: {}", reason)
              }
              WorkflowOutcome::DeterminismViolation(e) => {
                  panic!("Determinism violation: {}", e)
              }
          }
      }
  }
  ```

- [ ] **5.5** Search and fix all remaining usages:
  ```bash
  grep -r "FlovynError::Suspended" sdk/
  grep -r "FlovynError::DeterminismViolation" sdk/
  ```

- [ ] **5.6** Update tests that explicitly use removed variants

## Phase 6: Update Tests

### TODO

- [ ] **6.1** Update unit tests in `sdk/src/error.rs`

- [ ] **6.2** Update executor tests

- [ ] **6.3** Update combinator tests

- [ ] **6.4** Update E2E tests if needed

- [ ] **6.5** Add negative test: verify users CAN'T match on removed variants
  ```rust
  #[test]
  fn test_cannot_match_system_errors() {
      // This test documents that the following code would NOT compile:
      //
      // match error {
      //     FlovynError::Suspended { .. } => {} // ERROR: no variant named `Suspended`
      //     _ => {}
      // }
      //
      // Uncomment to verify it doesn't compile:
      // let _ = FlovynError::Suspended { reason: "test".into() };
  }
  ```

## Phase 7: Documentation

### TODO

- [ ] **7.1** Update `FlovynError` documentation (remove system error mentions)

- [ ] **7.2** Add internal documentation for suspension mechanism

- [ ] **7.3** Update CHANGELOG with breaking changes

- [ ] **7.4** Create migration guide for users (if any were matching on these)

## Risk Assessment

### Thread-Local Safety
**Risk**: Thread-local state could leak between workflow executions.
**Mitigation**: RAII guard (`WorkflowExecutionGuard`) ensures cleanup on exit.

### Panic on DeterminismViolation
**Risk**: Panic could crash the worker.
**Mitigation**:
- Executor wraps workflow execution in `catch_unwind`
- Convert panic to `FailWorkflow` command
- This is actually desired behavior - determinism violations are fatal

### Combinator Complexity
**Risk**: Combinators need careful handling of suspension propagation.
**Mitigation**: Extensive testing, clear patterns.

### Async Runtime Compatibility
**Risk**: Thread-local might not work with work-stealing runtimes.
**Mitigation**: Workflow execution is already single-threaded per workflow. Use `tokio::task::LocalSet` if needed.

## Testing Strategy

1. **Unit tests**: Each phase includes tests for modified components
2. **Integration tests**: Run E2E tests after each phase
3. **Panic tests**: Verify DeterminismViolation panics correctly
4. **Compilation tests**: Verify removed variants cause compile errors
5. **Thread-local tests**: Verify context cleanup and isolation

## Rollback Plan

- Phases 1-4: Can be rolled back by reverting to `into_result_legacy()`
- Phase 5: Breaking change - requires version bump if rolled back

## Verification Checklist

After implementation:
- [ ] `cargo build --workspace` succeeds
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `FlovynError::Suspended` does not exist (compile error if used)
- [ ] `FlovynError::DeterminismViolation` does not exist (compile error if used)
- [ ] Workflows suspend correctly via thread-local mechanism
- [ ] DeterminismViolation causes panic → workflow failure
- [ ] Combinators propagate suspension correctly
- [ ] E2E tests pass

## Estimated Effort

| Phase | Effort | Risk |
|-------|--------|------|
| Phase 1: Suspension Context | 2-3 hours | Low |
| Phase 2: Update Futures | 2-3 hours | Medium |
| Phase 3: Update Combinators | 3-4 hours | Medium |
| Phase 4: Update Executor | 2-3 hours | Medium |
| Phase 5: Remove Variants | 1-2 hours | Low |
| Phase 6: Update Tests | 2-3 hours | Low |
| Phase 7: Documentation | 1-2 hours | Low |
| **Total** | **~2 days** | Medium |

## Decision

This plan is for a **future major version** (e.g., v0.2.0 or v1.0.0) because:
1. It's a breaking change (removes error variants)
2. Current `#[doc(hidden)]` approach is sufficient for most cases
3. Requires careful testing of thread-local behavior

Proceed when:
- Preparing for stable release
- Users report issues with accidentally catching system errors
- Major refactoring is already planned
