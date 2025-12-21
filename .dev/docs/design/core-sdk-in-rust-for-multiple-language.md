# Core SDK in Rust for Multiple Languages

## Overview

This document outlines the plan to extract a shared Rust core (`flovyn-core`) from the existing SDK that can be reused across multiple language SDKs. The core handles all complex logic (determinism, replay, state machines, gRPC communication) while language-specific SDKs provide idiomatic APIs.

For the detailed architecture, component breakdown, and API alignment strategy, see the [Multi-Language SDK Strategy](../research/multi-language-sdk-strategy.md) research document.

## Goals

1. **Extract reusable core**: Split the current `sdk/` crate into `core/` (language-agnostic) and `sdk/` (Rust-specific)
2. **Validate the architecture**: Prove the core works by building a Kotlin/Java SDK on top of it
3. **Maintain API consistency**: Ensure the Rust SDK continues to work with the same API after refactoring
4. **Enable future language SDKs**: Python, TypeScript, and others can build on the same core

## Non-Goals

- Building Python or TypeScript SDKs (future work)
- Changing the Rust SDK's public API
- Modifying the server protocol

## Architecture

### Repository Layout (After Refactoring)

The Rust SDK repository (`sdk-rust`) and Kotlin SDK repository (`sdk-kotlin`) are **separate Git repositories** at the same level:

```
flovyn/
├── sdk-rust/                 # Rust SDK repository (this repo)
│   ├── proto/                # Keep as-is: protobuf definitions
│   ├── examples/             # Keep as-is: Rust examples
│   ├── core/                 # NEW: Language-agnostic core (flovyn-core crate)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── worker/       # Worker lifecycle, polling, slot management
│   │   │   ├── replay/       # Replay engine, determinism validation
│   │   │   ├── state_machine/# Timer, task, promise, child workflow state machines
│   │   │   ├── command/      # Command processing & recording
│   │   │   ├── event/        # Event history management
│   │   │   ├── client/       # gRPC client implementation
│   │   │   ├── activation/   # WorkflowActivation/Completion types
│   │   │   └── error.rs
│   │   └── Cargo.toml
│   └── sdk/                  # Rust-specific layer (flovyn-sdk crate)
│       ├── src/
│       │   ├── lib.rs
│       │   ├── workflow/     # WorkflowDefinition trait, WorkflowContext
│       │   ├── task/         # TaskDefinition trait, TaskContext
│       │   ├── client/       # FlovynClient builder (uses core::client)
│       │   ├── worker/       # Worker builder (uses core::worker)
│       │   ├── testing/      # Mock contexts, test environment
│       │   └── config/       # Client configuration
│       └── Cargo.toml
│
└── sdk-kotlin/               # Kotlin SDK repository (separate repo)
    └── (see below)
```

### What Goes Where

| Component | Location | Reason |
|-----------|----------|--------|
| gRPC Client | `core/` | Protocol handling is language-agnostic |
| Worker Manager | `core/` | Worker lifecycle, polling, shutdown |
| Replay Engine | `core/` | Deterministic replay is core logic |
| State Machines | `core/` | Timer, task, promise state machines |
| Command Processing | `core/` | Command validation and recording |
| Event Sourcing | `core/` | History processing |
| Determinism Validation | `core/` | Critical for correctness |
| WorkflowDefinition trait | `sdk/` | Rust-specific trait with generics |
| TaskDefinition trait | `sdk/` | Rust-specific trait with generics |
| WorkflowContext | `sdk/` | Rust-idiomatic context API |
| TaskContext | `sdk/` | Rust-idiomatic context API |
| FlovynClient builder | `sdk/` | Rust-idiomatic builder pattern |
| Testing utilities | `sdk/` | Rust-specific test helpers |

### Core Protocol

The core exposes an activation-based protocol (similar to Temporal's sdk-core):

```rust
// core/src/lib.rs

/// Worker trait exposed to language bindings
pub trait CoreWorker: Send + Sync {
    /// Poll for the next workflow activation
    async fn poll_workflow_activation(&self) -> Result<WorkflowActivation>;

    /// Complete a workflow activation with commands
    async fn complete_workflow_activation(&self, completion: WorkflowActivationCompletion) -> Result<()>;

    /// Poll for the next task
    async fn poll_task(&self) -> Result<TaskActivation>;

    /// Complete a task
    async fn complete_task(&self, completion: TaskCompletion) -> Result<()>;

    /// Initiate graceful shutdown
    fn initiate_shutdown(&self);
}
```

## Kotlin/Java SDK (Proof of Concept)

To validate the architecture, we'll build a Kotlin SDK in a **separate Git repository** (`sdk-kotlin`) that uses the Rust core via JNI (using [uniffi](https://mozilla.github.io/uniffi-rs/)).

### Kotlin SDK Structure

```
sdk-kotlin/                            # Separate Git repository
├── core/                              # Kotlin core module (wraps Rust core)
│   ├── build.gradle.kts
│   └── src/main/kotlin/io/flovyn/core/
│       ├── CoreWorker.kt              # JNI bindings to Rust core
│       ├── Activation.kt              # Kotlin activation types
│       └── FlovynRuntime.kt           # Runtime initialization
├── sdk-jackson/                       # Jackson serialization variant
│   ├── build.gradle.kts
│   └── src/main/kotlin/io/flovyn/sdk/
│       ├── Workflow.kt                # @Workflow annotation
│       ├── Task.kt                    # @Task annotation
│       ├── WorkflowContext.kt         # Kotlin workflow context
│       ├── TaskContext.kt             # Kotlin task context
│       ├── Worker.kt                  # Worker builder
│       └── Client.kt                  # Flovyn client
├── native/                            # Rust JNI bridge (depends on flovyn-core)
│   ├── Cargo.toml                     # References sdk-rust/core as git dependency
│   └── src/lib.rs                     # uniffi bindings
├── examples/                          # Kotlin examples
└── build.gradle.kts
```

The `native/Cargo.toml` will reference `flovyn-core` as a git dependency:

```toml
[dependencies]
flovyn-core = { git = "https://github.com/flovyn/sdk-rust", branch = "main" }
uniffi = "0.25"
```

### Initial Scope

For the proof of concept, focus on:

1. **core + sdk-jackson modules only** - Other variants (kotlinx-serialization, java) are out of scope
2. **Basic workflow execution** - Start workflow, schedule tasks, timers
3. **E2E tests** - Verify end-to-end functionality against the server

### API Alignment

The Kotlin SDK should follow the API alignment defined in the [Multi-Language SDK Strategy](../research/multi-language-sdk-strategy.md#aligned-api-design-across-languages):

```kotlin
// Kotlin workflow example
@Workflow(name = "order-workflow", version = "1.0.0")
class OrderWorkflow {
    @WorkflowMethod
    fun execute(ctx: WorkflowContext, input: OrderInput): OrderResult {
        val startTime = ctx.currentTimeMillis()
        val uuid = ctx.randomUuid()

        val paymentFuture = ctx.schedule(ProcessPaymentTask::class, input)
        ctx.sleep(Duration.ofSeconds(30))

        val result = paymentFuture.get()
        return OrderResult(input.orderId, "completed")
    }
}
```

## Implementation Phases

Each phase is **incremental** - after completing each phase, the system should be fully functional and all tests should pass.

### Phase 1: Extract Core (Rust) - sdk-rust repository

**Goal**: Split `sdk/` into `core/` and `sdk/` within the same repository. The Rust SDK continues to work exactly as before.

**Steps**:
1. Create `core/` crate with activation-based API
2. Move worker, replay, state machine, client, gRPC code to `core/`
3. Update `sdk/` to depend on `core/` and re-export necessary types
4. No public API changes to `flovyn-sdk` - existing code compiles without changes
5. All existing unit, TCK, and E2E tests pass

**Verification**: `cargo test --workspace` passes, examples run successfully.

### Phase 2: Add uniffi Bindings - sdk-rust repository

**Goal**: Add uniffi annotations to `flovyn-core` so it can be consumed by other languages. The Rust SDK remains unaffected.

**Steps**:
1. Add uniffi as optional dependency to `core/`
2. Add `#[uniffi::export]` annotations to public APIs
3. Create uniffi bindings generation in build script
4. Verify Rust SDK still works (uniffi is optional, not required for Rust usage)

**Verification**: `cargo test --workspace` passes, uniffi bindings generate without errors.

### Phase 3: Kotlin SDK - sdk-kotlin repository (new repo)

**Goal**: Create a new `sdk-kotlin` repository with JNI bridge and Kotlin SDK.

**Steps**:
1. Create `sdk-kotlin` repository
2. Implement `native/` Rust crate that depends on `flovyn-core` via git
3. Generate Kotlin bindings from uniffi
4. Implement `core/` Kotlin module wrapping native bindings
5. Implement `sdk-jackson/` with annotations and context
6. Create basic examples
7. Write E2E tests against the server

**Verification**: Kotlin E2E tests pass, examples run successfully.

## Success Criteria

### Per-Phase Criteria

**Phase 1 (Extract Core)**:
- All existing Rust SDK tests pass
- No breaking changes to `flovyn-sdk` public API
- Examples compile and run without modification

**Phase 2 (uniffi Bindings)**:
- uniffi bindings generate successfully
- Rust SDK tests still pass (uniffi is optional)
- Generated bindings compile for target platforms (macOS, Linux)

**Phase 3 (Kotlin SDK)**:
- Kotlin SDK can execute workflows end-to-end
- Kotlin SDK API matches the alignment specification
- E2E tests pass against the server

### Overall Criteria

1. Incremental delivery - each phase leaves the system in a working state
2. Performance is comparable to native Rust execution
3. Build and test infrastructure works for both repositories

## Testing Strategy

### Core Tests (Rust)

- Unit tests for state machines, replay, command processing
- Integration tests with mock server
- TCK (Technology Compatibility Kit) tests with recorded histories

### Kotlin SDK Tests

- Unit tests for Kotlin wrappers and annotations
- E2E tests against real server (using different task queues to avoid conflicts)
- Cross-language TCK validation (same histories as Rust)

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| JNI complexity | Use uniffi for automatic binding generation |
| Async bridging between Kotlin and Rust | Use Kotlin coroutines with suspend functions |
| Performance overhead | Profile and optimize hot paths; protobuf for activation protocol |
| Breaking existing Rust SDK | Extensive test coverage; no public API changes in Phase 1; incremental phases with verification |
| Cross-repo dependency management | Use git dependencies with pinned versions; CI validates compatibility |
| uniffi version compatibility | Pin uniffi version across both repos; test binding generation in CI |

## References

- [Multi-Language SDK Strategy](../research/multi-language-sdk-strategy.md) - Detailed architecture and API alignment
- [Temporal SDK Core](https://github.com/temporalio/sdk-core) - Reference implementation
- [uniffi Documentation](https://mozilla.github.io/uniffi-rs/) - Rust FFI bindings for multiple languages
