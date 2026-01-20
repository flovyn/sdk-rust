# Python SDK Design

## Overview

Build a Pythonic, fully-typed SDK for Flovyn workflow orchestration in `../sdk-python`. The SDK uses a Rust core via UniFFI bindings (same approach as `sdk-kotlin`) while providing an idiomatic Python API.

**Goals:**
- **Pythonic API** - Decorators, snake_case, context managers, async/await
- **Fully Typed** - Complete type annotations, generics, PEP 561 compliant (`py.typed`)
- **Type-Safe at Runtime** - Pydantic models for validation and serialization
- **Full Feature Parity** - Match Rust and Kotlin SDK capabilities
- **E2E Tested** - Testcontainers-based integration tests

**Non-Goals:**
- Workflow sandbox/isolation (unlike Temporal, we rely on context-based determinism enforcement)

## Architecture Comparison

### Industry Approaches

| Aspect | Temporal | Restate | Hatchet | Flovyn |
|--------|----------|---------|---------|--------|
| **Core Language** | Rust (sdk-core) | Rust | Go | Rust (worker-ffi) |
| **Python FFI** | PyO3 | Pure Python (HTTP) | Pure Python (gRPC) | UniFFI |
| **Definition Style** | Class + decorators | Builder + decorators | Instance + decorators | Class/function + decorators |
| **Task Concept** | Activities | Handlers | Tasks with DAG | Tasks |
| **Determinism** | Sandbox + context | Context-only | Context + conditions | Context-only |
| **Serialization** | Custom converters | Auto-detect (Pydantic/msgspec) | Pydantic-first | Pydantic-first |
| **Concurrency** | Resource-based | Per-key isolation | CEL expressions | Slot-based |

**Temporal** - Most mature, complex sandbox for determinism, string-based activity references

**Restate** - Elegant context API, type-safe handler references, multi-tier serde, no native dependencies

**Hatchet** - DAG-first design, rich conditions (wait_for/skip_if/cancel_if), CEL expressions, Pydantic-native

### Best Patterns to Adopt

From **Restate**:
- Type-safe handler invocation via function references (not strings)
- `select()` pattern for concurrent operations with match/case
- Multi-tier serde auto-detection (Pydantic → msgspec → dataclass → JSON)
- `ctx.run()` for durable side effects with automatic caching

From **Hatchet**:
- DAG-based task dependencies via `parents=[]` parameter
- `ctx.task_output(parent_task)` for type-safe parent output access
- Rich workflow options (sticky workers, cron triggers, event triggers)
- Pydantic `input_validator` pattern on workflow/task decorators

From **Temporal**:
- Activation-based execution model (poll-complete cycle)
- Rust core for determinism and replay (via UniFFI, not PyO3)
- Worker registration with explicit workflow/task lists

### Why UniFFI over PyO3?

| Aspect | UniFFI | PyO3 |
|--------|--------|------|
| Multi-language | Single UDL → Kotlin, Swift, Python | Python-only |
| Consistency | Same FFI layer as Kotlin SDK | Separate Python-specific bridge |
| Maintenance | One codebase for all language bindings | Per-language maintenance |
| Learning curve | Declarative UDL | Rust macros + Python knowledge |

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  User Code (Python)                  │
│   @workflow, @task decorators, async/await          │
├─────────────────────────────────────────────────────┤
│                flovyn-sdk (Python)                   │
│   WorkflowContext, TaskContext, FlovynClient        │
├─────────────────────────────────────────────────────┤
│              flovyn-native (Python)                  │
│   UniFFI-generated bindings, native library loader  │
├─────────────────────────────────────────────────────┤
│            libflovyn_worker_ffi (Rust)              │
│   CoreWorker, CoreClient, FfiWorkflowContext        │
└─────────────────────────────────────────────────────┘
```

### Package Structure

```
sdk-python/
├── flovyn/
│   ├── __init__.py           # Public API exports
│   ├── workflow.py           # @workflow decorator, WorkflowContext
│   ├── task.py               # @task decorator, TaskContext
│   ├── client.py             # FlovynClient, FlovynClientBuilder
│   ├── worker.py             # WorkflowWorker, TaskWorker (internal)
│   ├── context.py            # Context implementations
│   ├── exceptions.py         # FlovynError, WorkflowSuspended, etc.
│   ├── types.py              # Common types, protocols
│   └── _native/
│       ├── __init__.py
│       ├── loader.py         # Native library extraction/loading
│       └── flovyn_worker_ffi.py  # UniFFI-generated bindings
├── tests/
│   ├── unit/
│   ├── e2e/
│   └── conftest.py
├── examples/
│   ├── hello_world.py
│   ├── order_processing.py
│   └── data_pipeline.py
├── pyproject.toml
└── README.md
```

## Public API Design

### Workflow Definition

Use class-based decorators with explicit type parameters for full type safety:

```python
from flovyn import workflow, WorkflowContext
from pydantic import BaseModel

class OrderInput(BaseModel):
    order_id: str
    items: list[str]
    total: float

class OrderResult(BaseModel):
    confirmation_id: str
    status: str

@workflow(name="order-processing")
class OrderWorkflow:
    """Process an order through validation, payment, and fulfillment."""

    async def run(self, ctx: WorkflowContext, input: OrderInput) -> OrderResult:
        # Validate order (string-based task reference for distributed systems)
        validation: ValidationResult = await ctx.execute_task(
            "validate-order",
            input.model_dump(),
        )

        # Process payment
        payment: PaymentResult = await ctx.execute_task(
            "process-payment",
            {"order_id": input.order_id, "amount": input.total},
        )

        # Fulfill order
        fulfillment: FulfillmentResult = await ctx.execute_task(
            "fulfill-order",
            {"order_id": input.order_id, "items": input.items},
        )

        return OrderResult(
            confirmation_id=payment.transaction_id,
            status="completed",
        )
```

**Decorator Options:**

```python
@workflow(
    name="my-workflow",           # Unique identifier (required)
    version="1.0.0",              # Optional version
    description="...",            # Optional description
    tags=["production", "v1"],    # Optional tags
    timeout=timedelta(hours=24),  # Optional workflow timeout
)
class MyWorkflow:
    async def run(self, ctx: WorkflowContext, input: MyInput) -> MyOutput:
        ...
```

### Task Definition

```python
from flovyn import task, TaskContext
from pydantic import BaseModel

class EmailRequest(BaseModel):
    to: str
    subject: str
    body: str

class EmailResult(BaseModel):
    message_id: str
    sent_at: datetime

@task(name="send-email")
class SendEmail:
    """Send an email notification."""

    async def run(self, ctx: TaskContext, input: EmailRequest) -> EmailResult:
        # Report progress
        await ctx.report_progress(0.1, "Connecting to SMTP server")

        # Check cancellation
        if ctx.is_cancelled:
            raise ctx.cancellation_error()

        # Actual email sending logic
        result = await send_email_impl(input)

        await ctx.report_progress(1.0, "Email sent successfully")

        return EmailResult(
            message_id=result.id,
            sent_at=datetime.now(UTC),
        )
```

**Decorator Options:**

```python
@task(
    name="my-task",                       # Unique identifier (required)
    timeout=timedelta(minutes=5),         # Execution timeout
    retry_policy=RetryPolicy(             # Retry configuration
        max_attempts=3,
        initial_interval=timedelta(seconds=1),
        max_interval=timedelta(minutes=1),
        backoff_coefficient=2.0,
    ),
    cancellable=True,                     # Allow cancellation
)
class MyTask:
    async def run(self, ctx: TaskContext, input: MyInput) -> MyOutput:
        ...
```

### Alternative: Function-Based Definition

For simpler workflows/tasks, support function decorators:

```python
from flovyn import workflow, task, WorkflowContext, TaskContext

@workflow(name="simple-workflow")
async def simple_workflow(ctx: WorkflowContext, name: str) -> str:
    greeting: str = await ctx.execute_task("greet", name)
    return f"Workflow completed: {greeting}"

@task(name="greet")
async def greet_task(ctx: TaskContext, name: str) -> str:
    return f"Hello, {name}!"
```

### WorkflowContext API

The context provides deterministic operations for workflow execution:

```python
class WorkflowContext(Protocol):
    """Deterministic execution context for workflows."""

    # Deterministic time and randomness
    def current_time(self) -> datetime:
        """Get current time (deterministic on replay)."""

    def current_time_millis(self) -> int:
        """Get current time as milliseconds since epoch."""

    def random_uuid(self) -> UUID:
        """Generate a deterministic UUID."""

    def random(self) -> Random:
        """Get a deterministic random number generator."""

    # Task execution (supports both string-based and typed APIs)
    async def execute_task(
        self,
        task: str | type[Task] | Callable,  # String or class reference
        input: Any,
        *,
        timeout: timedelta | None = None,
        retry_policy: RetryPolicy | None = None,
    ) -> Any:
        """Execute a task and await its result.

        Supports both APIs:
        - String-based (distributed): execute_task("add-task", {"a": 1, "b": 2})
        - Typed (single-server): execute_task(AddTask, AddInput(a=1, b=2))
        """

    def schedule_task(
        self,
        task: str | type[Task] | Callable,  # String or class reference
        input: Any,
        *,
        timeout: timedelta | None = None,
    ) -> TaskHandle[Any]:
        """Schedule a task for execution, returns immediately.

        Supports both APIs:
        - String-based (distributed): schedule_task("add-task", {"a": 1, "b": 2})
        - Typed (single-server): schedule_task(AddTask, AddInput(a=1, b=2))
        """

    # Child workflows (supports both string-based and typed APIs)
    async def execute_workflow(
        self,
        workflow: str | type[Workflow] | Callable,  # String or class reference
        input: Any,
        *,
        workflow_id: str | None = None,
        timeout: timedelta | None = None,
    ) -> Any:
        """Execute a child workflow and await its result.

        Supports both APIs:
        - String-based (distributed): execute_workflow("order-workflow", {...})
        - Typed (single-server): execute_workflow(OrderWorkflow, OrderInput(...))
        """

    def schedule_workflow(
        self,
        workflow: str | type[Workflow] | Callable,  # String or class reference
        input: Any,
        *,
        workflow_id: str | None = None,
    ) -> WorkflowHandle[Any]:
        """Schedule a child workflow, returns immediately.

        Supports both APIs:
        - String-based (distributed): schedule_workflow("order-workflow", {...})
        - Typed (single-server): schedule_workflow(OrderWorkflow, OrderInput(...))
        """

    # Timers
    async def sleep(self, duration: timedelta) -> None:
        """Sleep for a duration (durable across replays)."""

    async def sleep_until(self, until: datetime) -> None:
        """Sleep until a specific time."""

    # Promises (external completion)
    async def wait_for_promise[T](
        self,
        name: str,
        *,
        timeout: timedelta | None = None,
        type_hint: type[T] = Any,
    ) -> T:
        """Wait for an external promise to be resolved."""

    # Signals
    async def wait_for_signal[T](
        self,
        name: str,
        *,
        timeout: timedelta | None = None,
        type_hint: type[T] = Any,
    ) -> T:
        """Wait for an external signal."""

    # State management
    async def get_state[T](self, key: str, *, type_hint: type[T] = Any, default: T | None = None) -> T | None:
        """Get workflow state by key."""

    async def set_state(self, key: str, value: Any) -> None:
        """Set workflow state."""

    async def clear_state(self, key: str) -> None:
        """Clear a state key."""

    # Side effects (cached on replay)
    async def run[T](self, name: str, fn: Callable[[], T | Awaitable[T]]) -> T:
        """Execute a side effect (result cached on replay)."""

    # Cancellation
    @property
    def is_cancellation_requested(self) -> bool:
        """Check if cancellation has been requested."""

    def check_cancellation(self) -> None:
        """Raise WorkflowCancelled if cancellation requested."""

    # Logging
    @property
    def logger(self) -> Logger:
        """Get a workflow-aware logger."""
```

### TaskContext API

```python
class TaskContext(Protocol):
    """Execution context for tasks."""

    @property
    def task_execution_id(self) -> str:
        """Unique ID for this task execution."""

    @property
    def attempt(self) -> int:
        """Current retry attempt number (1-based)."""

    @property
    def is_cancelled(self) -> bool:
        """Check if task cancellation has been requested."""

    def cancellation_error(self) -> TaskCancelled:
        """Create a cancellation error to raise."""

    async def report_progress(self, progress: float, message: str | None = None) -> None:
        """Report task progress (0.0 to 1.0)."""

    async def heartbeat(self) -> None:
        """Send a heartbeat to indicate the task is still running."""

    @property
    def logger(self) -> Logger:
        """Get a task-aware logger."""
```

### FlovynClient API

```python
from flovyn import FlovynClient

# Builder pattern for configuration
client = (
    FlovynClient.builder()
    .server_address("localhost", 9090)
    .org_id("my-org")
    .queue("default")
    .worker_token("secret-token")
    .register_workflow(OrderWorkflow)
    .register_workflow(simple_workflow)
    .register_task(SendEmail)
    .register_task(greet_task)
    .add_hook(MyWorkflowHook())
    .build()
)

# Start the worker (blocking)
await client.start()

# Or use context manager
async with client:
    # Worker runs until context exits
    await asyncio.sleep(3600)

# Start a workflow (supports both string-based and typed APIs)
# String-based API (for distributed systems)
handle = await client.start_workflow(
    "order-processing",  # workflow kind (string)
    {"order_id": "123", "items": ["item1"], "total": 99.99},  # input as dict
    workflow_id="order-123",  # Optional custom ID
)

# Typed API (for single-server deployments)
handle = await client.start_workflow(
    OrderWorkflow,  # workflow class
    {"order_id": "123", "items": ["item1"], "total": 99.99},
    workflow_id="order-123",
)

# Wait for result
result = await handle.result()

# Query workflow state
state = await handle.query("get_status")

# Signal workflow
await handle.signal("cancel_order", {"reason": "customer request"})

# Resolve external promise
await client.resolve_promise(
    workflow_id="order-123",
    promise_name="payment_confirmed",
    value={"transaction_id": "txn-456"},
)
```

### Hooks

```python
from flovyn import WorkflowHook, WorkflowStartedEvent, WorkflowCompletedEvent, WorkflowFailedEvent

class MetricsHook(WorkflowHook):
    async def on_workflow_started(self, event: WorkflowStartedEvent) -> None:
        metrics.increment("workflow.started", tags={"kind": event.workflow_kind})

    async def on_workflow_completed(self, event: WorkflowCompletedEvent) -> None:
        metrics.increment("workflow.completed", tags={"kind": event.workflow_kind})
        metrics.timing("workflow.duration", event.duration_ms)

    async def on_workflow_failed(self, event: WorkflowFailedEvent) -> None:
        metrics.increment("workflow.failed", tags={"kind": event.workflow_kind})
        logger.error(f"Workflow failed: {event.error}")
```

### Parallel Execution

```python
@workflow(name="parallel-tasks")
class ParallelWorkflow:
    async def run(self, ctx: WorkflowContext, items: list[str]) -> list[str]:
        # Schedule all tasks concurrently (string-based task reference)
        handles = [ctx.schedule_task("process-item", item) for item in items]

        # Await all results
        results = await asyncio.gather(*[h.result() for h in handles])

        return results
```

### DAG-Based Task Dependencies (Inspired by Hatchet)

Define task dependencies declaratively using the `parents` parameter:

```python
from flovyn import workflow, task, WorkflowContext, TaskContext
from pydantic import BaseModel

class OrderInput(BaseModel):
    order_id: str
    items: list[str]

class ValidationResult(BaseModel):
    valid: bool
    issues: list[str] = []

class InventoryResult(BaseModel):
    reserved: bool
    reservation_id: str

class PaymentResult(BaseModel):
    transaction_id: str

@workflow(name="order-processing")
class OrderWorkflow:
    pass

@task(name="validate-order")
class ValidateOrder:
    async def run(self, ctx: TaskContext, input: OrderInput) -> ValidationResult:
        # Validation logic
        return ValidationResult(valid=True)

@task(name="reserve-inventory")
class ReserveInventory:
    async def run(self, ctx: TaskContext, input: OrderInput) -> InventoryResult:
        return InventoryResult(reserved=True, reservation_id="inv-123")

# Task with multiple parents - runs after both complete
@task(name="process-payment", parents=[ValidateOrder, ReserveInventory])
class ProcessPayment:
    async def run(self, ctx: TaskContext, input: OrderInput) -> PaymentResult:
        # Access parent outputs (type-safe)
        validation = ctx.task_output(ValidateOrder)  # ValidationResult
        inventory = ctx.task_output(ReserveInventory)  # InventoryResult

        if not validation.valid:
            raise TaskFailed("Validation failed")

        return PaymentResult(transaction_id="txn-456")
```

### Dual API: String-Based vs Typed

The SDK supports both string-based and typed APIs for workflow/task invocation:

**String-Based API (Distributed Systems)**

Use string-based references when the client and worker may be on different machines:

```python
@workflow(name="order")
class OrderWorkflow:
    async def run(self, ctx: WorkflowContext, input: OrderInput) -> OrderResult:
        # String-based: task kind as string, works across distributed workers
        validation: ValidationResult = await ctx.execute_task(
            "validate-order",
            input.model_dump(),
        )

        # Caller must know the expected return type
        greeting: str = await ctx.execute_task("greet", "Alice")

        return OrderResult(...)
```

**Note**: String-based invocation is required for distributed systems where the workflow
executor may not have the task class available. The caller is responsible for knowing
the expected input/output types.

**Typed API (Single-Server Deployments)**

Use typed references when the client and worker are on the same machine:

```python
from myapp.tasks import ValidateOrder, GreetTask

@workflow(name="order")
class OrderWorkflow:
    async def run(self, ctx: WorkflowContext, input: OrderInput) -> OrderResult:
        # Typed: pass class reference, enables IDE support and type checking
        validation = await ctx.execute_task(
            ValidateOrder,  # Class reference
            input.model_dump(),
        )

        greeting = await ctx.execute_task(GreetTask, "Alice")

        return OrderResult(...)
```

**When to Use Each API:**

| Scenario | API | Reason |
|----------|-----|--------|
| Distributed system (multiple machines) | String-based | Worker may not have task class available |
| Microservices architecture | String-based | Services deploy independently |
| Single server (all-in-one) | Typed | Better IDE support, type checking |
| Development/testing | Typed | Easier refactoring, catch errors early |
| Dynamic task routing | String-based | Task kind determined at runtime |

### Durable Side Effects with ctx.run() (Inspired by Restate)

Cache non-deterministic operations for replay safety:

```python
@workflow(name="order-with-side-effects")
class OrderWorkflow:
    async def run(self, ctx: WorkflowContext, input: OrderInput) -> OrderResult:
        # Durable side effect - result cached on first execution, replayed after
        api_response = await ctx.run(
            "fetch-exchange-rate",
            lambda: fetch_exchange_rate(input.currency),
        )

        # Supports async functions too
        user_data = await ctx.run(
            "fetch-user-data",
            lambda: fetch_user_async(input.user_id),
        )

        # With retry options
        external_result = await ctx.run(
            "call-external-api",
            lambda: call_flaky_api(input.data),
            max_attempts=3,
            retry_interval=timedelta(seconds=1),
        )

        return OrderResult(rate=api_response.rate)
```

### Concurrent Operations with select() (Inspired by Restate)

Use Python 3.10+ match/case for elegant concurrent handling:

```python
from flovyn import select

@workflow(name="payment-with-timeout")
class PaymentWorkflow:
    async def run(self, ctx: WorkflowContext, input: PaymentInput) -> PaymentResult:
        # Race between payment confirmation and timeout
        match await select(
            confirmation=ctx.wait_for_promise("payment-confirmed"),
            timeout=ctx.sleep(timedelta(minutes=5)),
        ):
            case ("confirmation", result):
                return PaymentResult(status="confirmed", data=result)
            case ("timeout", _):
                await ctx.execute_task("refund", input.model_dump())
                raise WorkflowCancelled("Payment timed out")

        # Race multiple tasks
        match await select(
            fast=ctx.execute_task("fast-provider", input.model_dump()),
            slow=ctx.execute_task("slow-provider", input.model_dump()),
        ):
            case ("fast", result):
                return result
            case ("slow", result):
                return result
```

### Workflow Triggers (Inspired by Hatchet)

Configure event and cron triggers declaratively:

```python
@workflow(
    name="daily-report",
    on_cron="0 9 * * *",  # Daily at 9 AM
)
class DailyReportWorkflow:
    async def run(self, ctx: WorkflowContext, input: ReportInput) -> ReportResult:
        ...

@workflow(
    name="order-event-handler",
    on_events=["order.created", "order.updated"],
)
class OrderEventWorkflow:
    async def run(self, ctx: WorkflowContext, input: OrderEvent) -> None:
        event_type = ctx.trigger_event.type  # "order.created" or "order.updated"
        ...
```

### Conditional Execution (Inspired by Hatchet)

Skip or wait based on conditions:

```python
from flovyn import task, skip_if, wait_for, SleepCondition, ParentCondition

@task(
    name="send-notification",
    # Skip if parent returned should_notify=False
    skip_if=ParentCondition(
        parent=CheckPreferences,
        expression="output.should_notify == false",
    ),
)
class SendNotification:
    async def run(self, ctx: TaskContext, input: NotificationInput) -> None:
        ...

@task(
    name="delayed-followup",
    parents=[InitialContact],
    # Wait 24 hours after parent completes
    wait_for=SleepCondition(timedelta(hours=24)),
)
class DelayedFollowup:
    async def run(self, ctx: TaskContext, input: FollowupInput) -> None:
        ...
```

### Dynamic Workflows (Untyped)

For cases requiring runtime flexibility:

```python
from flovyn import dynamic_workflow, dynamic_task

@dynamic_workflow(name="dynamic-processor")
async def dynamic_processor(ctx: WorkflowContext, input: dict[str, Any]) -> dict[str, Any]:
    task_kind = input.get("task_kind", "default-task")
    result = await ctx.execute_task_by_name(task_kind, input.get("payload", {}))
    return {"result": result}

@dynamic_task(name="dynamic-task")
async def dynamic_task(ctx: TaskContext, input: dict[str, Any]) -> dict[str, Any]:
    return {"processed": True, "input": input}
```

## Exceptions

```python
# Base exception
class FlovynError(Exception):
    """Base exception for all Flovyn errors."""

# Workflow exceptions
class WorkflowSuspended(FlovynError):
    """Raised internally when workflow needs to suspend for pending work."""

class WorkflowCancelled(FlovynError):
    """Raised when workflow cancellation is requested."""

class DeterminismViolation(FlovynError):
    """Raised when workflow code violates determinism during replay."""

# Task exceptions
class TaskFailed(FlovynError):
    """Raised when a task fails."""

class TaskCancelled(FlovynError):
    """Raised when a task is cancelled."""

class TaskTimeout(FlovynError):
    """Raised when a task exceeds its timeout."""

# Child workflow exceptions
class ChildWorkflowFailed(FlovynError):
    """Raised when a child workflow fails."""

# Promise exceptions
class PromiseTimeout(FlovynError):
    """Raised when a promise times out."""

class PromiseRejected(FlovynError):
    """Raised when a promise is rejected."""
```

## Type System Design

The SDK is **fully typed** and designed for excellent IDE support and static analysis.

### PEP 561 Compliance

```
flovyn/
├── py.typed              # Marker file for type checkers
├── __init__.py
├── __init__.pyi          # Type stubs (if needed for complex overloads)
├── workflow.py
├── task.py
└── ...
```

### Generic Types

```python
from typing import Generic, TypeVar, Protocol, ParamSpec, Concatenate

InputT = TypeVar("InputT", contravariant=True)
OutputT = TypeVar("OutputT", covariant=True)
P = ParamSpec("P")

class Workflow(Protocol[InputT, OutputT]):
    """Protocol for typed workflow definitions."""
    async def run(self, ctx: WorkflowContext, input: InputT) -> OutputT: ...

class Task(Protocol[InputT, OutputT]):
    """Protocol for typed task definitions."""
    async def run(self, ctx: TaskContext, input: InputT) -> OutputT: ...

class TaskHandle(Generic[OutputT]):
    """Handle to a scheduled task with typed result."""
    async def result(self) -> OutputT: ...
    @property
    def task_execution_id(self) -> str: ...

class WorkflowHandle(Generic[OutputT]):
    """Handle to a running workflow with typed result."""
    @property
    def workflow_id(self) -> str: ...
    async def result(self, *, timeout: timedelta | None = None) -> OutputT: ...
    async def query(self, query_name: str, args: Any = None) -> Any: ...
    async def signal(self, signal_name: str, payload: Any = None) -> None: ...
    async def cancel(self) -> None: ...
```

### Type-Safe Decorators

```python
from typing import Callable, overload

# Overloaded decorator for both class and function forms
@overload
def workflow(
    *,
    name: str,
    version: str | None = None,
    timeout: timedelta | None = None,
) -> Callable[[type[Workflow[InputT, OutputT]]], type[Workflow[InputT, OutputT]]]: ...

@overload
def workflow(
    *,
    name: str,
    version: str | None = None,
    timeout: timedelta | None = None,
) -> Callable[
    [Callable[Concatenate[WorkflowContext, P], Awaitable[OutputT]]],
    Callable[Concatenate[WorkflowContext, P], Awaitable[OutputT]]
]: ...

def workflow(*, name: str, **kwargs):
    """Decorator to define a workflow."""
    ...
```

### Type-Safe Context Methods

```python
class WorkflowContext(Protocol):
    # Type parameter inference from task class
    async def execute_task(
        self,
        task: type[Task[InputT, OutputT]],
        input: InputT,
        *,
        timeout: timedelta | None = None,
    ) -> OutputT:
        """Execute a task with full type inference."""
        ...

    # Overload for function-based tasks
    @overload
    async def execute_task(
        self,
        task: Callable[[TaskContext, InputT], Awaitable[OutputT]],
        input: InputT,
    ) -> OutputT: ...

    # State with type hints
    async def get_state(
        self,
        key: str,
        *,
        type_hint: type[T],
        default: T | None = None,
    ) -> T | None:
        """Get typed state value."""
        ...
```

### Type Annotation Examples

With string-based API, type annotations are explicit (no automatic inference):

```python
@task(name="process-payment")
class ProcessPayment:
    async def run(self, ctx: TaskContext, input: PaymentRequest) -> PaymentResult:
        ...

@workflow(name="order")
class OrderWorkflow:
    async def run(self, ctx: WorkflowContext, input: OrderInput) -> OrderOutput:
        # String-based API: caller annotates expected type
        payment: PaymentResult = await ctx.execute_task(
            "process-payment",
            {"amount": 100.0, "currency": "USD"},
        )

        return OrderOutput(...)

# Client API (string-based for distributed systems)
client = FlovynClient.builder().build()

# Start workflow by kind string
handle = await client.start_workflow(
    "order",  # workflow kind
    {"order_id": "123", "items": ["item1"]},
)

# Result is Any - caller deserializes as needed
result = await handle.result()
```

### Runtime Type Validation

Pydantic provides runtime validation:

```python
from pydantic import BaseModel, field_validator

class OrderInput(BaseModel):
    order_id: str
    items: list[str]
    total: float

    @field_validator("total")
    @classmethod
    def total_must_be_positive(cls, v: float) -> float:
        if v <= 0:
            raise ValueError("total must be positive")
        return v

# Runtime validation happens automatically
input = OrderInput(order_id="123", items=["a"], total=-10)  # Raises ValidationError
```

## Serialization

### Auto-Detection (Inspired by Restate)

The SDK automatically detects the best serializer based on type hints:

```python
from pydantic import BaseModel
from dataclasses import dataclass
import msgspec

# Tier 1: Pydantic models (preferred)
class OrderInput(BaseModel):
    order_id: str
    items: list[str]

# Tier 2: msgspec.Struct (faster, optional dependency)
class FastOrder(msgspec.Struct):
    order_id: str
    items: list[str]

# Tier 3: dataclasses (requires dacite)
@dataclass
class SimpleOrder:
    order_id: str
    items: list[str]

# Tier 4: TypedDict / dict (fallback)
class OrderDict(TypedDict):
    order_id: str
    items: list[str]

# Auto-detection in action - no explicit serde needed
@task(name="process-order")
class ProcessOrder:
    async def run(self, ctx: TaskContext, input: OrderInput) -> OrderResult:
        # Pydantic automatically used for serialization/deserialization
        return OrderResult(...)
```

### Explicit Serializer Configuration

```python
from flovyn import Serializer, JsonSerde, MsgPackSerde, PydanticSerde

# Per-operation serde override
result = await ctx.run(
    "external-call",
    lambda: fetch_data(),
    serde=MsgPackSerde(),  # Use msgpack for this operation
)

# Per-client default
client = (
    FlovynClient.builder()
    .default_serde(MsgPackSerde())
    .build()
)

# Custom serializer
class PickleSerde(Serializer[T]):
    def serialize(self, value: T) -> bytes:
        return pickle.dumps(value)

    def deserialize(self, data: bytes, type_hint: type[T]) -> T:
        return pickle.loads(data)
```

### JSON Schema Generation

Automatic schema generation for workflow/task inputs:

```python
from flovyn import get_workflow_schema, get_task_schema

# Get JSON schema for workflow input
schema = get_workflow_schema(OrderWorkflow)
# {
#   "type": "object",
#   "properties": {
#     "order_id": {"type": "string"},
#     "items": {"type": "array", "items": {"type": "string"}}
#   },
#   "required": ["order_id", "items"]
# }

# Useful for API documentation, validation, UI generation
```

## Testing

### Unit Testing with Mocks

```python
import pytest
from flovyn.testing import MockWorkflowContext, MockTaskContext

@pytest.mark.asyncio
async def test_order_workflow():
    ctx = MockWorkflowContext()

    # Configure mock task results
    ctx.mock_task_result(ValidateOrder, ValidationResult(valid=True))
    ctx.mock_task_result(ProcessPayment, PaymentResult(transaction_id="txn-123"))
    ctx.mock_task_result(FulfillOrder, FulfillmentResult(shipped=True))

    # Execute workflow
    workflow = OrderWorkflow()
    result = await workflow.run(ctx, OrderInput(
        order_id="order-1",
        items=["item1"],
        total=99.99,
    ))

    # Assertions
    assert result.confirmation_id == "txn-123"
    assert result.status == "completed"
    assert ctx.executed_tasks == [ValidateOrder, ProcessPayment, FulfillOrder]

@pytest.mark.asyncio
async def test_send_email_task():
    ctx = MockTaskContext()

    task = SendEmail()
    result = await task.run(ctx, EmailRequest(
        to="test@example.com",
        subject="Test",
        body="Hello",
    ))

    assert result.message_id is not None
    assert ctx.progress_reports[-1].progress == 1.0
```

### E2E Testing with Testcontainers

```python
import pytest
from flovyn.testing import FlovynTestEnvironment

@pytest.fixture
async def env():
    """Start Flovyn server in container."""
    async with FlovynTestEnvironment() as env:
        env.register_workflow(OrderWorkflow)
        env.register_task(ValidateOrder)
        env.register_task(ProcessPayment)
        env.register_task(FulfillOrder)
        await env.start()
        yield env

@pytest.mark.asyncio
async def test_order_workflow_e2e(env: FlovynTestEnvironment):
    # Start workflow
    handle = await env.start_workflow(
        OrderWorkflow,
        OrderInput(order_id="test-1", items=["item1"], total=50.00),
    )

    # Wait for completion
    result = await handle.result(timeout=timedelta(seconds=30))

    assert result.status == "completed"

@pytest.mark.asyncio
async def test_workflow_cancellation(env: FlovynTestEnvironment):
    handle = await env.start_workflow(LongRunningWorkflow, {})

    # Cancel after short delay
    await asyncio.sleep(1)
    await handle.cancel()

    with pytest.raises(WorkflowCancelled):
        await handle.result()
```

### Time Control

```python
from flovyn.testing import MockWorkflowContext, TimeController

@pytest.mark.asyncio
async def test_timer_workflow():
    time_controller = TimeController(start_time=datetime(2024, 1, 1, tzinfo=UTC))
    ctx = MockWorkflowContext(time_controller=time_controller)

    workflow = ReminderWorkflow()

    # Start workflow execution in background
    task = asyncio.create_task(workflow.run(ctx, ReminderInput(
        message="Test",
        delay=timedelta(hours=1),
    )))

    # Advance time
    await time_controller.advance(timedelta(hours=1))

    # Workflow should complete
    result = await task
    assert result.sent is True
```

## Activation Model

The SDK uses an **activation-based execution model** (similar to Temporal's SDK Core):

### Poll-Complete Cycle

```
┌─────────────────┐     poll_workflow_activation()     ┌─────────────────┐
│  Python Worker  │ ──────────────────────────────────→│   Rust Core     │
│                 │                                     │  (worker-ffi)   │
│                 │←────────────────────────────────── │                 │
│                 │     WorkflowActivation              │                 │
│                 │     (jobs to process)               │                 │
│                 │                                     │                 │
│  Execute jobs   │                                     │                 │
│  via context    │                                     │                 │
│                 │                                     │                 │
│                 │     complete_workflow_activation()  │                 │
│                 │ ──────────────────────────────────→│                 │
│                 │     (commands + status)             │                 │
└─────────────────┘                                     └─────────────────┘
```

### Activation Jobs

The Rust core sends activations containing jobs for Python to process:

```python
@dataclass
class WorkflowActivation:
    """Activation received from Rust core."""
    workflow_execution_id: str
    workflow_kind: str
    context: FfiWorkflowContext  # Replay-aware context
    jobs: list[WorkflowActivationJob]

@dataclass
class WorkflowActivationJob:
    """A single job within an activation."""
    # Variants:
    # - Initialize(input: bytes)
    # - FireTimer(timer_id: str)
    # - ResolveTask(task_execution_id: str, result: bytes)
    # - FailTask(task_execution_id: str, error: str, retryable: bool)
    # - ResolvePromise(promise_name: str, value: bytes)
    # - Signal(signal_name: str, payload: bytes)
    # - Query(query_name: str, args: bytes)
    # - CancelWorkflow()
    ...
```

### Worker Loop (Internal)

```python
class WorkflowWorker:
    """Internal worker that processes workflow activations."""

    async def run(self) -> None:
        while not self._shutdown:
            # Poll for activation from Rust core
            activation = await self._core_worker.poll_workflow_activation()

            if activation is None:
                continue

            # Create Python context wrapping FFI context
            ctx = WorkflowContextImpl(activation.context, self._serializer)

            # Process activation
            try:
                status = await self._process_activation(activation, ctx)
            except Exception as e:
                status = WorkflowCompletionStatus.Failed(
                    error=str(e),
                    stack_trace=traceback.format_exc(),
                )

            # Send completion back to Rust core
            await self._core_worker.complete_workflow_activation(
                workflow_execution_id=activation.workflow_execution_id,
                status=status,
            )

    async def _process_activation(
        self,
        activation: WorkflowActivation,
        ctx: WorkflowContext,
    ) -> WorkflowCompletionStatus:
        """Process all jobs in the activation."""
        workflow = self._registry.get(activation.workflow_kind)

        for job in activation.jobs:
            match job:
                case Initialize(input=input_bytes):
                    input = self._serializer.deserialize(input_bytes, workflow.input_type)
                    try:
                        output = await workflow.run(ctx, input)
                        return WorkflowCompletionStatus.Completed(
                            output=self._serializer.serialize(output)
                        )
                    except WorkflowSuspended:
                        return WorkflowCompletionStatus.Suspended()

                case FireTimer(timer_id=timer_id):
                    ctx._fire_timer(timer_id)

                case ResolveTask(task_execution_id=id, result=result):
                    ctx._resolve_task(id, result)

                # ... handle other job types

        return WorkflowCompletionStatus.Suspended()
```

### Why Activation-Based?

1. **Determinism** - Rust core manages history replay; Python just processes jobs
2. **Efficiency** - Only wake Python when there's work to do
3. **Consistency** - Same model as Kotlin SDK (and Temporal)
4. **Separation** - Core logic in Rust, language bindings are thin wrappers

## UniFFI Integration

### Native Library Loading

```python
# flovyn/_native/loader.py
import platform
import sys
from pathlib import Path

def load_native_library():
    """Load the appropriate native library for the current platform."""
    system = platform.system().lower()
    machine = platform.machine().lower()

    lib_name = {
        ("linux", "x86_64"): "libflovyn_worker_ffi.so",
        ("linux", "aarch64"): "libflovyn_worker_ffi.so",
        ("darwin", "x86_64"): "libflovyn_worker_ffi.dylib",
        ("darwin", "arm64"): "libflovyn_worker_ffi.dylib",
        ("windows", "amd64"): "flovyn_worker_ffi.dll",
    }.get((system, machine))

    if not lib_name:
        raise RuntimeError(f"Unsupported platform: {system}/{machine}")

    # Extract from package resources and load
    lib_path = extract_native_lib(lib_name)
    return load_library(lib_path)
```

### FFI Context Wrapping

```python
# flovyn/context.py
from flovyn._native import flovyn_worker_ffi as ffi

class WorkflowContextImpl(WorkflowContext):
    """Workflow context implementation wrapping FFI context."""

    def __init__(self, ffi_context: ffi.FfiWorkflowContext, serializer: Serializer):
        self._ffi = ffi_context
        self._serializer = serializer

    def current_time(self) -> datetime:
        millis = self._ffi.current_time_millis()
        return datetime.fromtimestamp(millis / 1000, tz=UTC)

    def random_uuid(self) -> UUID:
        return UUID(self._ffi.random_uuid())

    async def execute_task(self, task, input, *, timeout=None, retry_policy=None):
        task_kind = get_task_kind(task)
        input_bytes = self._serializer.serialize(input)

        result = self._ffi.schedule_task(
            task_execution_id=str(self.random_uuid()),
            kind=task_kind,
            input=input_bytes,
            timeout_ms=int(timeout.total_seconds() * 1000) if timeout else None,
        )

        match result:
            case ffi.FfiTaskResult.Completed(output=output):
                return self._serializer.deserialize(output, get_output_type(task))
            case ffi.FfiTaskResult.Failed(error=error, retryable=retryable):
                raise TaskFailed(error, retryable=retryable)
            case ffi.FfiTaskResult.Pending(task_execution_id=_):
                raise WorkflowSuspended()

    async def sleep(self, duration: timedelta) -> None:
        timer_id = str(self.random_uuid())
        result = self._ffi.create_timer(timer_id, int(duration.total_seconds() * 1000))

        match result:
            case ffi.FfiTimerResult.Fired():
                return
            case ffi.FfiTimerResult.Pending():
                raise WorkflowSuspended()
```

## Platform Support

| Platform | Architecture | Status |
|----------|--------------|--------|
| Linux    | x86_64       | ✅     |
| Linux    | aarch64      | ✅     |
| macOS    | x86_64       | ✅     |
| macOS    | arm64        | ✅     |
| Windows  | x86_64       | ✅     |
| Windows  | aarch64      | ✅     |

## Dependencies

**Runtime:**
- Python >= 3.11 (for modern typing features)
- pydantic >= 2.0 (serialization, validation)
- typing-extensions (backports if needed)

**Development:**
- pytest, pytest-asyncio (testing)
- testcontainers (E2E testing)
- mypy (type checking)
- ruff (linting, formatting)

## Summary

### Design Principles

1. **Pythonic API**
   - Decorators (`@workflow`, `@task`) for definitions
   - `snake_case` naming throughout
   - Context managers (`async with client`)
   - Native `async`/`await` integration

2. **Fully Typed**
   - Complete type annotations on all public APIs
   - Generic types for type inference (`WorkflowHandle[T]`, `TaskHandle[T]`)
   - PEP 561 compliant (`py.typed` marker)
   - Works with mypy, pyright, pylance

3. **Runtime Type Safety**
   - Pydantic models for input/output validation
   - Automatic JSON schema generation
   - Clear error messages for type mismatches

4. **Determinism via Context**
   - All non-deterministic operations through `WorkflowContext`
   - No workflow sandbox (simpler than Temporal's approach)
   - Rust core handles replay and history validation

5. **Activation-Based Model**
   - Same architecture as Temporal SDK Core
   - Rust core manages complexity; Python is a thin wrapper
   - Poll-complete cycle for efficient execution

6. **Cross-Platform**
   - Native binaries via UniFFI (same as Kotlin SDK)
   - Linux, macOS, Windows support
   - x86_64 and ARM64 architectures

### Influences from Competitors

| Feature | Inspired By | Flovyn Implementation |
|---------|-------------|----------------------|
| Activation model | Temporal | Poll-complete cycle via UniFFI |
| String-based invocation | Temporal | `ctx.execute_task("task-kind", input)` |
| DAG dependencies | Hatchet | `@task(parents=["task-1", "task-2"])` |
| Durable side effects | Restate | `ctx.run("name", fn)` |
| select() pattern | Restate | `match await select(a=..., b=...)` |
| Conditional execution | Hatchet | `skip_if`, `wait_for` parameters |
| Multi-tier serde | Restate | Auto-detect Pydantic/msgspec/dataclass |
| Workflow triggers | Hatchet | `on_cron`, `on_events` parameters |
| Parent output access | Hatchet | `ctx.task_output("parent-task")` |

### Key Differentiators

| Aspect | Flovyn | Temporal | Restate | Hatchet |
|--------|--------|----------|---------|---------|
| FFI Technology | UniFFI | PyO3 | None (HTTP) | None (gRPC) |
| Multi-language | Kotlin/Swift/Python | Per-language | Per-language | Per-language |
| Native deps | Yes (Rust) | Yes (Rust) | No | No |
| Determinism | Context-only | Sandbox + context | Context-only | Context + conditions |

### Public API Surface

```python
# Core decorators
from flovyn import workflow, task, dynamic_workflow, dynamic_task

# Context types
from flovyn import WorkflowContext, TaskContext

# Client and handles
from flovyn import FlovynClient, WorkflowHandle, TaskHandle

# Configuration
from flovyn import RetryPolicy, WorkflowHook

# Concurrency patterns (inspired by Restate)
from flovyn import select

# Conditional execution (inspired by Hatchet)
from flovyn import skip_if, wait_for, cancel_if
from flovyn import SleepCondition, ParentCondition, EventCondition

# Serialization
from flovyn import Serializer, JsonSerde, PydanticSerde, MsgPackSerde

# Exceptions
from flovyn import (
    FlovynError,
    WorkflowCancelled,
    WorkflowSuspended,
    TaskFailed,
    TaskCancelled,
    TaskTimeout,
    DeterminismViolation,
    PromiseTimeout,
    PromiseRejected,
)

# Testing utilities
from flovyn.testing import (
    MockWorkflowContext,
    MockTaskContext,
    FlovynTestEnvironment,
    TimeController,
)
```
