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

# Code quality
cargo fmt --all                                       # Format code
cargo clippy --workspace --all-targets -- -D warnings # Lint
cargo doc --no-deps --workspace                       # Build docs

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

## Documentation

### Location
- **Design documents**: `.dev/docs/design/` - Architecture, API design, technical decisions
- **Implementation plans**: `.dev/docs/plans/` - Step-by-step implementation plans
- **Bug reports**: `.dev/docs/plans/` - Bug report and fixes

### Plan Guidelines
- Plans should NOT repeat content already in design documents - reference them instead
- Plans MUST include a TODO list with concrete tasks
- No "deferred" or "optional" items unless explicitly marked as non-goal or out of scope
- Each TODO item should be actionable and specific

### Testing
- Use test first approach
- Write test one by one. Verify one test work first before moving on the next.

## Prerequisites

- Rust 1.82+
- Protocol Buffers compiler (`protoc`)
- Flovyn server running on `localhost:9090` (for examples/e2e tests)

## Platform Notes (macOS)

### Command Timeouts
The `timeout` command does not exist on macOS. Use these alternatives instead:

```bash
# Option 1: Use Bash tool's run_in_background parameter
# Run command in background, then use BashOutput to check status

# Option 2: Use cargo test's built-in timeout (for tests)
cargo test -- --test-threads=1  # Tests have their own timeout

# Option 3: For E2E tests, use the test's internal timeout mechanism
# E2E tests use tokio::time::timeout() internally
```

**IMPORTANT**: Never use `timeout` command in bash - it will fail with "command not found".
