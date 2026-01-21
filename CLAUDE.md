# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

Use [mise](https://mise.jdx.dev/) for all development tasks:

```bash
# Build entire workspace
mise run build

# Run all tests
mise run test

# Run specific test suite
mise run test:unit            # Unit tests
mise run test:tck             # TCK tests
mise run test:e2e             # E2E tests (requires server)
mise run test:model           # Stateright model checking

# Code quality
mise run check                # Check code without building
mise run fmt                  # Format code
mise run clippy               # Run linter
mise run lint                 # Run fmt check + clippy
mise run doc                  # Build documentation

# Run examples
mise run example:hello        # Hello world
mise run example:ecommerce    # E-commerce sample
mise run example:pipeline     # Data pipeline sample
mise run example:patterns     # Patterns sample

# Utilities
mise run model:check          # Run model checking
mise run cleanup:containers   # Cleanup test containers
```

## Architecture

This is the Rust SDK for Flovyn, a workflow orchestration platform using event sourcing with deterministic replay.

### Workspace Structure

- `worker-core/` - Core library (`flovyn-worker-core` crate) - gRPC clients, protobuf, worker internals
- `worker-sdk/` - Worker SDK library (`flovyn-worker-sdk` crate) - traits, contexts, executors
- `worker-ffi/` - FFI library (`flovyn-worker-ffi` crate) - UniFFI bindings for Kotlin/Swift/Python
- `examples/` - Example applications (hello-world, ecommerce, data-pipeline, patterns)

### SDK Module Layout (`worker-sdk/src/`)

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
flovyn-worker-sdk = { path = "./worker-sdk", features = ["testing"] }
```

Provides: `MockWorkflowContext`, `MockTaskContext`, `TestWorkflowEnvironment`, `TimeController`, test builders.

## Documentation

### Location
Documentation is centralized in the `dev` repo:
- **Design documents**: `dev/docs/design/` - Architecture, API design, technical decisions
- **Implementation plans**: `dev/docs/plans/` - Step-by-step implementation plans
- **Bug reports**: `dev/docs/bugs/` - Bug reports and fixes

### Plan Guidelines
- Before writing a plan, think critically to find things we would have missed from the design
- After each phase, the system should ideally still work, so an incremental approach is preferred.
- Plans should NOT repeat content already in design documents - reference them instead
- Plans MUST include a TODO list with concrete tasks
- No "deferred" or "optional" items unless explicitly marked as non-goal or out of scope
- Each TODO item should be actionable and specific

### Testing
- Use test first approach
- Write test one by one. Verify one test work first before moving on the next.
- For E2E test, use different queue for tests to prevent a worker in a test might pull jobs from another tests.

## Prerequisites

- [mise](https://mise.jdx.dev/) (installs Rust automatically)
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
