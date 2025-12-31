# Implementation Plan: Encapsulating System Errors

**Date**: 2025-12-31
**Design Document**: `.dev/docs/design/20251231_suspended-error-encapsulation.md`
**Status**: Complete

## Summary

This plan implements Option 1 (Separate Error Hierarchies) from the design document to prevent user code from catching system errors that should not be handled by application code. The implementation separates internal control flow/system errors from public application errors.

**System errors to encapsulate:**
1. `Suspended` - Control flow mechanism for workflow suspension
2. `DeterminismViolation` - Replay corruption indicator

## Progress

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1 | ✅ Complete | Added `WorkflowOutcome<T>` type in `sdk/src/workflow/outcome.rs` |
| Phase 2 | ✅ Complete | All futures now use `poll_outcome()` internally |
| Phase 3 | ✅ Complete (No changes) | Combinators work with generic futures, can't use `poll_outcome()` |
| Phase 4 | ✅ Complete (No changes) | Executor/worker correctly handle errors internally |
| Phase 5 | ✅ Complete | Added `#[doc(hidden)]` and `#[non_exhaustive]` to hide system errors |
| Phase 6 | ⏸️ Deferred | Optional diagnostic hooks - not needed for initial implementation |
| Phase 7 | ✅ Complete | Documentation updated with error categories |

## Implementation Summary

### Phase 1: Internal Types ✅

Created `sdk/src/workflow/outcome.rs` with:
- `WorkflowOutcome<T>` enum with `Ready`, `Suspended`, `DeterminismViolation` variants
- Helper methods: `ok()`, `err()`, `suspended()`, `determinism_violation()`
- `into_result_legacy()` for backward-compatible conversion
- Comprehensive unit tests

### Phase 2: Future Migration ✅

Updated all workflow futures to use `WorkflowOutcome` internally:
- Added `WorkflowFuturePoll` trait with `poll_outcome()` method
- Updated `TaskFuture`, `TimerFuture`, `ChildWorkflowFuture`, `PromiseFuture`, `OperationFuture`
- Each future now has:
  - `poll_outcome()` returning `Poll<WorkflowOutcome<T>>`
  - `Future::poll()` delegating to `poll_outcome()` and converting via `into_result_legacy()`

### Phase 3: Combinators ✅ (No Changes Needed)

**Key Insight**: Combinators (`join_all`, `select`, `join_n`, `with_timeout`) work with generic `F: Future<Output = Result<T>>`. They can't use the `WorkflowFuturePoll` trait because:
1. They accept any future implementing `Future`, not just our workflow futures
2. The trait is internal (`pub(crate)`)

The combinators correctly match on `FlovynError::Suspended` to treat it like `Pending`. This will need revisiting when we remove the variant in Phase 5.

### Phase 4: Executor/Worker ✅ (No Changes Needed)

**Key Insight**: The executor's `handle_result()` method correctly:
1. Matches on `FlovynError::Suspended` to generate `SuspendWorkflow` command
2. Matches on `FlovynError::DeterminismViolation` to generate `FailWorkflow` command
3. Handles other errors appropriately

No changes needed here because the executor SHOULD see these errors - it's the boundary where they're handled. The goal is to prevent USER code from matching on them, not internal SDK code.

## Completed Work

### Phase 5: Hide System Error Variants ✅

Implemented **Option C**: Using `#[non_exhaustive]` + `#[doc(hidden)]` + documentation.

Changes to `sdk/src/error.rs`:
- Added `#[non_exhaustive]` to `FlovynError` enum
- Added `#[doc(hidden)]` to `Suspended` and `DeterminismViolation` variants
- Added comprehensive documentation explaining error categories
- Added strong warnings in variant doc comments
- Organized variants into "Internal Errors" and "Application Errors" sections

**Result**:
- Variants hidden from generated documentation
- Users must use `_ =>` catch-all when matching (due to `#[non_exhaustive]`)
- Strong warnings discourage matching on system errors
- Internal SDK code continues to work unchanged

### Phase 7: Documentation ✅

- [x] Added error category documentation to `FlovynError`
- [x] Clear separation of application vs. internal errors
- [x] Example code showing proper error handling with catch-all

### Phase 6: Diagnostic Hooks (Deferred)

Not implemented - can be added later if users need to log/monitor system errors without catching them.

## Files Modified

| File | Changes |
|------|---------|
| `sdk/src/workflow/outcome.rs` | NEW - `WorkflowOutcome<T>` type with Ready/Suspended/DeterminismViolation |
| `sdk/src/workflow/mod.rs` | Added `pub(crate) mod outcome` |
| `sdk/src/workflow/future.rs` | Added `WorkflowFuturePoll` trait, `poll_outcome()` to all futures |
| `sdk/src/error.rs` | Added `#[non_exhaustive]`, `#[doc(hidden)]`, error category docs |

## Verification

All 344 SDK lib tests pass:
```
cargo test -p flovyn-sdk --lib
test result: ok. 344 passed; 0 failed; 0 ignored
```

Clippy passes with no warnings:
```
cargo clippy -p flovyn-sdk --lib -- -D warnings
Finished `dev` profile [unoptimized + debuginfo]
```

## Conclusion

The implementation successfully:
1. **Created internal `WorkflowOutcome<T>` type** - separates control flow from application errors
2. **Migrated all futures** - `TaskFuture`, `TimerFuture`, `ChildWorkflowFuture`, `PromiseFuture`, `OperationFuture` now use `poll_outcome()` internally
3. **Hid system error variants** - `Suspended` and `DeterminismViolation` are now `#[doc(hidden)]`
4. **Required catch-all matching** - `#[non_exhaustive]` forces `_ =>` arm
5. **Added clear documentation** - Error categories explained, warnings on system errors

**Limitations** (acceptable for now):
- Users can still explicitly match on hidden variants if they know the names
- Combinators still match on `FlovynError::Suspended` internally (correct behavior)

**Future consideration** (Option B from plan):
- For a major version, could split into separate `FlovynError` (public) and `InternalError` (private) types
- Would require significant refactoring but provide complete encapsulation
