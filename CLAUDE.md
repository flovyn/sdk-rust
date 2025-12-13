# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
# Build entire workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Run specific test suite (unit, tck, or e2e)
cargo test --test unit -p flovyn-sdk
cargo test --test tck -p flovyn-sdk
cargo test --test e2e -p flovyn-sdk

# Run tests with testing utilities feature
cargo test --features testing

# Code quality (matches CI)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --no-deps --workspace

# Run examples
cargo run -p hello-world-sample
cargo run -p ecommerce-sample
cargo run -p data-pipeline-sample
cargo run -p patterns-sample
```

## Architecture

This is the Rust SDK for Flovyn, a workflow orchestration platform using event sourcing with deterministic replay.

### Workspace Structure

- `sdk/` - Core SDK library (`flovyn-sdk` crate)
- `examples/` - Example applications (hello-world, ecommerce, data-pipeline, patterns)

### SDK Module Layout (`sdk/src/`)

- **client/** - FlovynClient builder, worker lifecycle, workflow dispatch, task execution
- **workflow/** - WorkflowDefinition trait, WorkflowContext, commands, events, replay recorder
- **task/** - TaskDefinition trait, TaskContext, executor, registry
- **worker/** - Workflow/task workers, executor, determinism validation
- **config/** - Client configuration and presets
- **testing/** - Mock contexts, test environment, time controller (feature-gated)
- **generated/** - gRPC/protobuf generated code (`flovyn.v1`)

### Key Traits

- `WorkflowDefinition` / `DynamicWorkflow` - Define typed or JSON-based workflows
- `TaskDefinition` / `DynamicTask` - Define typed or JSON-based tasks
- `WorkflowContext` - Deterministic APIs for workflow execution (time, random, state, tasks, timers, promises)
- `TaskContext` - Progress reporting, logging, cancellation for tasks
- `WorkflowHook` - Lifecycle callbacks (started, completed, failed)

### Determinism Requirement

Workflows must be deterministic for replay. Use context methods instead of standard library:
- `ctx.current_time_millis()` instead of `SystemTime::now()`
- `ctx.random_uuid()` instead of `Uuid::new_v4()`
- `ctx.random()` instead of `rand::random()`

### Testing Feature

Enable `testing` feature for mock contexts and test environment:
```toml
flovyn-sdk = { path = "./sdk", features = ["testing"] }
```

Provides: `MockWorkflowContext`, `MockTaskContext`, `TestWorkflowEnvironment`, `TimeController`, test builders.

## Prerequisites

- Rust 1.82+
- Protocol Buffers compiler (`protoc`)
- Flovyn server running on `localhost:9090` (for examples/e2e tests)
