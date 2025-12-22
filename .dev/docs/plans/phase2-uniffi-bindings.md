# Phase 2: Add uniffi Bindings - Implementation Plan

## Overview

This plan implements Phase 2 from the [Core SDK in Rust for Multiple Languages](../design/core-sdk-in-rust-for-multiple-language.md) design document.

**Goal**: Create a separate `ffi` crate that wraps `flovyn-core` with uniffi bindings for consumption by other languages (Kotlin, Swift, Python). The `core` crate remains unchanged.

**Constraint**: After each step, `cargo test --workspace` must pass.

## Prerequisites

- Phase 1 complete (core crate extracted)
- Familiarity with [uniffi documentation](https://mozilla.github.io/uniffi-rs/)

## Key Design Decisions

### 1. Separate FFI Crate

Instead of adding uniffi to `core`, we create a **separate `ffi` crate** that:
- Depends on `flovyn-core`
- Contains all uniffi annotations and wrapper types
- Keeps `core` clean and focused on business logic
- Users who don't need FFI don't see any uniffi-related code

```
sdk-rust/
├── core/     # Language-agnostic core (unchanged)
├── ffi/      # NEW: uniffi bindings layer
└── sdk/      # Rust SDK
```

### 2. Activation-Based API

Following Temporal's model, we expose an **activation-based protocol** where:
- Core polls for work and returns `WorkflowActivation` / `TaskActivation`
- Language SDK processes activations and returns `WorkflowActivationCompletion` / `TaskCompletion`
- Core handles all gRPC communication, replay, and determinism validation

This keeps the FFI boundary simple with data-only types crossing the boundary.

### 3. What to Export

**Export** (data types that cross FFI boundary):
- `WorkflowActivation` - workflow work to process
- `WorkflowActivationCompletion` - result from language SDK
- `TaskActivation` - task work to process
- `TaskCompletion` - task result
- `WorkflowCommand` variants
- `EventType`, `ReplayEvent`
- `TaskMetadata`, `TaskExecutionResult`
- `WorkerStatus`, `WorkerLifecycleEvent`
- Error types (simplified for FFI)

**Export** (objects with methods):
- `CoreWorker` - main worker object with poll/complete methods
- `CoreClient` - client for starting workflows, queries

**Don't Export** (stay internal):
- Raw gRPC clients (wrapped by CoreWorker)
- Protobuf types (converted to/from activation types)
- Rust-specific traits

### 4. Async Handling

uniffi supports async functions natively. Async methods become:
- Kotlin: `suspend fun`
- Swift: `async func`
- Python: `async def`

The foreign language's event loop drives completion.

### 5. Error Handling

Create uniffi-compatible error types that wrap internal errors:

```rust
#[derive(Debug, uniffi::Error)]
pub enum FfiError {
    Grpc { message: String, code: i32 },
    Timeout { message: String },
    Serialization { message: String },
    DeterminismViolation { message: String },
    Other { message: String },
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Kotlin/Swift/Python SDK                       │
│  • Workflow/Task definitions                                     │
│  • Idiomatic async (coroutines/async-await)                     │
│  • Language-native types                                         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ uniffi FFI
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      flovyn-ffi (new crate)                      │
│                                                                  │
│  • CoreWorker object                                            │
│  • CoreClient object                                            │
│  • Activation/Completion types                                  │
│  • FfiError                                                     │
│  • uniffi scaffolding                                           │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ Rust dependency
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      flovyn-core (unchanged)                     │
│                                                                  │
│  • client/ (gRPC)                                               │
│  • workflow/ (commands, events)                                 │
│  • worker/ (determinism, lifecycle)                             │
│  • task/ (execution, streaming)                                 │
└─────────────────────────────────────────────────────────────────┘
```

## TODO List

### Step 1: Create FFI Crate Skeleton

- [ ] Create `ffi/` directory
- [ ] Create `ffi/Cargo.toml` with uniffi dependency
- [ ] Add `ffi` to workspace in root `Cargo.toml`
- [ ] Create `ffi/src/lib.rs` with `uniffi::setup_scaffolding!()`
- [ ] Verify: `cargo build --workspace` passes

```toml
# ffi/Cargo.toml
[package]
name = "flovyn-ffi"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "staticlib", "lib"]

[dependencies]
flovyn-core = { path = "../core" }
uniffi = "0.28"
tokio = { version = "1", features = ["rt-multi-thread"] }
async-trait = "0.1"

[build-dependencies]
uniffi = { version = "0.28", features = ["build"] }
```

### Step 2: Create FFI Error Type

- [ ] Create `ffi/src/error.rs` with `FfiError` enum
- [ ] Add `#[derive(Debug, thiserror::Error, uniffi::Error)]` to `FfiError`
- [ ] Implement `From<flovyn_core::CoreError>` for `FfiError`
- [ ] Implement `From<flovyn_core::DeterminismViolationError>` for `FfiError`
- [ ] Export from `ffi/src/lib.rs`
- [ ] Verify: `cargo build -p flovyn-ffi` passes

```rust
// ffi/src/error.rs
use flovyn_core::{CoreError, DeterminismViolationError};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("gRPC error: {message} (code: {code})")]
    Grpc { message: String, code: i32 },
    #[error("Timeout: {message}")]
    Timeout { message: String },
    #[error("Serialization error: {message}")]
    Serialization { message: String },
    #[error("Determinism violation: {message}")]
    DeterminismViolation { message: String },
    #[error("Cancelled")]
    Cancelled,
    #[error("{message}")]
    Other { message: String },
}

impl From<CoreError> for FfiError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Grpc(status) => FfiError::Grpc {
                message: status.message().to_string(),
                code: status.code() as i32,
            },
            CoreError::Timeout(msg) => FfiError::Timeout { message: msg },
            CoreError::Serialization(err) => FfiError::Serialization {
                message: err.to_string(),
            },
            CoreError::Other(msg) => FfiError::Other { message: msg },
            _ => FfiError::Other { message: err.to_string() },
        }
    }
}
```

### Step 3: Create FFI Activation Types

Define the activation protocol types that cross the FFI boundary.

- [ ] Create `ffi/src/activation.rs`
- [ ] Define `WorkflowActivation` record with `#[derive(uniffi::Record)]`
- [ ] Define `WorkflowActivationJob` enum with `#[derive(uniffi::Enum)]`
- [ ] Define `WorkflowActivationCompletion` record
- [ ] Define `TaskActivation` record
- [ ] Define `TaskCompletion` record
- [ ] Export from `ffi/src/lib.rs`
- [ ] Verify: `cargo build -p flovyn-ffi` passes

```rust
// ffi/src/activation.rs

#[derive(uniffi::Record)]
pub struct WorkflowActivation {
    pub run_id: String,
    pub workflow_execution_id: String,
    pub workflow_kind: String,
    pub timestamp_ms: i64,
    pub is_replaying: bool,
    pub random_seed: Vec<u8>,
    pub jobs: Vec<WorkflowActivationJob>,
}

#[derive(uniffi::Enum)]
pub enum WorkflowActivationJob {
    Initialize { input: Vec<u8> },
    FireTimer { timer_id: String },
    ResolveTask { task_id: String, result: Vec<u8> },
    ResolvePromise { promise_name: String, value: Vec<u8> },
    CancelWorkflow,
    // ... other variants
}

#[derive(uniffi::Record)]
pub struct WorkflowActivationCompletion {
    pub run_id: String,
    pub commands: Vec<FfiWorkflowCommand>,
}
```

### Step 4: Create FFI Command Types

- [ ] Create `ffi/src/command.rs`
- [ ] Define `FfiWorkflowCommand` enum with variants matching `WorkflowCommand`
- [ ] Implement conversion from `FfiWorkflowCommand` to `flovyn_core::WorkflowCommand`
- [ ] Export from `ffi/src/lib.rs`
- [ ] Verify: `cargo build -p flovyn-ffi` passes

```rust
// ffi/src/command.rs

#[derive(uniffi::Enum)]
pub enum FfiWorkflowCommand {
    ScheduleTask {
        task_id: String,
        task_kind: String,
        input: Vec<u8>,
        timeout_seconds: Option<u32>,
    },
    StartTimer {
        timer_id: String,
        duration_ms: u64,
    },
    CompleteWorkflow {
        result: Vec<u8>,
    },
    FailWorkflow {
        error: String,
    },
    CreatePromise {
        promise_name: String,
    },
    SetState {
        key: String,
        value: Vec<u8>,
    },
    // ... other variants
}

impl From<FfiWorkflowCommand> for flovyn_core::WorkflowCommand {
    fn from(cmd: FfiWorkflowCommand) -> Self {
        // Convert FFI command to core command
    }
}
```

### Step 5: Create CoreWorker Object

The main object exposed to foreign languages.

- [ ] Create `ffi/src/worker.rs`
- [ ] Define `CoreWorker` struct with `#[derive(uniffi::Object)]`
- [ ] Implement constructor `new(config: WorkerConfig) -> Arc<Self>`
- [ ] Implement `async fn poll_workflow_activation(&self) -> Result<WorkflowActivation, FfiError>`
- [ ] Implement `async fn complete_workflow_activation(&self, completion: WorkflowActivationCompletion) -> Result<(), FfiError>`
- [ ] Implement `async fn poll_task_activation(&self) -> Result<TaskActivation, FfiError>`
- [ ] Implement `async fn complete_task(&self, completion: TaskCompletion) -> Result<(), FfiError>`
- [ ] Implement `fn initiate_shutdown(&self)`
- [ ] Export from `ffi/src/lib.rs`
- [ ] Verify: `cargo build -p flovyn-ffi` passes

```rust
// ffi/src/worker.rs
use std::sync::Arc;
use crate::{FfiError, WorkflowActivation, WorkflowActivationCompletion, TaskActivation, TaskCompletion, WorkerConfig};

#[derive(uniffi::Object)]
pub struct CoreWorker {
    // Internal state: gRPC clients, runtime, etc.
}

#[uniffi::export]
impl CoreWorker {
    #[uniffi::constructor]
    pub async fn new(config: WorkerConfig) -> Result<Arc<Self>, FfiError> {
        // Initialize gRPC clients, connect to server
    }

    pub async fn poll_workflow_activation(&self) -> Result<WorkflowActivation, FfiError> {
        // Poll for workflow work, convert to activation
    }

    pub async fn complete_workflow_activation(
        &self,
        completion: WorkflowActivationCompletion
    ) -> Result<(), FfiError> {
        // Convert commands, submit to server
    }

    pub async fn poll_task_activation(&self) -> Result<TaskActivation, FfiError> {
        // Poll for task work
    }

    pub async fn complete_task(&self, completion: TaskCompletion) -> Result<(), FfiError> {
        // Submit task result
    }

    pub fn initiate_shutdown(&self) {
        // Signal shutdown
    }
}
```

### Step 6: Create CoreClient Object

Client for workflow operations (start, query, signal).

- [ ] Create `ffi/src/client.rs`
- [ ] Define `CoreClient` struct with `#[derive(uniffi::Object)]`
- [ ] Implement constructor
- [ ] Implement `async fn start_workflow(&self, ...) -> Result<String, FfiError>`
- [ ] Implement `async fn query_workflow(&self, ...) -> Result<Vec<u8>, FfiError>`
- [ ] Implement `async fn signal_workflow(&self, ...) -> Result<(), FfiError>`
- [ ] Export from `ffi/src/lib.rs`
- [ ] Verify: `cargo build -p flovyn-ffi` passes

### Step 7: Create Config Records

- [ ] Create `ffi/src/config.rs`
- [ ] Define `WorkerConfig` record with connection settings
- [ ] Define `ClientConfig` record
- [ ] Export from `ffi/src/lib.rs`
- [ ] Verify: `cargo build -p flovyn-ffi` passes

```rust
// ffi/src/config.rs

#[derive(uniffi::Record)]
pub struct WorkerConfig {
    pub server_url: String,
    pub namespace: String,
    pub task_queue: String,
    pub worker_identity: Option<String>,
    pub max_concurrent_workflow_tasks: Option<u32>,
    pub max_concurrent_tasks: Option<u32>,
}

#[derive(uniffi::Record)]
pub struct ClientConfig {
    pub server_url: String,
    pub namespace: String,
}
```

### Step 8: Re-export Core Types

Create wrapper types or re-export core types that are useful for foreign SDKs.

- [ ] Create `ffi/src/types.rs` for re-exported/wrapped types
- [ ] Create `FfiEventType` enum mirroring `flovyn_core::EventType`
- [ ] Create `FfiWorkerStatus` enum mirroring `flovyn_core::WorkerStatus`
- [ ] Create `FfiTaskExecutionResult` enum mirroring `flovyn_core::TaskExecutionResult`
- [ ] Implement conversions between FFI and core types
- [ ] Verify: `cargo build -p flovyn-ffi` passes

### Step 9: Create uniffi-bindgen Binary

- [ ] Create `ffi/uniffi-bindgen.rs` with main function
- [ ] Add `[[bin]]` entry to `ffi/Cargo.toml`
- [ ] Test binding generation: `cargo run -p flovyn-ffi --bin uniffi-bindgen -- generate --library target/debug/libflovyn_ffi.dylib --language kotlin --out-dir out`
- [ ] Verify: Kotlin bindings generate without errors

```rust
// ffi/uniffi-bindgen.rs
fn main() {
    uniffi::uniffi_bindgen_main()
}
```

```toml
# Add to ffi/Cargo.toml
[[bin]]
name = "uniffi-bindgen"
path = "uniffi-bindgen.rs"
```

### Step 10: Test Generated Bindings

- [ ] Generate Kotlin bindings to `out/kotlin/`
- [ ] Review generated Kotlin code for correctness
- [ ] Verify suspend functions are generated for async methods
- [ ] Verify data classes generated for Record types
- [ ] Verify sealed classes generated for Enum types
- [ ] Generate Swift bindings to `out/swift/` (for completeness)
- [ ] Generate Python bindings to `out/python/` (for completeness)

### Step 11: Documentation and Cleanup

- [ ] Add module-level documentation to `ffi/src/lib.rs` explaining the FFI API
- [ ] Add README.md to `ffi/` directory with usage instructions
- [ ] Update root CLAUDE.md with uniffi build commands
- [ ] Clean up any unused imports or dead code

### Step 12: Final Verification

- [ ] `cargo build --workspace` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo build -p flovyn-ffi` passes
- [ ] `cargo test -p flovyn-ffi` passes
- [ ] Kotlin bindings generate successfully
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo fmt --all -- --check` passes

## Repository Structure After Phase 2

```
sdk-rust/
├── core/                   # Language-agnostic core (UNCHANGED)
│   ├── src/
│   │   ├── client/
│   │   ├── error.rs
│   │   ├── generated/
│   │   ├── task/
│   │   ├── worker/
│   │   ├── workflow/
│   │   └── lib.rs
│   └── Cargo.toml
│
├── ffi/                    # NEW: uniffi bindings crate
│   ├── src/
│   │   ├── lib.rs          # Module root with setup_scaffolding!()
│   │   ├── error.rs        # FfiError type
│   │   ├── activation.rs   # Activation/Completion types
│   │   ├── command.rs      # FfiWorkflowCommand
│   │   ├── worker.rs       # CoreWorker object
│   │   ├── client.rs       # CoreClient object
│   │   ├── config.rs       # Configuration records
│   │   └── types.rs        # Re-exported/wrapped core types
│   ├── uniffi-bindgen.rs   # Binding generator binary
│   ├── Cargo.toml
│   └── README.md
│
├── sdk/                    # Rust SDK (UNCHANGED)
│   └── ...
│
├── examples/               # Examples (UNCHANGED)
│   └── ...
│
└── Cargo.toml              # Workspace (updated to include ffi)
```

## FFI Type Mapping

| Rust Type | uniffi Type | Foreign Type |
|-----------|-------------|--------------|
| `String` | Built-in | String/str |
| `Vec<u8>` | Built-in | ByteArray/Data/bytes |
| `Option<T>` | Built-in | T?/Optional/None |
| `Result<T, FfiError>` | Built-in | throws/raises |
| `Arc<CoreWorker>` | Object | Class instance |
| `WorkflowActivation` | Record | Data class/struct |
| `FfiWorkflowCommand` | Enum | Sealed class/enum |
| `async fn` | Async | suspend/async/await |

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| uniffi version compatibility | Pin to specific version (0.28) |
| Complex types not exportable | Use wrapper types with Vec<u8> for arbitrary data |
| Async runtime conflicts | uniffi uses foreign executor - no Tokio runtime needed on foreign side |
| Large binding size | Only export necessary types; keep internal types private |
| Breaking changes in uniffi | Pin version; test binding generation in CI |
| Core crate changes | ffi crate depends on core; breaking changes in core may require ffi updates |

## Success Criteria

1. `cargo test --workspace` passes (ffi crate doesn't affect existing tests)
2. `cargo build -p flovyn-ffi` compiles successfully
3. `flovyn-core` crate is unchanged (no modifications needed)
4. Kotlin bindings generate without errors
5. Generated bindings include:
   - `CoreWorker` class with suspend functions
   - `CoreClient` class with suspend functions
   - All activation/completion data classes
   - `FfiError` sealed class
6. No changes to Rust SDK public API
