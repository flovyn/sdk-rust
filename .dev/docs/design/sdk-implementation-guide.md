# Flovyn SDK Implementation Guide

This document captures lessons learned from implementing the Rust SDK (referencing the Kotlin SDK) and provides guidance for implementing SDKs in new languages.

## Table of Contents

1. [Core Concepts](#core-concepts)
2. [Architecture Overview](#architecture-overview)
3. [Implementation Order](#implementation-order)
4. [Critical Implementation Details](#critical-implementation-details)
5. [Language-Specific Adaptations](#language-specific-adaptations)
6. [Testing Strategy](#testing-strategy)
7. [Common Pitfalls](#common-pitfalls)
8. [CI/CD Considerations](#cicd-considerations)

---

## Core Concepts

### What is Flovyn?

Flovyn is a workflow orchestration platform using **event sourcing with deterministic replay**. Understanding this is critical:

- **Workflows** are durable, long-running processes that survive failures
- **Tasks** are units of work that execute actual business logic
- **Events** are the source of truth - the workflow state is rebuilt by replaying events
- **Determinism** is mandatory - workflows must produce the same commands when replayed with the same events

### Key Abstractions

| Concept | Purpose |
|---------|---------|
| `WorkflowDefinition` | Defines workflow logic (must be deterministic) |
| `TaskDefinition` | Defines task logic (can be non-deterministic) |
| `WorkflowContext` | Provides deterministic APIs for workflows |
| `TaskContext` | Provides APIs for task execution |
| `FlovynClient` | Main entry point - configures and starts workers |
| `WorkerHandle` | Manages worker lifecycle |

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      FlovynClient                            │
│  ┌─────────────────┐  ┌─────────────────┐                   │
│  │ WorkflowWorker  │  │   TaskWorker    │                   │
│  │                 │  │                 │                   │
│  │ ┌─────────────┐ │  │ ┌─────────────┐ │                   │
│  │ │  Registry   │ │  │ │  Registry   │ │                   │
│  │ │ (workflows) │ │  │ │  (tasks)    │ │                   │
│  │ └─────────────┘ │  │ └─────────────┘ │                   │
│  │                 │  │                 │                   │
│  │ ┌─────────────┐ │  │ ┌─────────────┐ │                   │
│  │ │  Executor   │ │  │ │  Executor   │ │                   │
│  │ └─────────────┘ │  │ └─────────────┘ │                   │
│  └────────┬────────┘  └────────┬────────┘                   │
│           │                    │                             │
│           └────────┬───────────┘                             │
│                    ▼                                         │
│           ┌─────────────────┐                                │
│           │   gRPC Client   │                                │
│           │ (WorkflowService│                                │
│           │  TaskService)   │                                │
│           └────────┬────────┘                                │
└────────────────────┼────────────────────────────────────────┘
                     │
                     ▼
              ┌─────────────┐
              │   Server    │
              └─────────────┘
```

---

## Implementation Order

Based on experience, follow this order for incremental, testable progress:

### Phase 1: Foundation
1. **gRPC/Protobuf setup** - Generate client stubs from `flovyn.proto`
2. **Basic types** - WorkflowId, TaskId, error types
3. **Configuration** - Server address, timeouts, retry settings

### Phase 2: Core Client
4. **FlovynClient builder** - Fluent API for configuration
5. **Task definitions** - Simple task registration and execution
6. **Workflow definitions** - Basic workflow registration

### Phase 3: Workflow Context
7. **WorkflowContext basics** - `workflow_id()`, `run_id()`
8. **Task scheduling** - `schedule_task()`, `await_task()`
9. **Timer support** - `sleep()`, `sleep_until()`
10. **Deterministic utilities** - `current_time_millis()`, `random_uuid()`, `random()`

### Phase 4: Advanced Features
11. **Child workflows** - `schedule_child_workflow()`, `await_child_workflow()`
12. **Promises** - Manual completion via `create_promise()`, `complete_promise()`
13. **Workflow state** - `get_state()`, `set_state()`
14. **Signals** - `signal()`, `await_signal()`

### Phase 5: Production Readiness
15. **Error handling** - Proper error types, retries
16. **Logging/Metrics** - Observability
17. **Testing utilities** - Mock contexts, test environment
18. **Documentation** - API docs, examples

---

## Critical Implementation Details

### 1. Replay Mechanism (MOST IMPORTANT)

The workflow executor must handle **replay mode** correctly:

```
When executing a workflow:
1. Load all existing events for the workflow
2. Execute workflow code
3. For each command generated:
   - If replaying: match against existing event, return recorded result
   - If not replaying: send command to server, record event
4. Track sequence numbers to detect non-determinism
```

**Sequence numbering is critical:**
- Each command gets a sequence number starting from 1
- During replay, sequence numbers must match exactly
- Mismatch = non-determinism error

### 2. Command/Event Model

Commands are requests from workflow to server:
- `ScheduleTask`
- `ScheduleChildWorkflow`
- `CreateTimer`
- `CreatePromise`
- `CompletePromise`
- `SetState`
- `AwaitSignal`

Events are recorded results:
- `TaskCompleted` / `TaskFailed`
- `ChildWorkflowCompleted` / `ChildWorkflowFailed`
- `TimerFired`
- `PromiseCompleted`
- `StateUpdated`
- `SignalReceived`

### 3. WorkflowContext Implementation

The context must:
- Track current sequence number
- Maintain event history for replay
- Record new events during execution
- Provide deterministic random/time APIs

```
// Pseudocode for schedule_task
fn schedule_task(name, input):
    seq = next_sequence_number()

    if replaying and has_event_at(seq):
        event = get_event_at(seq)
        return event.result  // Return recorded result

    // Not replaying - execute for real
    command = ScheduleTask { seq, name, input }
    send_to_server(command)

    // Block until task completes (or use futures)
    result = await_task_completion()
    record_event(seq, result)
    return result
```

### 4. Worker Polling Loop

```
loop:
    poll_response = grpc_client.poll_workflow_task()

    if poll_response.has_task:
        workflow = registry.get(poll_response.workflow_type)
        context = create_context(poll_response.events)

        try:
            result = workflow.execute(context, input)
            report_completion(result, context.new_commands)
        catch error:
            report_failure(error)

    sleep(poll_interval)
```

### 5. Determinism Requirements

Workflows MUST NOT use:
- System time (`now()`, `SystemTime`, etc.)
- Random number generators
- Thread-local storage
- Network calls (except through tasks)
- File I/O (except through tasks)

Workflows MUST use context methods:
- `ctx.current_time_millis()` - Deterministic timestamp
- `ctx.random_uuid()` - Deterministic UUID
- `ctx.random()` - Deterministic random number
- `ctx.schedule_task()` - For any non-deterministic work

---

## Language-Specific Adaptations

### Do NOT blindly copy the reference SDK

Each language has idioms that should be respected:

| Aspect | Kotlin | Rust | Python | Go |
|--------|--------|------|--------|-----|
| Async | Coroutines | async/await + Futures | asyncio | goroutines |
| Error handling | Exceptions | Result<T, E> | Exceptions | (T, error) |
| Nullability | Nullable types | Option<T> | Optional/None | pointers |
| Generics | Reified generics | Monomorphization | Duck typing | Interface{} |
| Builder pattern | DSL builders | Builder structs | **kwargs | Functional options |

### Rust-Specific Lessons

1. **Trait design**: Use associated types for input/output, not generic parameters
   ```rust
   // Good
   trait WorkflowDefinition {
       type Input;
       type Output;
   }

   // Avoid - causes trait object issues
   trait WorkflowDefinition<I, O> {}
   ```

2. **Async runtime**: Choose one (tokio) and stick with it

3. **Error handling**: Define a proper error enum, implement `std::error::Error`

4. **Lifetimes**: Design APIs to minimize lifetime annotations in user code

5. **Dynamic dispatch**: Sometimes needed for registries - use `Box<dyn Trait>`

### Future SDK Considerations

**Python:**
- Use dataclasses for types
- Consider both sync and async APIs
- Use type hints throughout

**Go:**
- Use interfaces for abstraction
- Context for cancellation
- Functional options for configuration

**TypeScript/JavaScript:**
- Promise-based API
- Consider both Node.js and browser support
- Use TypeScript for type safety

---

## Testing Strategy

### Unit Tests
- Test individual components in isolation
- Mock the gRPC client
- Test replay logic with synthetic events

### Integration Tests (E2E)
- Run against real server (use testcontainers)
- Test full workflow lifecycle
- Test error scenarios

### E2E Test Infrastructure

Create a test helper that:
1. Manages testcontainer lifecycle
2. Provides client setup/teardown
3. Has utilities for common assertions

```rust
// Example from Rust SDK
struct E2ETestEnvironment {
    harness: &'static TestHarness,  // Manages container
    client: Option<FlovynClient>,
    handle: Option<WorkerHandle>,
}

impl E2ETestEnvironment {
    fn start_and_await<W, T>(&mut self, workflow, task, input) -> Result<Output>
    fn await_completion(&self, workflow_id, timeout) -> Result<Output>
    fn assert_completed(&self, workflow_id)
    fn assert_failed(&self, workflow_id)
}
```

### Test Categories

1. **Basic workflow execution** - Simple workflow completes
2. **Task scheduling** - Workflow schedules and awaits tasks
3. **Timers** - Sleep, sleep_until work correctly
4. **Child workflows** - Parent spawns and awaits children
5. **Promises** - Manual completion works
6. **State management** - Get/set state persists
7. **Signals** - Signal delivery works
8. **Error handling** - Failures are handled correctly
9. **Replay/Determinism** - Workflow replays correctly after restart

---

## Common Pitfalls

### 1. Sequence Number Off-by-One
- Sequence numbers start at 1, not 0
- Increment BEFORE using, not after

### 2. Missing Event Types
- Handle ALL event types the server might send
- Unknown events should error, not be silently ignored

### 3. Blocking in Async Context
- Don't block the async runtime
- Use async-aware primitives (channels, mutexes)

### 4. Memory Leaks in Long-Running Workers
- Clean up completed workflow state
- Use weak references where appropriate

### 5. gRPC Connection Management
- Handle reconnection gracefully
- Implement backoff for retries

### 6. Protobuf Code Generation
- Pin protoc version
- Format generated code for consistent CI
- Consider checking generated code into repo

### 7. Thread Safety
- Workers may process multiple workflows concurrently
- Registries must be thread-safe
- Context must be single-threaded per workflow

---

## CI/CD Considerations

### Toolchain Version Pinning

Pin exact versions to avoid "works locally, fails in CI":

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.92.0"
components = ["rustfmt", "clippy"]
```

### Generated Code Formatting

If generating code (protobuf, etc.), ensure consistent formatting:

```rust
// In build.rs - format generated code
let _ = Command::new("rustfmt")
    .arg(generated_file)
    .arg("--edition")
    .arg("2021")
    .status();
```

### CI Pipeline Structure

```yaml
jobs:
  build:
    - checkout
    - install toolchain (pinned version)
    - build
    - test (unit only, no external deps)

  lint:
    - format check
    - linter (clippy, eslint, etc.)

  e2e:
    - start server container
    - run e2e tests
    - cleanup

  docs:
    - build documentation
    - (optional) publish to docs site
```

---

## Checklist for New SDK

### Minimum Viable SDK
- [ ] gRPC client generated from proto
- [ ] FlovynClient with builder pattern
- [ ] WorkflowDefinition trait/interface
- [ ] TaskDefinition trait/interface
- [ ] WorkflowContext with basic methods
- [ ] Worker polling loop
- [ ] Basic replay support
- [ ] Unit tests
- [ ] One working example

### Production Ready
- [ ] All WorkflowContext methods implemented
- [ ] Child workflow support
- [ ] Promise support
- [ ] Signal support
- [ ] State management
- [ ] Proper error types
- [ ] Retry logic
- [ ] Logging
- [ ] E2E tests with testcontainers
- [ ] Documentation
- [ ] Multiple examples
- [ ] CI/CD pipeline

### Nice to Have
- [ ] Mock contexts for testing
- [ ] Test environment utilities
- [ ] Workflow hooks
- [ ] Metrics/tracing integration
- [ ] IDE integration helpers

---

## References

- Kotlin SDK: `/server/sdk/kotlin/` - Reference implementation
- Rust SDK: This repository - Second implementation
- Proto definition: `/proto/flovyn.proto` - gRPC service definition
- Server E2E tests: `/server/app/src/test/kotlin/ai/flovyn/e2e/` - Server test cases
