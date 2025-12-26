# Multi-Language SDK Strategy

## Overview

This document outlines the strategy for supporting multiple language SDKs (TypeScript, Python, Java, etc.) while maximizing code reuse. The approach follows Temporal's proven model: a shared Rust core (`flovyn-core`) with language-specific bindings that provide idiomatic developer experiences.

## Design Goals

1. **Maximize Code Reuse**: Core logic (determinism, replay, state machines, gRPC, event sourcing) lives in Rust
2. **Idiomatic Language Experience**: Each SDK feels native to its language (decorators in Python, annotations in Java, etc.)
3. **Consistent Behavior**: All SDKs behave identically for core operations
4. **Independent Release Cycles**: Language SDKs can evolve independently for language-specific features
5. **Performance**: Rust core provides optimal performance for critical paths

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Application Layer                                   │
│                    (User's Workflows & Tasks)                                │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                    ┌───────────────┼───────────────┐
                    ▼               ▼               ▼
┌─────────────────────┐ ┌─────────────────────┐ ┌─────────────────────┐
│   Python SDK        │ │   TypeScript SDK    │ │   Java SDK          │
│   (temporalio)      │ │   (@flovyn/sdk)     │ │   (io.flovyn:sdk)   │
│                     │ │                     │ │                     │
│ • Decorators        │ │ • Decorators        │ │ • Annotations       │
│ • async/await       │ │ • Promises          │ │ • CompletableFuture │
│ • Type hints        │ │ • Types             │ │ • Generics          │
│ • Pythonic API      │ │ • Idiomatic JS API  │ │ • Idiomatic Java    │
└─────────┬───────────┘ └─────────┬───────────┘ └─────────┬───────────┘
          │                       │                       │
          │ PyO3                  │ Neon/napi-rs          │ JNI / uniffi
          ▼                       ▼                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                       flovyn-core-bridge                                     │
│               (Language Binding Layer - Rust)                                │
│                                                                              │
│  • Language-specific FFI (PyO3, Neon, JNI)                                  │
│  • Type conversion (Rust <-> Lang types)                                    │
│  • Async runtime bridging                                                    │
│  • Protobuf serialization layer                                              │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           flovyn-core                                        │
│                    (Shared Rust Core Library)                                │
│                                                                              │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐       │
│  │   Worker     │ │   Replay     │ │   State      │ │   gRPC       │       │
│  │   Manager    │ │   Engine     │ │   Machines   │ │   Client     │       │
│  └──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘       │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐       │
│  │   Command    │ │   Event      │ │   Determinism│ │   Telemetry  │       │
│  │   Processor  │ │   Sourcing   │ │   Validation │ │   & Tracing  │       │
│  └──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘       │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                          Flovyn Server                                       │
│                    (Event Sourcing Backend)                                  │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Component Breakdown

### 1. flovyn-core (Rust)

The shared core library containing all language-agnostic logic:

```
flovyn-core/
├── src/
│   ├── lib.rs
│   ├── worker/
│   │   ├── mod.rs
│   │   ├── manager.rs          # Worker lifecycle management
│   │   ├── poller.rs           # Task polling
│   │   └── executor.rs         # Workflow/task execution coordination
│   ├── replay/
│   │   ├── mod.rs
│   │   ├── engine.rs           # Core replay logic
│   │   └── validator.rs        # Determinism validation
│   ├── state_machine/
│   │   ├── mod.rs
│   │   ├── timer.rs            # Timer state machine
│   │   ├── task.rs             # Task state machine
│   │   ├── promise.rs          # Promise state machine
│   │   └── child_workflow.rs   # Child workflow state machine
│   ├── command/
│   │   ├── mod.rs
│   │   └── processor.rs        # Command processing & recording
│   ├── event/
│   │   ├── mod.rs
│   │   └── history.rs          # Event history management
│   ├── client/
│   │   ├── mod.rs
│   │   └── grpc.rs             # gRPC client implementation
│   ├── activation/
│   │   ├── mod.rs
│   │   ├── workflow.rs         # WorkflowActivation types
│   │   └── completion.rs       # WorkflowActivationCompletion types
│   ├── telemetry/
│   │   ├── mod.rs
│   │   └── tracing.rs          # Tracing and metrics
│   └── error.rs                # Error types
├── proto/
│   └── flovyn/
│       └── core/
│           ├── activation.proto       # Lang SDK <-> Core protocol
│           ├── workflow_commands.proto
│           └── common.proto
└── Cargo.toml
```

#### Core Protocol (Activation-based)

Following Temporal's model, the core exposes a protobuf-based protocol:

```protobuf
// activation.proto - Protocol between Core and Lang SDK

// Sent from Core to Lang SDK when workflow code needs to run
message WorkflowActivation {
  string run_id = 1;
  int64 timestamp_ms = 2;
  bool is_replaying = 3;
  bytes random_seed = 4;
  repeated WorkflowActivationJob jobs = 5;
}

// Individual jobs within an activation
message WorkflowActivationJob {
  oneof variant {
    InitializeWorkflow initialize = 1;
    FireTimer fire_timer = 2;
    ResolveTask resolve_task = 3;
    ResolvePromise resolve_promise = 4;
    ResolveChildWorkflow resolve_child_workflow = 5;
    CancelWorkflow cancel_workflow = 6;
    QueryWorkflow query = 7;
    SignalWorkflow signal = 8;
  }
}

// Response from Lang SDK to Core
message WorkflowActivationCompletion {
  string run_id = 1;
  oneof status {
    Successful successful = 2;
    Failed failed = 3;
  }
}

message Successful {
  repeated WorkflowCommand commands = 1;
}

message WorkflowCommand {
  oneof variant {
    ScheduleTask schedule_task = 1;
    StartTimer start_timer = 2;
    CompleteWorkflow complete_workflow = 3;
    FailWorkflow fail_workflow = 4;
    CreatePromise create_promise = 5;
    ScheduleChildWorkflow schedule_child_workflow = 6;
    SetState set_state = 7;
    CancelTimer cancel_timer = 8;
    RequestCancelTask request_cancel_task = 9;
    // ... other commands
  }
}
```

#### Key Core APIs

```rust
// flovyn-core/src/lib.rs

/// Initialize the Flovyn runtime
pub fn init_runtime(config: RuntimeConfig) -> Result<Runtime>;

/// Create a new worker
pub fn init_worker(
    runtime: &Runtime,
    config: WorkerConfig,
    client: Arc<dyn GrpcClient>,
) -> Result<Worker>;

/// Core Worker trait exposed to language bindings
pub trait Worker: Send + Sync {
    /// Poll for the next workflow activation
    async fn poll_workflow_activation(&self) -> Result<WorkflowActivation, PollError>;

    /// Poll for the next task (activity)
    async fn poll_task(&self) -> Result<TaskActivation, PollError>;

    /// Complete a workflow activation with commands
    async fn complete_workflow_activation(
        &self,
        completion: WorkflowActivationCompletion
    ) -> Result<()>;

    /// Complete a task
    async fn complete_task(&self, completion: TaskCompletion) -> Result<()>;

    /// Report task heartbeat
    fn record_heartbeat(&self, heartbeat: TaskHeartbeat);

    /// Initiate graceful shutdown
    fn initiate_shutdown(&self);

    /// Wait for complete shutdown
    async fn finalize_shutdown(self);

    /// Validate worker configuration
    async fn validate(&self) -> Result<()>;
}
```

### 2. flovyn-core-bridge (Language Bindings)

Thin adapters exposing flovyn-core to each language. This can be:

#### Option A: Unified Bridge (uniffi)

Use Mozilla's [uniffi](https://github.com/aspect-rs/aspect-lang/tree/main) for automatic binding generation:

```rust
// flovyn-core-bridge/src/lib.rs

#[uniffi::export]
pub fn init_runtime(config: RuntimeConfig) -> Result<Arc<Runtime>, FlovynError>;

#[uniffi::export]
impl Worker {
    pub async fn poll_workflow_activation(&self) -> Result<Vec<u8>, FlovynError>;
    pub async fn complete_workflow_activation(&self, proto: Vec<u8>) -> Result<(), FlovynError>;
    // ...
}
```

Pros:
- Single codebase generates Swift, Kotlin, Python, Ruby bindings
- Consistent API across languages
- Less maintenance

Cons:
- Less control over language-specific optimizations
- May not support all async patterns

#### Option B: Per-Language Bridges (Temporal's approach)

Separate bridge crates for each language:

```
flovyn-core-bridge/
├── python/           # PyO3 bindings
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── worker.rs
│       ├── runtime.rs
│       └── client.rs
├── typescript/       # Neon/napi-rs bindings
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       └── ...
├── java/             # JNI bindings (or via uniffi)
│   ├── Cargo.toml
│   └── src/
│       └── ...
└── c-bridge/         # C FFI for other languages
    ├── Cargo.toml
    ├── include/
    │   └── flovyn.h
    └── src/
        └── lib.rs
```

**Python Bridge Example (PyO3):**

```rust
// flovyn-core-bridge/python/src/lib.rs

use pyo3::prelude::*;
use pyo3_asyncio::tokio::future_into_py;

#[pymodule]
fn flovyn_bridge(py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<RuntimeRef>()?;
    m.add_class::<WorkerRef>()?;
    m.add_class::<ClientRef>()?;
    m.add_function(wrap_pyfunction!(init_runtime, m)?)?;
    m.add_function(wrap_pyfunction!(new_worker, m)?)?;
    Ok(())
}

#[pyclass]
pub struct WorkerRef {
    worker: Arc<flovyn_core::Worker>,
    runtime: Runtime,
}

#[pymethods]
impl WorkerRef {
    fn poll_workflow_activation<'p>(&self, py: Python<'p>) -> PyResult<&'p PyAny> {
        let worker = self.worker.clone();
        future_into_py(py, async move {
            let activation = worker.poll_workflow_activation().await?;
            Ok(activation.encode_to_vec())  // Return protobuf bytes
        })
    }

    fn complete_workflow_activation<'p>(
        &self,
        py: Python<'p>,
        proto: &PyBytes,
    ) -> PyResult<&'p PyAny> {
        let worker = self.worker.clone();
        let completion = WorkflowActivationCompletion::decode(proto.as_bytes())?;
        future_into_py(py, async move {
            worker.complete_workflow_activation(completion).await?;
            Ok(())
        })
    }
}
```

**TypeScript Bridge Example (napi-rs):**

```rust
// flovyn-core-bridge/typescript/src/lib.rs

use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi]
pub struct WorkerRef {
    worker: Arc<flovyn_core::Worker>,
    runtime: Runtime,
}

#[napi]
impl WorkerRef {
    #[napi]
    pub async fn poll_workflow_activation(&self) -> Result<Buffer> {
        let activation = self.worker.poll_workflow_activation().await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(activation.encode_to_vec().into())
    }

    #[napi]
    pub async fn complete_workflow_activation(&self, proto: Buffer) -> Result<()> {
        let completion = WorkflowActivationCompletion::decode(&proto[..])?;
        self.worker.complete_workflow_activation(completion).await
            .map_err(|e| Error::from_reason(e.to_string()))
    }
}
```

### 3. Language SDKs

Each language SDK provides idiomatic APIs on top of the bridge:

#### Python SDK

```python
# flovyn/workflow.py

from dataclasses import dataclass
from datetime import timedelta
from typing import TypeVar, Generic
import flovyn_bridge  # Native Rust extension

T = TypeVar('T')

def defn(cls=None, *, name: str | None = None):
    """Decorator for workflow classes."""
    def decorator(cls):
        _Definition._apply_to_class(cls, name=name or cls.__name__)
        return cls
    if cls is not None:
        return decorator(cls)
    return decorator

def run(fn):
    """Decorator for the workflow run method."""
    setattr(fn, "__flovyn_workflow_run", True)
    return fn

# Context functions available within workflow
def current_time_millis() -> int:
    """Get deterministic current time."""
    return _current_activation().timestamp_ms

async def sleep(duration: timedelta) -> None:
    """Sleep for the specified duration."""
    return await _schedule_timer(duration)

async def execute_task(
    task: type | str,
    input: any,
    *,
    timeout: timedelta | None = None,
) -> any:
    """Execute a task and await its result."""
    return await _schedule_task(task, input, timeout=timeout)

# Example usage
@defn
class OrderWorkflow:
    @run
    async def execute(self, order_id: str) -> str:
        # Deterministic time
        start_time = current_time_millis()

        # Execute task
        result = await execute_task(ProcessOrder, order_id)

        # Sleep (timer)
        await sleep(timedelta(seconds=30))

        return result
```

```python
# flovyn/task.py

def defn(fn=None, *, name: str | None = None):
    """Decorator for task functions."""
    def decorator(fn):
        setattr(fn, "__flovyn_task_name", name or fn.__name__)
        return fn
    if fn is not None:
        return decorator(fn)
    return decorator

def heartbeat(details: any = None) -> None:
    """Report task heartbeat."""
    _current_task_context().heartbeat(details)

@defn
async def process_order(order_id: str) -> str:
    # Real I/O allowed here
    heartbeat({"progress": 0.5})
    return f"Processed {order_id}"
```

```python
# flovyn/worker.py

class Worker:
    def __init__(
        self,
        client: Client,
        task_queue: str,
        workflows: list[type] = [],
        tasks: list[Callable] = [],
    ):
        self._workflows = {w.__flovyn_name__: w for w in workflows}
        self._tasks = {t.__flovyn_task_name__: t for t in tasks}
        self._bridge_worker = flovyn_bridge.new_worker(
            client._ref,
            self._build_config(task_queue)
        )

    async def run(self) -> None:
        """Run the worker until shutdown."""
        await asyncio.gather(
            self._poll_workflows(),
            self._poll_tasks(),
        )

    async def _poll_workflows(self):
        while True:
            try:
                # Get activation from core (protobuf bytes)
                activation_bytes = await self._bridge_worker.poll_workflow_activation()
                activation = WorkflowActivation.FromString(activation_bytes)

                # Run workflow code in Python
                completion = await self._handle_activation(activation)

                # Send completion back to core
                await self._bridge_worker.complete_workflow_activation(
                    completion.SerializeToString()
                )
            except PollShutdownError:
                break
```

#### TypeScript SDK

```typescript
// @flovyn/sdk/src/workflow.ts

export interface WorkflowContext {
  currentTimeMillis(): number;
  sleep(duration: Duration): Promise<void>;
  scheduleTask<T>(task: TaskDef<T>, input: any): Promise<T>;
  // ...
}

export function defineWorkflow<I, O>(
  name: string,
  fn: (ctx: WorkflowContext, input: I) => Promise<O>
): WorkflowDefinition<I, O> {
  return {
    name,
    execute: fn,
  };
}

// Usage
export const orderWorkflow = defineWorkflow<OrderInput, OrderResult>(
  'OrderWorkflow',
  async (ctx, input) => {
    const startTime = ctx.currentTimeMillis();

    const result = await ctx.scheduleTask(processOrder, input.orderId);

    await ctx.sleep({ seconds: 30 });

    return result;
  }
);
```

```typescript
// @flovyn/sdk/src/worker.ts

import { Worker as BridgeWorker } from './bridge';

export class Worker {
  private bridge: BridgeWorker;
  private workflows: Map<string, WorkflowDefinition>;
  private tasks: Map<string, TaskDefinition>;

  constructor(options: WorkerOptions) {
    this.bridge = new BridgeWorker(options.client._ref, {
      taskQueue: options.taskQueue,
      // ...
    });
    this.workflows = new Map(options.workflows.map(w => [w.name, w]));
    this.tasks = new Map(options.tasks.map(t => [t.name, t]));
  }

  async run(): Promise<void> {
    await Promise.all([
      this.pollWorkflows(),
      this.pollTasks(),
    ]);
  }

  private async pollWorkflows(): Promise<void> {
    while (true) {
      const activationBytes = await this.bridge.pollWorkflowActivation();
      const activation = WorkflowActivation.decode(activationBytes);

      const completion = await this.handleActivation(activation);

      await this.bridge.completeWorkflowActivation(
        WorkflowActivationCompletion.encode(completion).finish()
      );
    }
  }
}
```

## Data Flow

### Workflow Execution Flow

```
┌──────────────┐        ┌──────────────┐        ┌──────────────┐
│  Flovyn      │        │  flovyn-core │        │  Lang SDK    │
│  Server      │        │  (Rust)      │        │  (Python)    │
└──────┬───────┘        └──────┬───────┘        └──────┬───────┘
       │                       │                       │
       │ 1. PollWorkflow       │                       │
       │◄──────────────────────│                       │
       │                       │                       │
       │ 2. WorkflowExecution  │                       │
       │      (with events)    │                       │
       │──────────────────────►│                       │
       │                       │                       │
       │                       │ 3. Process events,    │
       │                       │    build activation   │
       │                       │                       │
       │                       │ 4. WorkflowActivation │
       │                       │    (protobuf bytes)   │
       │                       │──────────────────────►│
       │                       │                       │
       │                       │               5. Run workflow code
       │                       │                  (user's Python)
       │                       │                       │
       │                       │ 6. WorkflowActivation │
       │                       │    Completion         │
       │                       │    (protobuf bytes)   │
       │                       │◄──────────────────────│
       │                       │                       │
       │                       │ 7. Validate commands, │
       │                       │    process state      │
       │                       │                       │
       │ 8. SubmitCommands     │                       │
       │◄──────────────────────│                       │
       │                       │                       │
```

### Task Execution Flow

```
┌──────────────┐        ┌──────────────┐        ┌──────────────┐
│  Flovyn      │        │  flovyn-core │        │  Lang SDK    │
│  Server      │        │  (Rust)      │        │  (Python)    │
└──────┬───────┘        └──────┬───────┘        └──────┬───────┘
       │                       │                       │
       │ 1. PollTask           │                       │
       │◄──────────────────────│                       │
       │                       │                       │
       │ 2. TaskActivation     │                       │
       │──────────────────────►│                       │
       │                       │                       │
       │                       │ 3. TaskActivation     │
       │                       │    (protobuf bytes)   │
       │                       │──────────────────────►│
       │                       │                       │
       │                       │               4. Run task code
       │                       │                  (real I/O allowed)
       │                       │                       │
       │                       │ 5. TaskHeartbeat      │
       │                       │◄──────────────────────│
       │ 6. Heartbeat          │                       │
       │◄──────────────────────│                       │
       │                       │                       │
       │                       │ 7. TaskCompletion     │
       │                       │◄──────────────────────│
       │                       │                       │
       │ 8. CompleteTask       │                       │
       │◄──────────────────────│                       │
```

## What Lives Where

### In flovyn-core (Rust)

| Component | Description |
|-----------|-------------|
| gRPC Client | All server communication |
| Worker Manager | Worker lifecycle, shutdown |
| Polling | Long-poll and notification handling |
| Event Processing | History processing, event sourcing |
| State Machines | Timer, Task, Promise, ChildWorkflow state machines |
| Replay Engine | Deterministic replay logic |
| Determinism Validation | Command sequence validation |
| Command Recording | Tracking and validating commands |
| Telemetry | Tracing, metrics collection |
| Slot Suppliers | Concurrency limiting |
| Random/Time | Deterministic RNG and time |

### In Language SDK

| Component | Description |
|-----------|-------------|
| Decorators/Annotations | Language-native workflow/task definition |
| Workflow Context | Language-native context API |
| Task Context | Language-native task API |
| Type System | Generics, type hints, annotations |
| Serialization | Language-native JSON/protobuf handling |
| Async Runtime | asyncio (Python), Promises (JS), CompletableFuture (Java) |
| Registry | Workflow/task registration and lookup |
| Sandbox (optional) | Workflow code isolation (Python-specific) |
| Testing Utilities | Language-native test helpers |

## Migration Path

### Phase 1: Extract Core

1. Identify core components in current `flovyn-sdk`:
   - Worker executor
   - Replay logic
   - Command/event processing
   - gRPC client

2. Create `flovyn-core` crate with activation-based API

3. Refactor `flovyn-sdk` (Rust) to use `flovyn-core`

4. Validate parity with existing tests

### Phase 2: Python SDK

1. Create `flovyn-core-bridge/python` with PyO3

2. Build Python SDK with:
   - Decorator-based workflow definition
   - async/await support
   - Type hints
   - pytest integration

3. Port examples and tests

### Phase 3: TypeScript SDK

1. Create `flovyn-core-bridge/typescript` with napi-rs

2. Build TypeScript SDK with:
   - Type-safe workflow definitions
   - Promise-based async
   - Full TypeScript types
   - Jest/Vitest integration

### Phase 4: Additional Languages

- Java (JNI or uniffi)
- Ruby (uniffi or magnus)
- Go (cgo)

## Build & Distribution

### Python

```toml
# pyproject.toml
[build-system]
requires = ["maturin>=1.0,<2.0"]
build-backend = "maturin"

[tool.maturin]
manifest-path = "flovyn/bridge/Cargo.toml"
module-name = "flovyn.bridge.flovyn_bridge"
```

Build: `maturin develop` or `maturin build`

### TypeScript

```json
// package.json
{
  "name": "@flovyn/sdk",
  "napi": {
    "targets": ["x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
  }
}
```

Build: `npm run build` (runs napi-rs)

### Java

Publish native libraries as Maven classifier artifacts:
- `flovyn-core-native-macos-aarch64`
- `flovyn-core-native-linux-x86_64`
- etc.

## Testing Strategy

### Core Tests (Rust)

- Unit tests for state machines
- Integration tests with mock server
- TCK (Technology Compatibility Kit) tests
- Replay tests with recorded histories

### Language SDK Tests

- Unit tests for decorators/annotations
- Integration tests with flovyn-core
- E2E tests with real server
- Cross-language TCK validation

### Cross-Language Verification

Run the same workflow in all SDKs against the same history to verify identical behavior:

```
test_histories/
├── simple_workflow.json
├── timer_workflow.json
├── parallel_tasks.json
└── child_workflow.json
```

Each SDK replays these histories and verifies identical command sequences.

## Comparison with Temporal

| Aspect | Temporal | Flovyn |
|--------|----------|--------|
| Core Language | Rust | Rust |
| Core Location | `sdk-core` repo | `flovyn-core` crate |
| Protocol | Protobuf (Activations) | Protobuf (Activations) |
| Python Binding | PyO3 | PyO3 |
| TypeScript Binding | Neon | napi-rs |
| Java Binding | JNI | uniffi or JNI |
| API Style | Activation/Completion | Activation/Completion |
| Determinism | State machines | State machines |

## Open Questions

1. **uniffi vs per-language bindings**: Should we use uniffi for automatic bindings or custom bindings for each language?
   - Recommendation: Start with PyO3/napi-rs for Python/TypeScript, evaluate uniffi for Java/Ruby

2. **Activation protocol version**: How do we handle protocol evolution?
   - Include version in protobuf, support backward compatibility

3. **Sandbox support**: Should we support Python workflow sandboxing?
   - Temporal does this for determinism enforcement; evaluate necessity

4. **Local activities**: Handle in core or language layer?
   - Core for consistency, language layer for language-specific optimizations

## Aligned API Design Across Languages

The goal is to have APIs that are **semantically identical** across all languages, with only syntactic differences to match each language's idioms. Developers should be able to switch languages with minimal cognitive overhead.

### API Alignment Table

#### WorkflowContext

| Rust | Python | TypeScript | Java |
|------|--------|------------|------|
| `ctx.workflow_execution_id()` | `ctx.workflow_execution_id` | `ctx.workflowExecutionId` | `ctx.getWorkflowExecutionId()` |
| `ctx.tenant_id()` | `ctx.tenant_id` | `ctx.tenantId` | `ctx.getTenantId()` |
| `ctx.current_time_millis()` | `ctx.current_time_millis()` | `ctx.currentTimeMillis()` | `ctx.currentTimeMillis()` |
| `ctx.random_uuid()` | `ctx.random_uuid()` | `ctx.randomUuid()` | `ctx.randomUuid()` |
| `ctx.random().next_int(0, 100)` | `ctx.random.next_int(0, 100)` | `ctx.random.nextInt(0, 100)` | `ctx.getRandom().nextInt(0, 100)` |
| `ctx.schedule::<Task>(input)` | `ctx.schedule(Task, input)` | `ctx.schedule(Task, input)` | `ctx.schedule(Task.class, input)` |
| `ctx.schedule_raw("task", input)` | `ctx.schedule_raw("task", input)` | `ctx.scheduleRaw("task", input)` | `ctx.scheduleRaw("task", input)` |
| `ctx.sleep(Duration::from_secs(10))` | `ctx.sleep(timedelta(seconds=10))` | `ctx.sleep({seconds: 10})` | `ctx.sleep(Duration.ofSeconds(10))` |
| `ctx.promise::<T>("name")` | `ctx.promise("name")` | `ctx.promise<T>("name")` | `ctx.promise("name", T.class)` |
| `ctx.schedule_workflow::<W>("n", i)` | `ctx.schedule_workflow(W, "n", i)` | `ctx.scheduleWorkflow(W, "n", i)` | `ctx.scheduleWorkflow(W.class, "n", i)` |
| `ctx.get::<T>("key").await` | `await ctx.get("key")` | `await ctx.get<T>("key")` | `ctx.get("key", T.class).join()` |
| `ctx.set("key", value).await` | `await ctx.set("key", value)` | `await ctx.set("key", value)` | `ctx.set("key", value).join()` |
| `ctx.is_cancellation_requested()` | `ctx.is_cancellation_requested()` | `ctx.isCancellationRequested()` | `ctx.isCancellationRequested()` |

#### TaskContext

| Rust | Python | TypeScript | Java |
|------|--------|------------|------|
| `ctx.task_execution_id()` | `ctx.task_execution_id` | `ctx.taskExecutionId` | `ctx.getTaskExecutionId()` |
| `ctx.attempt()` | `ctx.attempt` | `ctx.attempt` | `ctx.getAttempt()` |
| `ctx.report_progress(0.5, "msg").await` | `await ctx.report_progress(0.5, "msg")` | `await ctx.reportProgress(0.5, "msg")` | `ctx.reportProgress(0.5, "msg")` |
| `ctx.is_cancelled()` | `ctx.is_cancelled()` | `ctx.isCancelled()` | `ctx.isCancelled()` |
| `ctx.heartbeat(details)` | `ctx.heartbeat(details)` | `ctx.heartbeat(details)` | `ctx.heartbeat(details)` |

#### WorkflowDefinition

| Rust | Python | TypeScript | Java |
|------|--------|------------|------|
| `impl WorkflowDefinition` | `@workflow.defn` | `defineWorkflow()` | `@Workflow` |
| `fn kind(&self) -> &str` | `name` param or class name | `name` param | `@Workflow(name=)` |
| `async fn execute(...)` | `@workflow.run async def` | `execute: async (ctx, input) =>` | `@WorkflowMethod` |
| `fn version(&self)` | `version` param | `version` param | `@Workflow(version=)` |
| `fn timeout_seconds(&self)` | `timeout` param | `timeout` param | `@Workflow(timeout=)` |
| `fn cancellable(&self)` | `cancellable` param | `cancellable` param | `@Workflow(cancellable=)` |

#### TaskDefinition

| Rust | Python | TypeScript | Java |
|------|--------|------------|------|
| `impl TaskDefinition` | `@task.defn` | `defineTask()` | `@Task` |
| `fn kind(&self) -> &str` | `name` param or func name | `name` param | `@Task(name=)` |
| `async fn execute(...)` | decorated function body | `execute: async (ctx, input) =>` | `execute()` method |
| `fn retry_config(&self)` | `retry` param | `retry` param | `@Task(retry=)` |
| `fn heartbeat_timeout_seconds(&self)` | `heartbeat_timeout` param | `heartbeatTimeout` param | `@Task(heartbeatTimeout=)` |

---

### Rust SDK (Reference Implementation)

```rust
use flovyn_sdk::prelude::*;
use std::time::Duration;

// ============================================================================
// Workflow Definition
// ============================================================================

#[derive(Default)]
struct OrderWorkflow;

#[async_trait]
impl WorkflowDefinition for OrderWorkflow {
    type Input = OrderInput;
    type Output = OrderResult;

    fn kind(&self) -> &str { "order-workflow" }

    fn version(&self) -> SemanticVersion { SemanticVersion::new(1, 0, 0) }

    fn timeout_seconds(&self) -> Option<u32> { Some(3600) }

    fn cancellable(&self) -> bool { true }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: OrderInput) -> Result<OrderResult> {
        // Deterministic time
        let start_time = ctx.current_time_millis();

        // Deterministic random
        let uuid = ctx.random_uuid();
        let random_number = ctx.random().next_int(0, 100);

        // Schedule typed task (parallel-ready)
        let task_future = ctx.schedule::<ProcessPaymentTask>(PaymentRequest {
            order_id: input.order_id.clone(),
            amount: input.amount,
        });

        // Sleep (durable timer)
        ctx.sleep(Duration::from_secs(30)).await?;

        // Await task result
        let payment_result = task_future.await?;

        // State management
        ctx.set("payment_status", &payment_result.status).await?;
        let status: Option<String> = ctx.get("payment_status").await?;

        // Child workflow
        let child_result = ctx.schedule_workflow::<ShippingWorkflow>(
            "shipping-1",
            ShippingInput { order_id: input.order_id.clone() }
        ).await?;

        // Promise (external signal)
        let approval: ApprovalDecision = ctx.promise("manager-approval").await?;

        // Cancellation check
        if ctx.is_cancellation_requested() {
            return Err(FlovynError::Cancelled);
        }

        Ok(OrderResult {
            order_id: input.order_id,
            status: "completed".to_string(),
        })
    }
}

// ============================================================================
// Task Definition
// ============================================================================

#[derive(Default)]
struct ProcessPaymentTask;

#[async_trait]
impl TaskDefinition for ProcessPaymentTask {
    type Input = PaymentRequest;
    type Output = PaymentResult;

    fn kind(&self) -> &str { "process-payment" }

    fn version(&self) -> SemanticVersion { SemanticVersion::new(1, 0, 0) }

    fn timeout_seconds(&self) -> Option<u32> { Some(60) }

    fn retry_config(&self) -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(30),
        }
    }

    fn heartbeat_timeout_seconds(&self) -> Option<u32> { Some(10) }

    async fn execute(&self, input: PaymentRequest, ctx: &dyn TaskContext) -> Result<PaymentResult> {
        // Report progress
        ctx.report_progress(0.0, "Starting payment").await?;

        // Check attempt number
        let attempt = ctx.attempt();

        // Heartbeat for long-running work
        ctx.heartbeat(json!({"step": "processing"}));

        // Check cancellation
        if ctx.is_cancelled() {
            return Err(FlovynError::Cancelled);
        }

        ctx.report_progress(1.0, "Payment complete").await?;

        Ok(PaymentResult {
            transaction_id: "txn-123".to_string(),
            status: "success".to_string(),
        })
    }
}

// ============================================================================
// Worker Setup
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let client = FlovynClient::builder()
        .server_url("http://localhost:9090")
        .tenant_id("550e8400-e29b-41d4-a716-446655440000".parse()?)
        .build()
        .await?;

    let worker = client.worker_builder("order-queue")
        .workflow::<OrderWorkflow>()
        .workflow::<ShippingWorkflow>()
        .task::<ProcessPaymentTask>()
        .task::<SendNotificationTask>()
        .max_concurrent_workflow_tasks(100)
        .max_concurrent_tasks(50)
        .build()
        .await?;

    worker.run().await
}
```

---

### Python SDK

```python
from flovyn import workflow, task, Client, Worker
from flovyn.workflow import WorkflowContext
from flovyn.task import TaskContext
from datetime import timedelta
from dataclasses import dataclass
from typing import Optional

# ============================================================================
# Data Types
# ============================================================================

@dataclass
class OrderInput:
    order_id: str
    amount: int

@dataclass
class OrderResult:
    order_id: str
    status: str

@dataclass
class PaymentRequest:
    order_id: str
    amount: int

@dataclass
class PaymentResult:
    transaction_id: str
    status: str

# ============================================================================
# Workflow Definition
# ============================================================================

@workflow.defn(
    name="order-workflow",          # Matches Rust: fn kind(&self)
    version="1.0.0",                # Matches Rust: fn version(&self)
    timeout=timedelta(hours=1),     # Matches Rust: fn timeout_seconds(&self)
    cancellable=True,               # Matches Rust: fn cancellable(&self)
)
class OrderWorkflow:

    @workflow.run
    async def execute(self, ctx: WorkflowContext, input: OrderInput) -> OrderResult:
        # Deterministic time - matches: ctx.current_time_millis()
        start_time = ctx.current_time_millis()

        # Deterministic random - matches: ctx.random_uuid(), ctx.random()
        uuid = ctx.random_uuid()
        random_number = ctx.random.next_int(0, 100)

        # Schedule typed task - matches: ctx.schedule::<T>(input)
        task_future = ctx.schedule(ProcessPaymentTask, PaymentRequest(
            order_id=input.order_id,
            amount=input.amount,
        ))

        # Sleep - matches: ctx.sleep(Duration::from_secs(30))
        await ctx.sleep(timedelta(seconds=30))

        # Await task result
        payment_result = await task_future

        # State management - matches: ctx.set/get
        await ctx.set("payment_status", payment_result.status)
        status: Optional[str] = await ctx.get("payment_status")

        # Child workflow - matches: ctx.schedule_workflow::<W>(name, input)
        child_result = await ctx.schedule_workflow(
            ShippingWorkflow,
            "shipping-1",
            ShippingInput(order_id=input.order_id),
        )

        # Promise - matches: ctx.promise::<T>(name)
        approval: ApprovalDecision = await ctx.promise("manager-approval")

        # Cancellation check - matches: ctx.is_cancellation_requested()
        if ctx.is_cancellation_requested():
            raise workflow.CancelledException()

        return OrderResult(
            order_id=input.order_id,
            status="completed",
        )

# ============================================================================
# Task Definition
# ============================================================================

@task.defn(
    name="process-payment",                         # Matches Rust: fn kind(&self)
    version="1.0.0",                                # Matches Rust: fn version(&self)
    timeout=timedelta(seconds=60),                  # Matches Rust: fn timeout_seconds(&self)
    retry=task.RetryConfig(                         # Matches Rust: fn retry_config(&self)
        max_retries=3,
        initial_backoff=timedelta(seconds=1),
        backoff_multiplier=2.0,
        max_backoff=timedelta(seconds=30),
    ),
    heartbeat_timeout=timedelta(seconds=10),        # Matches Rust: fn heartbeat_timeout_seconds(&self)
)
async def ProcessPaymentTask(ctx: TaskContext, input: PaymentRequest) -> PaymentResult:
    # Report progress - matches: ctx.report_progress(progress, msg).await
    await ctx.report_progress(0.0, "Starting payment")

    # Check attempt - matches: ctx.attempt()
    attempt = ctx.attempt

    # Heartbeat - matches: ctx.heartbeat(details)
    ctx.heartbeat({"step": "processing"})

    # Check cancellation - matches: ctx.is_cancelled()
    if ctx.is_cancelled():
        raise task.CancelledException()

    await ctx.report_progress(1.0, "Payment complete")

    return PaymentResult(
        transaction_id="txn-123",
        status="success",
    )

# ============================================================================
# Worker Setup
# ============================================================================

async def main():
    client = await Client.connect(
        server_url="http://localhost:9090",
        tenant_id="550e8400-e29b-41d4-a716-446655440000",
    )

    worker = Worker(
        client=client,
        task_queue="order-queue",
        workflows=[OrderWorkflow, ShippingWorkflow],
        tasks=[ProcessPaymentTask, SendNotificationTask],
        max_concurrent_workflow_tasks=100,
        max_concurrent_tasks=50,
    )

    await worker.run()

if __name__ == "__main__":
    import asyncio
    asyncio.run(main())
```

---

### TypeScript SDK

```typescript
import {
  defineWorkflow,
  defineTask,
  WorkflowContext,
  TaskContext,
  Client,
  Worker,
  RetryConfig,
  Duration,
} from '@flovyn/sdk';

// ============================================================================
// Data Types
// ============================================================================

interface OrderInput {
  orderId: string;
  amount: number;
}

interface OrderResult {
  orderId: string;
  status: string;
}

interface PaymentRequest {
  orderId: string;
  amount: number;
}

interface PaymentResult {
  transactionId: string;
  status: string;
}

// ============================================================================
// Workflow Definition
// ============================================================================

export const OrderWorkflow = defineWorkflow<OrderInput, OrderResult>({
  name: 'order-workflow',           // Matches Rust: fn kind(&self)
  version: '1.0.0',                 // Matches Rust: fn version(&self)
  timeout: { hours: 1 },            // Matches Rust: fn timeout_seconds(&self)
  cancellable: true,                // Matches Rust: fn cancellable(&self)

  async execute(ctx: WorkflowContext, input: OrderInput): Promise<OrderResult> {
    // Deterministic time - matches: ctx.current_time_millis()
    const startTime = ctx.currentTimeMillis();

    // Deterministic random - matches: ctx.random_uuid(), ctx.random()
    const uuid = ctx.randomUuid();
    const randomNumber = ctx.random.nextInt(0, 100);

    // Schedule typed task - matches: ctx.schedule::<T>(input)
    const taskFuture = ctx.schedule(ProcessPaymentTask, {
      orderId: input.orderId,
      amount: input.amount,
    });

    // Sleep - matches: ctx.sleep(Duration::from_secs(30))
    await ctx.sleep({ seconds: 30 });

    // Await task result
    const paymentResult = await taskFuture;

    // State management - matches: ctx.set/get
    await ctx.set('paymentStatus', paymentResult.status);
    const status = await ctx.get<string>('paymentStatus');

    // Child workflow - matches: ctx.schedule_workflow::<W>(name, input)
    const childResult = await ctx.scheduleWorkflow(
      ShippingWorkflow,
      'shipping-1',
      { orderId: input.orderId },
    );

    // Promise - matches: ctx.promise::<T>(name)
    const approval = await ctx.promise<ApprovalDecision>('manager-approval');

    // Cancellation check - matches: ctx.is_cancellation_requested()
    if (ctx.isCancellationRequested()) {
      throw new CancelledException();
    }

    return {
      orderId: input.orderId,
      status: 'completed',
    };
  },
});

// ============================================================================
// Task Definition
// ============================================================================

export const ProcessPaymentTask = defineTask<PaymentRequest, PaymentResult>({
  name: 'process-payment',                          // Matches Rust: fn kind(&self)
  version: '1.0.0',                                 // Matches Rust: fn version(&self)
  timeout: { seconds: 60 },                         // Matches Rust: fn timeout_seconds(&self)
  retry: {                                          // Matches Rust: fn retry_config(&self)
    maxRetries: 3,
    initialBackoff: { seconds: 1 },
    backoffMultiplier: 2.0,
    maxBackoff: { seconds: 30 },
  },
  heartbeatTimeout: { seconds: 10 },                // Matches Rust: fn heartbeat_timeout_seconds(&self)

  async execute(ctx: TaskContext, input: PaymentRequest): Promise<PaymentResult> {
    // Report progress - matches: ctx.report_progress(progress, msg).await
    await ctx.reportProgress(0.0, 'Starting payment');

    // Check attempt - matches: ctx.attempt()
    const attempt = ctx.attempt;

    // Heartbeat - matches: ctx.heartbeat(details)
    ctx.heartbeat({ step: 'processing' });

    // Check cancellation - matches: ctx.is_cancelled()
    if (ctx.isCancelled()) {
      throw new CancelledException();
    }

    await ctx.reportProgress(1.0, 'Payment complete');

    return {
      transactionId: 'txn-123',
      status: 'success',
    };
  },
});

// ============================================================================
// Worker Setup
// ============================================================================

async function main() {
  const client = await Client.connect({
    serverUrl: 'http://localhost:9090',
    tenantId: '550e8400-e29b-41d4-a716-446655440000',
  });

  const worker = new Worker({
    client,
    taskQueue: 'order-queue',
    workflows: [OrderWorkflow, ShippingWorkflow],
    tasks: [ProcessPaymentTask, SendNotificationTask],
    maxConcurrentWorkflowTasks: 100,
    maxConcurrentTasks: 50,
  });

  await worker.run();
}

main().catch(console.error);
```

---

### Java SDK (Synchronous Style - Recommended)

Java workflows can use a **synchronous-looking API** where blocking calls are intercepted and
replayed deterministically. This is the recommended style for Java as it's more natural and
matches Temporal's Java SDK approach.

```java
package com.example.orders;

import io.flovyn.sdk.*;
import io.flovyn.sdk.workflow.*;
import io.flovyn.sdk.task.*;

import java.time.Duration;
import java.util.UUID;

// ============================================================================
// Data Types
// ============================================================================

public record OrderInput(String orderId, int amount) {}
public record OrderResult(String orderId, String status) {}
public record PaymentRequest(String orderId, int amount) {}
public record PaymentResult(String transactionId, String status) {}

// ============================================================================
// Workflow Definition (Synchronous Style)
// ============================================================================

@Workflow(
    name = "order-workflow",
    version = "1.0.0",
    timeout = 3600,
    cancellable = true
)
public class OrderWorkflow {

    @WorkflowMethod
    public OrderResult execute(WorkflowContext ctx, OrderInput input) {
        // Deterministic time - matches: ctx.current_time_millis()
        long startTime = ctx.currentTimeMillis();

        // Deterministic random - matches: ctx.random_uuid(), ctx.random()
        UUID uuid = ctx.randomUuid();
        int randomNumber = ctx.getRandom().nextInt(0, 100);

        // Schedule task and get future (non-blocking)
        var paymentFuture = ctx.schedule(ProcessPaymentTask.class, new PaymentRequest(
            input.orderId(),
            input.amount()
        ));

        // Sleep - looks blocking but is actually durable timer
        ctx.sleep(Duration.ofSeconds(30));

        // Await task result - looks blocking but replays deterministically
        PaymentResult paymentResult = paymentFuture.get();

        // State management
        ctx.set("paymentStatus", paymentResult.status());
        String status = ctx.get("paymentStatus", String.class);

        // Child workflow - blocking call
        ShippingResult childResult = ctx.scheduleWorkflow(
            ShippingWorkflow.class,
            "shipping-1",
            new ShippingInput(input.orderId())
        ).get();

        // Promise - blocking wait for external signal
        ApprovalDecision approval = ctx.promise("manager-approval", ApprovalDecision.class).get();

        // Cancellation check
        if (ctx.isCancellationRequested()) {
            throw new CancelledException();
        }

        return new OrderResult(input.orderId(), "completed");
    }
}

// ============================================================================
// Parallel Execution (Synchronous Style)
// ============================================================================

@Workflow(name = "parallel-order-workflow")
public class ParallelOrderWorkflow {

    @WorkflowMethod
    public OrderResult execute(WorkflowContext ctx, OrderInput input) {
        // Schedule multiple tasks in parallel - returns immediately
        var paymentFuture = ctx.schedule(ProcessPaymentTask.class,
            new PaymentRequest(input.orderId(), input.amount()));
        var inventoryFuture = ctx.schedule(CheckInventoryTask.class,
            new InventoryRequest(input.orderId()));
        var fraudFuture = ctx.schedule(FraudCheckTask.class,
            new FraudCheckRequest(input.orderId()));

        // Wait for all to complete - Workflow.await handles replay correctly
        Workflow.await(() ->
            paymentFuture.isDone() &&
            inventoryFuture.isDone() &&
            fraudFuture.isDone()
        );

        // Or use allOf helper
        Workflow.allOf(paymentFuture, inventoryFuture, fraudFuture).get();

        // Get results
        PaymentResult payment = paymentFuture.get();
        InventoryResult inventory = inventoryFuture.get();
        FraudCheckResult fraud = fraudFuture.get();

        return new OrderResult(input.orderId(), "completed");
    }
}

// ============================================================================
// Task Definition (Always Synchronous)
// ============================================================================

@Task(
    name = "process-payment",
    version = "1.0.0",
    timeout = 60,
    retry = @RetryConfig(
        maxRetries = 3,
        initialBackoffSeconds = 1,
        backoffMultiplier = 2.0,
        maxBackoffSeconds = 30
    ),
    heartbeatTimeout = 10
)
public class ProcessPaymentTask implements TaskDefinition<PaymentRequest, PaymentResult> {

    @Override
    public PaymentResult execute(TaskContext ctx, PaymentRequest input) throws Exception {
        // Tasks are always synchronous - real I/O happens here
        ctx.reportProgress(0.0, "Starting payment");

        int attempt = ctx.getAttempt();

        // Long-running work with heartbeat
        for (int i = 0; i < 10; i++) {
            ctx.heartbeat(Map.of("step", i));

            if (ctx.isCancelled()) {
                throw new CancelledException();
            }

            // Simulate work
            Thread.sleep(100);
        }

        ctx.reportProgress(1.0, "Payment complete");

        return new PaymentResult("txn-123", "success");
    }
}

// ============================================================================
// Worker Setup
// ============================================================================

public class Main {
    public static void main(String[] args) throws Exception {
        var client = Client.connect(ClientOptions.builder()
            .serverUrl("http://localhost:9090")
            .tenantId(UUID.fromString("550e8400-e29b-41d4-a716-446655440000"))
            .build());

        var worker = Worker.builder()
            .client(client)
            .taskQueue("order-queue")
            .workflow(OrderWorkflow.class)
            .workflow(ParallelOrderWorkflow.class)
            .task(ProcessPaymentTask.class)
            .maxConcurrentWorkflowTasks(100)
            .maxConcurrentTasks(50)
            .build();

        worker.run();  // Blocks until shutdown
    }
}
```

---

### Java SDK (Async Style with CompletableFuture)

For developers who prefer async/reactive patterns, a CompletableFuture-based API is also available:

```java
package com.example.orders;

import io.flovyn.sdk.*;
import io.flovyn.sdk.workflow.*;
import io.flovyn.sdk.task.*;

import java.time.Duration;
import java.util.concurrent.CompletableFuture;

// ============================================================================
// Data Types
// ============================================================================

public record OrderInput(String orderId, int amount) {}
public record OrderResult(String orderId, String status) {}
public record PaymentRequest(String orderId, int amount) {}
public record PaymentResult(String transactionId, String status) {}

// ============================================================================
// Workflow Definition
// ============================================================================

@Workflow(
    name = "order-workflow",            // Matches Rust: fn kind(&self)
    version = "1.0.0",                  // Matches Rust: fn version(&self)
    timeout = 3600,                     // Matches Rust: fn timeout_seconds(&self)
    cancellable = true                  // Matches Rust: fn cancellable(&self)
)
public class OrderWorkflow {

    @WorkflowMethod
    public CompletableFuture<OrderResult> execute(WorkflowContext ctx, OrderInput input) {
        // Deterministic time - matches: ctx.current_time_millis()
        long startTime = ctx.currentTimeMillis();

        // Deterministic random - matches: ctx.random_uuid(), ctx.random()
        UUID uuid = ctx.randomUuid();
        int randomNumber = ctx.getRandom().nextInt(0, 100);

        // Schedule typed task - matches: ctx.schedule::<T>(input)
        var taskFuture = ctx.schedule(ProcessPaymentTask.class, new PaymentRequest(
            input.orderId(),
            input.amount()
        ));

        // Sleep - matches: ctx.sleep(Duration::from_secs(30))
        return ctx.sleep(Duration.ofSeconds(30))
            .thenCompose(v -> taskFuture)
            .thenCompose(paymentResult -> {
                // State management - matches: ctx.set/get
                return ctx.set("paymentStatus", paymentResult.status())
                    .thenCompose(v -> ctx.get("paymentStatus", String.class));
            })
            .thenCompose(status -> {
                // Child workflow - matches: ctx.schedule_workflow::<W>(name, input)
                return ctx.scheduleWorkflow(
                    ShippingWorkflow.class,
                    "shipping-1",
                    new ShippingInput(input.orderId())
                );
            })
            .thenCompose(childResult -> {
                // Promise - matches: ctx.promise::<T>(name)
                return ctx.promise("manager-approval", ApprovalDecision.class);
            })
            .thenApply(approval -> {
                // Cancellation check - matches: ctx.is_cancellation_requested()
                if (ctx.isCancellationRequested()) {
                    throw new CancelledException();
                }

                return new OrderResult(input.orderId(), "completed");
            });
    }
}

// ============================================================================
// Task Definition
// ============================================================================

@Task(
    name = "process-payment",                           // Matches Rust: fn kind(&self)
    version = "1.0.0",                                  // Matches Rust: fn version(&self)
    timeout = 60,                                       // Matches Rust: fn timeout_seconds(&self)
    retry = @RetryConfig(                               // Matches Rust: fn retry_config(&self)
        maxRetries = 3,
        initialBackoffSeconds = 1,
        backoffMultiplier = 2.0,
        maxBackoffSeconds = 30
    ),
    heartbeatTimeout = 10                               // Matches Rust: fn heartbeat_timeout_seconds(&self)
)
public class ProcessPaymentTask implements TaskDefinition<PaymentRequest, PaymentResult> {

    @Override
    public PaymentResult execute(TaskContext ctx, PaymentRequest input) throws Exception {
        // Report progress - matches: ctx.report_progress(progress, msg).await
        ctx.reportProgress(0.0, "Starting payment");

        // Check attempt - matches: ctx.attempt()
        int attempt = ctx.getAttempt();

        // Heartbeat - matches: ctx.heartbeat(details)
        ctx.heartbeat(Map.of("step", "processing"));

        // Check cancellation - matches: ctx.is_cancelled()
        if (ctx.isCancelled()) {
            throw new CancelledException();
        }

        ctx.reportProgress(1.0, "Payment complete");

        return new PaymentResult("txn-123", "success");
    }
}

// ============================================================================
// Worker Setup
// ============================================================================

public class Main {
    public static void main(String[] args) throws Exception {
        var client = Client.connect(ClientOptions.builder()
            .serverUrl("http://localhost:9090")
            .tenantId(UUID.fromString("550e8400-e29b-41d4-a716-446655440000"))
            .build());

        var worker = Worker.builder()
            .client(client)
            .taskQueue("order-queue")
            .workflow(OrderWorkflow.class)
            .workflow(ShippingWorkflow.class)
            .task(ProcessPaymentTask.class)
            .task(SendNotificationTask.class)
            .maxConcurrentWorkflowTasks(100)
            .maxConcurrentTasks(50)
            .build();

        worker.run().join();
    }
}
```

---

### API Alignment Summary

| Concept | Rust | Python | TypeScript | Java (Sync) | Java (Async) |
|---------|------|--------|------------|-------------|--------------|
| **Workflow decorator** | `impl WorkflowDefinition` | `@workflow.defn` | `defineWorkflow({})` | `@Workflow` | `@Workflow` |
| **Task decorator** | `impl TaskDefinition` | `@task.defn` | `defineTask({})` | `@Task` | `@Task` |
| **Run method** | `async fn execute` | `@workflow.run` | `execute:` | `execute()` | `execute()` → `CompletableFuture` |
| **Schedule task** | `ctx.schedule::<T>()` | `ctx.schedule(T, input)` | `ctx.schedule(T, input)` | `ctx.schedule().get()` | `ctx.schedule().thenCompose()` |
| **Sleep** | `ctx.sleep(Duration)` | `await ctx.sleep(timedelta)` | `await ctx.sleep({})` | `ctx.sleep(Duration)` | `ctx.sleep().thenCompose()` |
| **Promise** | `ctx.promise::<T>()` | `ctx.promise()` | `ctx.promise<T>()` | `ctx.promise().get()` | `ctx.promise().thenCompose()` |
| **Child workflow** | `ctx.schedule_workflow::<W>()` | `ctx.schedule_workflow()` | `ctx.scheduleWorkflow()` | `ctx.scheduleWorkflow().get()` | `ctx.scheduleWorkflow().thenCompose()` |
| **Parallel tasks** | `join_all(futures)` | `asyncio.gather()` | `Promise.all()` | `Workflow.allOf().get()` | `CompletableFuture.allOf()` |
| **Current time** | `ctx.current_time_millis()` | `ctx.current_time_millis()` | `ctx.currentTimeMillis()` | `ctx.currentTimeMillis()` | `ctx.currentTimeMillis()` |
| **Random UUID** | `ctx.random_uuid()` | `ctx.random_uuid()` | `ctx.randomUuid()` | `ctx.randomUuid()` | `ctx.randomUuid()` |
| **State get** | `ctx.get::<T>().await` | `await ctx.get()` | `await ctx.get<T>()` | `ctx.get(key, T.class)` | `ctx.get().thenCompose()` |
| **State set** | `ctx.set().await` | `await ctx.set()` | `await ctx.set()` | `ctx.set(key, value)` | `ctx.set().thenCompose()` |
| **Heartbeat** | `ctx.heartbeat()` | `ctx.heartbeat()` | `ctx.heartbeat()` | `ctx.heartbeat()` | `ctx.heartbeat()` |
| **Progress** | `ctx.report_progress()` | `ctx.report_progress()` | `ctx.reportProgress()` | `ctx.reportProgress()` | `ctx.reportProgress()` |
| **Cancellation** | `ctx.is_cancellation_requested()` | `ctx.is_cancellation_requested()` | `ctx.isCancellationRequested()` | `ctx.isCancellationRequested()` | `ctx.isCancellationRequested()` |

### Key Design Principles

1. **Same method names** (adjusted for language conventions):
   - Rust: `snake_case`
   - Python: `snake_case`
   - TypeScript: `camelCase`
   - Java: `camelCase` with `get` prefix for getters

2. **Same parameter order**: `(context, input)` for workflows, `(input, context)` or `(context, input)` for tasks

3. **Same return types**: Futures/Promises that resolve to the output type

4. **Same configuration options**: All languages support the same workflow/task metadata (name, version, timeout, retry, etc.)

5. **Type-safe scheduling**: All languages support both typed (`schedule::<T>`) and raw (`schedule_raw`) variants

6. **Parallel execution**: All languages return futures immediately, enabling `join_all`/`Promise.all`/`CompletableFuture.allOf` patterns

7. **Java dual-style support**:
   - **Sync style (recommended)**: Blocking-looking calls that are replayed deterministically. More natural for Java developers, similar to Temporal's Java SDK.
   - **Async style**: `CompletableFuture`-based for developers who prefer reactive patterns.
   - Both styles use the same underlying mechanism - the sync style is just syntactic sugar over the async primitives.

## References

- [Temporal SDK Core](https://github.com/temporalio/sdk-core)
- [Temporal Python SDK](https://github.com/temporalio/sdk-python)
- [PyO3 Documentation](https://pyo3.rs/)
- [napi-rs Documentation](https://napi.rs/)
- [uniffi Documentation](https://mozilla.github.io/uniffi-rs/)
- [Temporal: Why Rust Powers Core SDK](https://temporal.io/blog/why-rust-powers-core-sdk)
