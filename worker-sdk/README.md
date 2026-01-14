# Flovyn SDK for Rust

The official Rust SDK for [Flovyn](https://github.com/flovyn/flovyn) - a unified workflow orchestration platform that uses event sourcing with deterministic replay.

## Features

- **Workflow Definitions** - Define durable workflows with typed inputs/outputs
- **Task Execution** - Schedule long-running tasks with progress tracking and retries
- **Deterministic Replay** - Workflows survive restarts and replay exactly
- **State Management** - Persist workflow state across executions
- **Durable Timers** - Sleep that survives worker restarts
- **Durable Promises** - Wait for external signals/events
- **Child Workflows** - Orchestrate complex workflow hierarchies
- **Cancellation** - Graceful cancellation with cleanup

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
flovyn-worker-sdk = { git = "https://github.com/flovyn/flovyn", branch = "main" }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

## Quick Start

### 1. Define a Workflow

```rust
use flovyn_worker_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct OrderInput {
    order_id: String,
    amount: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct OrderOutput {
    order_id: String,
    status: String,
}

struct OrderWorkflow;

#[async_trait]
impl WorkflowDefinition for OrderWorkflow {
    type Input = OrderInput;
    type Output = OrderOutput;

    fn kind(&self) -> &str {
        "order-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: Self::Input,
    ) -> Result<Self::Output> {
        // Deterministic timestamp (same on replay)
        let timestamp = ctx.current_time_millis();

        // Cached operation (won't re-execute on replay)
        let validated = ctx.run_raw("validate-order", json!({
            "valid": true,
            "timestamp": timestamp
        })).await?;

        // Schedule external task
        let payment: Value = ctx.schedule_raw("payment-task", json!({
            "order_id": input.order_id.clone(),
            "amount": input.amount,
        })).await?;

        Ok(OrderOutput {
            order_id: input.order_id,
            status: "completed".to_string(),
        })
    }
}
```

### 2. Define a Task

```rust
use flovyn_worker_sdk::prelude::*;

struct PaymentTask;

#[async_trait]
impl DynamicTask for PaymentTask {
    fn kind(&self) -> &str {
        "payment-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("");

        // Report progress
        ctx.report_progress(0.5, Some("Processing payment")).await?;

        // Check for cancellation
        if ctx.is_cancelled() {
            return Err(FlovynError::TaskCancelled);
        }

        ctx.report_progress(1.0, Some("Payment completed")).await?;

        let mut output = Map::new();
        output.insert("transaction_id".to_string(), json!("txn-12345"));
        output.insert("status".to_string(), json!("succeeded"));
        Ok(output)
    }

    fn retry_config(&self) -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            initial_backoff: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(60),
        }
    }
}
```

### 3. Start the Worker

```rust
use flovyn_worker_sdk::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build the client with fluent registration
    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .org_id(Uuid::parse_str("your-org-id")?)
        .task_queue("default")
        .register_workflow(OrderWorkflow)
        .register_task(PaymentTask)
        .build()
        .await?;

    // Start the workers
    let handle = client.start().await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    handle.stop().await;

    Ok(())
}
```

## Core Concepts

### WorkflowContext

The `WorkflowContext` provides APIs for workflow execution:

| Method | Description |
|--------|-------------|
| `workflow_execution_id()` | Unique ID of this execution |
| `org_id()` | Organization ID |
| `current_time_millis()` | Deterministic timestamp |
| `random_uuid()` | Deterministic UUID generation |
| `random()` | Deterministic random number generator |
| `run_raw(name, result)` | Execute and cache a side effect |
| `schedule_raw(task_type, input)` | Schedule a task and wait for result |
| `get_raw(key)` / `set_raw(key, value)` | State management |
| `sleep(duration)` | Durable timer |
| `promise_raw(name)` | Wait for external signal |
| `schedule_workflow_raw(name, kind, input)` | Schedule child workflow |
| `check_cancellation()` | Check if workflow was cancelled |

### WorkflowDefinition

Define workflows by implementing the `WorkflowDefinition` trait:

```rust
#[async_trait]
pub trait WorkflowDefinition: Send + Sync {
    type Input: Serialize + DeserializeOwned + Send;
    type Output: Serialize + DeserializeOwned + Send;

    fn kind(&self) -> &str;
    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output>;

    // Optional overrides
    fn name(&self) -> &str { self.kind() }
    fn version(&self) -> SemanticVersion { SemanticVersion::default() }
    fn description(&self) -> Option<&str> { None }
    fn timeout_seconds(&self) -> Option<u32> { None }
    fn cancellable(&self) -> bool { false }
    fn tags(&self) -> Vec<String> { vec![] }
}
```

For dynamic workflows (JSON input/output), implement `DynamicWorkflow` instead.

### TaskDefinition

Define tasks by implementing the `TaskDefinition` trait:

```rust
#[async_trait]
pub trait TaskDefinition: Send + Sync {
    type Input: Serialize + DeserializeOwned + Send;
    type Output: Serialize + DeserializeOwned + Send;

    fn kind(&self) -> &str;
    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output>;

    // Optional overrides
    fn retry_config(&self) -> RetryConfig { RetryConfig::default() }
    fn timeout_seconds(&self) -> Option<u32> { None }
    fn heartbeat_timeout_seconds(&self) -> Option<u32> { None }
}
```

For dynamic tasks (JSON input/output), implement `DynamicTask` instead.

### TaskContext

The `TaskContext` provides APIs for task execution:

| Method | Description |
|--------|-------------|
| `task_execution_id()` | Unique ID of this task execution |
| `attempt()` | Current retry attempt (1-based) |
| `report_progress(progress, message)` | Report progress (0.0-1.0) |
| `log(level, message)` | Log a message |
| `is_cancelled()` | Check if task was cancelled |
| `check_cancellation()` | Throw if cancelled |

## Deterministic Operations

Workflows must be deterministic - they produce the same results on replay. Use these APIs instead of standard library functions:

| Instead of | Use |
|------------|-----|
| `std::time::SystemTime::now()` | `ctx.current_time_millis()` |
| `uuid::Uuid::new_v4()` | `ctx.random_uuid()` |
| `rand::thread_rng()` | `ctx.random()` |

**Why?** Standard functions return different values on each call, breaking replay. The `ctx.*` methods return values recorded during the first execution.

## State Management

Persist data across workflow tasks:

```rust
// Set state
ctx.set_raw("order_status", json!("processing")).await?;

// Get state
let status: Option<Value> = ctx.get_raw("order_status").await?;

// Clear state
ctx.clear("order_status").await?;

// Get all keys
let keys: Vec<String> = ctx.state_keys().await?;
```

## Durable Timers

Sleep that survives worker restarts:

```rust
use std::time::Duration;

// Sleep for 1 hour (durable)
ctx.sleep(Duration::from_secs(3600)).await?;

// Workflow continues after timer fires, even if worker restarted
```

## Durable Promises

Wait for external events/signals:

```rust
// Wait for user approval
let approval: Value = ctx.promise_raw("user-approval").await?;

// Or with timeout
let approval: Value = ctx.promise_with_timeout_raw(
    "user-approval",
    Duration::from_secs(86400), // 24 hours
).await?;
```

Resolve the promise externally:

```rust
client.resolve_promise(
    workflow_execution_id,
    "user-approval",
    json!({"approved": true, "approver": "admin"}),
).await?;
```

## Child Workflows

Orchestrate workflow hierarchies:

```rust
let result: Value = ctx.schedule_workflow_raw(
    "process-item",      // name
    "item-processor",    // workflow kind
    json!({"item_id": "item-123"}),
).await?;
```

## Cancellation

Handle graceful cancellation:

```rust
async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
    // Step 1
    let payment = ctx.schedule_raw("payment-task", input.clone()).await?;

    // Check for cancellation at safe points
    ctx.check_cancellation().await?;

    // Step 2
    let shipping = ctx.schedule_raw("shipping-task", input.clone()).await?;

    Ok(output)
}
```

When `check_cancellation()` detects cancellation, it returns `Err(FlovynError::TaskCancelled)`.

## Configuration

### Client Configuration

```rust
let client = FlovynClient::builder()
    .server_address("localhost", 9090)
    .org_id(org_id)
    .worker_id("my-worker-1")
    .task_queue("gpu-workers")
    .poll_timeout(Duration::from_secs(60))
    .heartbeat_interval(Duration::from_secs(30))
    .max_concurrent_workflows(10)
    .max_concurrent_tasks(20)
    .register_workflow(MyWorkflow)
    .register_task(MyTask)
    .register_hook(LoggingHook::info())
    .build()
    .await?;
```

### Configuration Presets

```rust
use flovyn_worker_sdk::config::FlovynClientConfig;

// High-throughput configuration
let config = FlovynClientConfig::high_throughput();

// Low-resource configuration
let config = FlovynClientConfig::low_resource();
```

### Workflow Hooks

Monitor workflow lifecycle events:

```rust
use flovyn_worker_sdk::prelude::*;

struct MetricsHook;

#[async_trait]
impl WorkflowHook for MetricsHook {
    async fn on_workflow_started(&self, id: Uuid, kind: &str, input: &Value) {
        // Record start metric
    }

    async fn on_workflow_completed(&self, id: Uuid, kind: &str, result: &Value) {
        // Record completion metric
    }

    async fn on_workflow_failed(&self, id: Uuid, kind: &str, error: &str) {
        // Record failure metric
    }
}

let client = FlovynClient::builder()
    .server_address("localhost", 9090)
    .org_id(org_id)
    .register_workflow(MyWorkflow)
    .register_task(MyTask)
    .register_hook(MetricsHook)
    .register_hook(LoggingHook::info())
    .build()
    .await?;
```

## Error Handling

The SDK uses `FlovynError` for all errors:

```rust
use flovyn_worker_sdk::FlovynError;

match result {
    Err(FlovynError::Suspended(reason)) => {
        // Workflow suspended (waiting for task/timer/promise)
    }
    Err(FlovynError::TaskCancelled) => {
        // Task or workflow was cancelled
    }
    Err(FlovynError::DeterminismViolation { message, sequence }) => {
        // Workflow changed between replays
    }
    Err(FlovynError::Grpc(status)) => {
        // gRPC communication error
    }
    Err(FlovynError::Serialization(err)) => {
        // JSON serialization error
    }
    _ => {}
}
```

## Prelude

Import commonly used types with the prelude:

```rust
use flovyn_worker_sdk::prelude::*;

// Includes:
// - FlovynClient, FlovynClientBuilder
// - WorkflowContext, WorkflowDefinition, DynamicWorkflow
// - TaskContext, TaskDefinition, DynamicTask
// - FlovynError, Result
// - SemanticVersion, RetryConfig
// - async_trait, Serialize, Deserialize
// - json!, Value, Map, Uuid
```

## Examples

See the [samples](../../samples/rust/) directory for complete examples:

- **hello-world** - Minimal workflow example
- **ecommerce** - Order processing with saga pattern
- **data-pipeline** - ETL pipeline with DAG pattern
- **patterns** - Timer, promise, and child workflow examples

## Testing

Run the test suite:

```bash
# All tests
cargo test

# Unit tests only
cargo test --test unit

# With testing utilities
cargo test --features testing

# With output
cargo test -- --nocapture
```

### Testing Utilities (Feature: `testing`)

Enable testing utilities to test workflows and tasks without a running server:

```toml
[dev-dependencies]
flovyn-worker-sdk = { git = "https://github.com/flovyn/flovyn", features = ["testing"] }
```

#### MockWorkflowContext

Test workflow logic in isolation with a mock context:

```rust
use flovyn_worker_sdk::testing::{MockWorkflowContext, WorkflowTestBuilder};
use flovyn_worker_sdk::workflow::context::WorkflowContext;
use serde_json::json;

#[tokio::test]
async fn test_workflow_logic() {
    // Create mock context with builder
    let ctx = MockWorkflowContext::builder()
        .input(json!({"order_id": "ORD-123", "amount": 99.99}))
        .task_result("payment-task", json!({"status": "succeeded"}))
        .promise_result("approval", json!({"approved": true}))
        .build();

    // Test workflow operations
    assert!(!ctx.workflow_execution_id().is_nil());
    assert_eq!(ctx.input_raw(), &json!({"order_id": "ORD-123", "amount": 99.99}));

    // Verify scheduled tasks
    assert!(!ctx.was_task_scheduled("payment-task"));
}
```

#### MockTaskContext

Test task logic in isolation:

```rust
use flovyn_worker_sdk::testing::MockTaskContext;
use flovyn_worker_sdk::task::context::{TaskContext, LogLevel};

#[tokio::test]
async fn test_task_progress() {
    let ctx = MockTaskContext::builder()
        .attempt(1)
        .build();

    // Report progress
    ctx.report_progress(0.5, Some("Processing")).await.unwrap();
    ctx.report_progress(1.0, Some("Done")).await.unwrap();

    // Verify progress reports
    let reports = ctx.progress_reports();
    assert_eq!(reports.len(), 2);
    assert_eq!(ctx.last_progress(), Some(1.0));

    // Log messages
    ctx.log(LogLevel::Info, "Task completed").await.unwrap();
    assert!(ctx.was_logged("completed"));
}
```

#### TestWorkflowEnvironment

In-memory execution environment for integration testing:

```rust
use flovyn_worker_sdk::testing::TestWorkflowEnvironment;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn test_workflow_execution() {
    let env = TestWorkflowEnvironment::new();

    // Register workflows and tasks
    env.register_workflow(MyWorkflow).unwrap();
    env.register_task(MyTask).unwrap();

    // Set expected task results
    env.set_task_result("my-task", json!({"result": "success"}));

    // Execute workflow
    let result = env.execute_workflow("my-workflow", json!({"input": "data"})).await;
    assert!(result.is_ok());

    // Check execution history
    let history = env.workflow_executions();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, "my-workflow");
}
```

#### TimeController

Control time progression for testing timers:

```rust
use flovyn_worker_sdk::testing::TimeController;
use std::time::Duration;

#[test]
fn test_time_control() {
    let controller = TimeController::with_initial_time(1000);

    // Register a timer
    controller.register_timer("timer-1", Duration::from_secs(60));

    // Advance time
    let fired = controller.advance(Duration::from_secs(30));
    assert!(fired.is_empty()); // Timer not yet fired

    // Advance past timer
    let fired = controller.advance(Duration::from_secs(60));
    assert!(fired.contains(&"timer-1".to_string()));

    // Skip to next timer
    controller.register_timer("timer-2", Duration::from_secs(120));
    let fired = controller.advance_to_next_timer();
    assert!(fired.contains(&"timer-2".to_string()));

    // Skip all timers
    controller.register_timer("timer-3", Duration::from_secs(300));
    controller.register_timer("timer-4", Duration::from_secs(600));
    let fired = controller.skip_all_timers();
    assert_eq!(fired.len(), 2);
}
```

#### WorkflowTestBuilder and TaskTestBuilder

Fluent builders for setting up test scenarios with replay events:

```rust
use flovyn_worker_sdk::testing::{WorkflowTestBuilder, TaskTestBuilder};
use serde_json::json;
use std::time::Duration;

#[test]
fn test_workflow_with_events() {
    let (ctx, events) = WorkflowTestBuilder::new()
        .input(json!({"order_id": "ORD-123"}))
        .with_workflow_started(json!({}))
        .with_operation_completed("validate", json!({"valid": true}))
        .with_task_scheduled("payment-task", json!({"amount": 100}))
        .with_task_completed("payment-task", json!({"status": "ok"}))
        .with_timer_started("delay", Duration::from_secs(60))
        .with_timer_fired("delay")
        .build();

    assert_eq!(events.len(), 6);
}

#[test]
fn test_task_setup() {
    let ctx = TaskTestBuilder::new()
        .attempt(2)
        .cancelled()
        .build();

    assert_eq!(ctx.attempt(), 2);
    assert!(ctx.is_cancelled());
}
```

#### Assert Helpers

Convenient assertion functions for test results:

```rust
use flovyn_worker_sdk::testing::*;
use serde_json::json;

#[test]
fn test_assertions() {
    let result = json!({"status": "completed", "data": {"count": 42}});

    // Workflow assertions
    assert_workflow_completed(&Ok(result.clone()));
    assert_workflow_failed(&Err(FlovynError::Other("error".into())), "error");

    // JSON assertions
    assert_json_has(&result, "status", &json!("completed"));
    assert_json_has_key(&result, "data");
    assert_json_missing_key(&result, "error");

    // Nested paths
    let count = result.get("data").and_then(|d| d.get("count"));
    assert_eq!(count, Some(&json!(42)));
}
```

## Requirements

- Rust 1.70+
- Flovyn server running (default: `localhost:9090`)
- PostgreSQL (for the server)

## License

Apache-2.0
