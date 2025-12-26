# Phase 1: Extract Core - Implementation Plan

## Overview

This plan implements Phase 1 from the [Core SDK in Rust for Multiple Languages](../design/core-sdk-in-rust-for-multiple-language.md) design document.

**Goal**: Split the current `sdk/` crate into `core/` (language-agnostic) and `sdk/` (Rust-specific) while maintaining full backward compatibility.

**Constraint**: After each step, all tests must pass and examples must compile.

## Prerequisites

- Familiarity with the current SDK structure (see design doc for module breakdown)
- Understanding of the replay/determinism model

## Architecture Summary

```
Before:                          After:
sdk/                             core/
├── workflow/                    ├── workflow/
├── task/                        │   ├── command.rs
├── worker/                      │   ├── event.rs
├── client/                      │   ├── execution.rs (was context_impl)
├── testing/                     │   └── recorder.rs
└── generated/                   ├── task/
                                 │   ├── execution.rs (was executor)
                                 │   └── registry.rs
                                 ├── worker/
                                 │   ├── workflow_worker.rs
                                 │   ├── task_worker.rs
                                 │   ├── determinism.rs
                                 │   └── lifecycle/
                                 ├── client/
                                 │   ├── grpc.rs
                                 │   ├── workflow_dispatch.rs
                                 │   ├── task_execution.rs
                                 │   └── worker_lifecycle.rs
                                 ├── generated/
                                 └── error.rs

                                 sdk/
                                 ├── workflow/
                                 │   ├── definition.rs
                                 │   ├── context.rs (trait)
                                 │   ├── future.rs
                                 │   └── combinators.rs
                                 ├── task/
                                 │   ├── definition.rs
                                 │   └── context.rs (trait)
                                 ├── client/
                                 │   ├── flovyn_client.rs
                                 │   └── builder.rs
                                 ├── worker/
                                 │   ├── builder.rs
                                 │   └── executor.rs
                                 ├── testing/
                                 └── config/
```

## Implementation Strategy

The extraction follows a **bottom-up, one-module-at-a-time** approach:

1. Create `core/` crate with minimal structure
2. Move lowest-level modules first (no internal dependencies)
3. Update imports in `sdk/` to use `core::`
4. Run tests after each move
5. Continue until all core logic is extracted

## TODO List

### Step 1: Create Core Crate Skeleton

- [x] Create `core/` directory with `Cargo.toml`
- [x] Add `core` to workspace in root `Cargo.toml`
- [x] Create minimal `core/src/lib.rs` with module declarations
- [x] Add `flovyn-core` as dependency in `sdk/Cargo.toml`
- [x] Verify: `cargo build --workspace` passes

### Step 2: Move Generated Protobuf Code

The generated gRPC code has no internal dependencies - move it first.

- [x] Move `sdk/src/generated/` to `core/src/generated/`
- [x] Update `core/src/lib.rs` to expose `generated` module
- [x] Update `sdk/src/lib.rs` to re-export `flovyn_core::generated`
- [x] Update all imports in `sdk/` from `crate::generated` to `flovyn_core::generated`
- [x] Verify: `cargo test --workspace` passes

### Step 3: Move Error Types

Error types are used throughout - move early to avoid circular deps.

- [x] Create `core/src/error.rs` with core error types
- [x] Move `DeterminismViolationError` and related errors to core
- [x] Keep `FlovynError` in sdk (it includes Rust-specific variants)
- [x] Move `EventType` to core (needed by `DeterminismViolationError`)
- [x] Update sdk to re-export `EventType` and `DeterminismViolationError` from core
- [x] Verify: `cargo test --workspace` passes

### Step 4: Move Workflow Command and Event Types

These are pure data types with no behavior dependencies.

- [x] Create `core/src/workflow/mod.rs`
- [x] Move `sdk/src/workflow/command.rs` to `core/src/workflow/command.rs`
- [x] Move `sdk/src/workflow/event.rs` (ReplayEvent) to `core/src/workflow/event.rs`
- [x] Update sdk to re-export these types
- [x] Update all internal imports
- [x] Verify: `cargo test --workspace` passes

### Step 5: Move Command Recorder

The recorder trait and implementations.

- [x] Move `sdk/src/workflow/recorder.rs` to `core/src/workflow/recorder.rs`
- [x] Update imports and re-exports
- [x] Verify: `cargo test --workspace` passes

### Step 6: Move Determinism Validator

Note: Moved before Step 5 since recorder depends on it.

- [x] Create `core/src/worker/` directory
- [x] Move `sdk/src/worker/determinism.rs` to `core/src/worker/determinism.rs`
- [x] Create `core/src/worker/mod.rs`
- [x] Update imports and re-exports
- [x] Verify: `cargo test --workspace` passes

### Step 7: Move Workflow Execution Engine

This is the core replay logic (currently `context_impl.rs`).

Note: After analysis, `context_impl.rs` is heavily Rust-specific (async traits, Tokio primitives,
Rust futures). Instead of extracting `CoreWorkflowExecution`, we extract language-agnostic
utilities that can be shared across SDKs.

- [x] Create `core/src/workflow/execution.rs` with language-agnostic utilities
- [x] Extract `DeterministicRandom` trait and `SeededRandom` implementation (xorshift64)
- [x] Extract `EventLookup` utilities for finding terminal events during replay
- [x] Extract `build_operation_cache` and `build_initial_state` helper functions
- [x] Keep `WorkflowContext` trait and `WorkflowContextImpl` in sdk (Rust-specific async traits)
- [x] Update `core/src/workflow/mod.rs` to export execution utilities
- [x] Verify: `cargo test --workspace` passes

### Step 8: Move Task Registry and Executor

Note: After analysis, the registry and executor are heavily Rust-specific (async traits, tokio primitives,
Arc<dyn Trait> patterns). Instead of moving them, we extract language-agnostic types that represent
task metadata and execution results.

- [x] Create `core/src/task/mod.rs`
- [x] Create `core/src/task/execution.rs` with language-agnostic types:
  - `TaskMetadata` - task metadata struct with builder pattern
  - `TaskExecutionResult` - enum for execution outcomes (Completed, Failed, Cancelled, TimedOut)
  - `TaskExecutorConfig` - configuration for task execution defaults
  - `BackoffConfig` - configuration for retry backoff
  - `calculate_backoff()` - backoff calculation function
  - `should_retry()` - retry decision function
- [x] Keep `TaskRegistry`, `TaskExecutor`, `TaskDefinition`, `TaskContext` traits in sdk
- [x] Update `core/src/lib.rs` to export task types
- [x] Verify: `cargo test --workspace` passes

### Step 9: Move Worker Lifecycle Module

Note: The lifecycle module contains both language-agnostic types (status, events, metrics)
and Rust-specific async traits (hooks, policies). We extract the types to core and keep
the async traits in SDK.

- [x] Create `core/src/worker/lifecycle/` directory
- [x] Create `core/src/worker/lifecycle/types.rs` with:
  - `WorkerStatus` - worker operational status enum
  - `StopReason` - reason for stopping enum
  - `RegistrationInfo`, `WorkflowConflict`, `TaskConflict` - registration types
  - `ConnectionInfo` - connection state struct
  - `WorkerControlError` - control operation errors
- [x] Create `core/src/worker/lifecycle/events.rs` with:
  - `WorkType` - workflow/task enum
  - `WorkerLifecycleEvent` - lifecycle event enum
- [x] Create `core/src/worker/lifecycle/metrics.rs` with:
  - `WorkerMetrics` - runtime metrics struct with recording methods
- [x] Create `core/src/worker/lifecycle/reconnection.rs` with:
  - `ReconnectionStrategy` - reconnection strategy enum (without async trait)
- [x] Update `core/src/worker/mod.rs` to export lifecycle types
- [x] Keep `WorkerLifecycleHook`, `ReconnectionPolicy`, `HookChain`, `WorkerInternals` in sdk
- [x] Verify: `cargo test --workspace` passes (116 core tests, 508 total)

### Step 10: Move gRPC Client Wrappers

**Decision: Moved to Core** - Since core already depends on tonic/tokio, the gRPC client
wrappers can be shared. The core provides authenticated clients, SDK re-exports them.

- [x] Create `core/src/client/` directory
- [x] Create `core/src/client/auth.rs` with `WorkerTokenInterceptor`
- [x] Create `core/src/client/task_execution.rs` with `TaskExecutionClient`
- [x] Create `core/src/client/workflow_dispatch.rs` with `WorkflowDispatch`
- [x] Create `core/src/client/workflow_query.rs` with `WorkflowQueryClient`
- [x] Create `core/src/client/worker_lifecycle.rs` with `WorkerLifecycleClient`
- [x] Create `core/src/error.rs` with `CoreError` and `CoreResult`
- [x] Move task streaming types to `core/src/task/streaming/`
- [x] Add `WorkflowMetadata` to `core/src/workflow/execution.rs`
- [x] Update SDK to re-export client types from core
- [x] Add `From<CoreError> for FlovynError` conversion
- [x] Add `From<WorkflowMetadata/TaskMetadata>` conversions (SDK to core types)
- [x] Delete duplicate SDK client files
- [x] Verify: `cargo test --workspace` passes (432 tests)

### Step 11: Move Worker Polling Logic

**Decision: Skip** - The worker polling logic is language-specific:
- `workflow_worker.rs` and `task_worker.rs` coordinate with SDK-specific registries
- Uses `Arc<dyn Trait>` patterns for callbacks
- Depends on SDK-specific executor implementations

Each language SDK will implement its own polling coordination.

- [x] Decision documented: Worker polling stays in SDK

### Step 12: Update SDK Re-exports

- [x] Update SDK's `client/mod.rs` to re-export from `flovyn_core::client`
- [x] Update SDK's `task/streaming/mod.rs` to re-export from `flovyn_core::task::streaming`
- [x] Keep `hook.rs` in SDK (user-facing observability API)
- [x] Update lib.rs documentation in both core and SDK
- [x] Verify: `cargo clippy --workspace --all-targets -- -D warnings` passes

### Step 13: Final Verification

- [x] `cargo build --workspace` passes
- [x] `cargo test --workspace` passes (432 tests)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] `cargo fmt --all -- --check` passes
- [x] All examples in `examples/` compile without changes
- [x] No public API changes to `flovyn-sdk` (existing user code compiles)

## Phase 1 Complete ✓

Successfully extracted language-agnostic core functionality from `flovyn-sdk` into `flovyn-core`:

**Core Crate Contents:**
- `generated/` - gRPC/protobuf generated code
- `workflow/` - WorkflowCommand, ReplayEvent, EventType, CommandRecorder, execution utilities, WorkflowMetadata
- `task/` - TaskMetadata, TaskExecutionResult, BackoffConfig, streaming types (StreamEvent, etc.)
- `worker/` - DeterminismValidator, lifecycle types (WorkerStatus, events, metrics, reconnection)
- `client/` - gRPC client wrappers (TaskExecutionClient, WorkflowDispatch, WorkerLifecycleClient, etc.)
- `error/` - CoreError, DeterminismViolationError

**SDK Crate Retains:**
- Async traits (WorkflowContext, TaskContext, WorkerLifecycleHook)
- Worker polling logic and executors (tokio-based coordination)
- Registry implementations with Rust-specific types (SemanticVersion)
- Workflow hooks for observability
- Testing utilities
- Client builders and high-level FlovynClient API

## Module Mapping Reference

| Original Location | New Location | Notes |
|-------------------|--------------|-------|
| `sdk/src/generated/` | `core/src/generated/` | Re-exported from sdk |
| `sdk/src/workflow/command.rs` | `core/src/workflow/command.rs` | |
| `sdk/src/workflow/event.rs` | `core/src/workflow/event.rs` | |
| `sdk/src/workflow/recorder.rs` | `core/src/workflow/recorder.rs` | |
| `sdk/src/workflow/context_impl.rs` | `core/src/workflow/execution.rs` | Refactored |
| `sdk/src/workflow/definition.rs` | `sdk/src/workflow/definition.rs` | Stays in sdk |
| `sdk/src/workflow/context.rs` | `sdk/src/workflow/context.rs` | Stays in sdk (trait) |
| `sdk/src/workflow/future.rs` | `sdk/src/workflow/future.rs` | Stays in sdk |
| `sdk/src/workflow/combinators.rs` | `sdk/src/workflow/combinators.rs` | Stays in sdk |
| `sdk/src/task/executor.rs` | `core/src/task/execution.rs` | Core logic extracted |
| `sdk/src/task/registry.rs` | `core/src/task/registry.rs` | |
| `sdk/src/task/definition.rs` | `sdk/src/task/definition.rs` | Stays in sdk |
| `sdk/src/task/context.rs` | `sdk/src/task/context.rs` | Stays in sdk (trait) |
| `sdk/src/worker/determinism.rs` | `core/src/worker/determinism.rs` | |
| `sdk/src/worker/workflow_worker.rs` | `core/src/worker/workflow_worker.rs` | |
| `sdk/src/worker/task_worker.rs` | `core/src/worker/task_worker.rs` | |
| `sdk/src/worker/lifecycle/` | `core/src/worker/lifecycle/` | Entire directory |
| `sdk/src/worker/executor.rs` | `sdk/src/worker/executor.rs` | Stays (uses traits) |
| `sdk/src/worker/registry.rs` | `core/src/worker/registry.rs` | |
| `sdk/src/client/workflow_dispatch.rs` | `core/src/client/workflow_dispatch.rs` | |
| `sdk/src/client/task_execution.rs` | `core/src/client/task_execution.rs` | |
| `sdk/src/client/worker_lifecycle.rs` | `core/src/client/worker_lifecycle.rs` | |
| `sdk/src/client/auth.rs` | `core/src/client/auth.rs` | |
| `sdk/src/client/flovyn_client.rs` | `sdk/src/client/flovyn_client.rs` | Stays in sdk |
| `sdk/src/client/builder.rs` | `sdk/src/client/builder.rs` | Stays in sdk |
| `sdk/src/testing/` | `sdk/src/testing/` | Stays in sdk |
| `sdk/src/config/` | `sdk/src/config/` | Stays in sdk |

## Dependencies

### core/Cargo.toml

```toml
[package]
name = "flovyn-core"
version = "0.1.0"
edition = "2021"

[dependencies]
# gRPC
tonic = { version = "0.12", features = ["tls"] }
prost = "0.13"
prost-types = "0.13"

# Async
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time", "macros"] }
tokio-stream = "0.1"
async-trait = "0.1"
futures = "0.3"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Utilities
tracing = "0.1"
thiserror = "1"
uuid = { version = "1", features = ["v4"] }

[build-dependencies]
tonic-build = "0.12"
```

### sdk/Cargo.toml Changes

```toml
[dependencies]
flovyn-core = { path = "../core" }
# ... existing dependencies
```

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Circular dependencies between core and sdk | Bottom-up extraction; core has no sdk dependency |
| Breaking public API | Re-export all moved types from sdk; run examples after each step |
| Test failures during refactoring | Run tests after each TODO item; fix immediately |
| Performance regression | Benchmark critical paths before/after |

## Success Criteria

1. `cargo test --workspace` passes
2. `cargo clippy --workspace --all-targets -- -D warnings` passes
3. All examples compile and run without modification
4. No changes to `flovyn-sdk` public API
5. `flovyn-core` exposes clean activation-based API for future language bindings
