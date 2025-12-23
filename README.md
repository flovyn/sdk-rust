# Flovyn Rust SDK

Rust SDK for building workflows and tasks with [Flovyn](https://github.com/flovyn/flovyn) workflow orchestration platform.

## Features

- **Deterministic Workflows**: Event-sourced execution with automatic replay
- **Task Execution**: Long-running task support with progress tracking
- **Testing Utilities**: Mock contexts and test environments
- **Type-safe API**: Strongly-typed workflow and task definitions

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
flovyn-sdk = "0.1"
```

Or use a path dependency during development:

```toml
[dependencies]
flovyn-sdk = { path = "../sdk-rust/sdk" }
```

## Quick Start

### Define a Workflow

```rust
use flovyn_sdk::prelude::*;

struct GreetingWorkflow;

#[async_trait]
impl WorkflowDefinition for GreetingWorkflow {
    type Input = GreetingInput;
    type Output = GreetingOutput;

    fn kind(&self) -> &str {
        "greeting-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: Self::Input,
    ) -> Result<Self::Output> {
        // Use deterministic time
        let timestamp = ctx.current_time_millis();

        // Cache side effects
        let greeting = ctx.run_raw("format-greeting", json!({
            "name": input.name,
            "timestamp": timestamp,
        })).await?;

        Ok(GreetingOutput { greeting })
    }
}
```

### Define a Task

```rust
use flovyn_sdk::prelude::*;

struct ProcessDataTask;

#[async_trait]
impl TaskDefinition for ProcessDataTask {
    type Input = DataInput;
    type Output = DataOutput;

    fn kind(&self) -> &str {
        "process-data"
    }

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &dyn TaskContext,
    ) -> Result<Self::Output> {
        ctx.report_progress(0.0, Some("Starting...")).await?;

        // Process data...

        ctx.report_progress(1.0, Some("Complete")).await?;
        Ok(DataOutput { /* ... */ })
    }
}
```

### Start a Worker

```rust
use flovyn_sdk::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let client = FlovynClient::builder()
        .server_url("http://localhost:9090")
        .build()
        .await?;

    client
        .worker()
        .register_workflow(GreetingWorkflow)
        .register_task(ProcessDataTask)
        .start()
        .await
}
```

## Examples

See the [examples](./examples) directory for complete working examples:

| Example | Description |
|---------|-------------|
| [hello-world](./examples/hello-world) | Minimal workflow example |
| [ecommerce](./examples/ecommerce) | Order processing with saga pattern |
| [data-pipeline](./examples/data-pipeline) | ETL pipeline with DAG pattern |
| [patterns](./examples/patterns) | Timers, promises, child workflows, retry |

## Testing

### Run Tests

```bash
# Run all tests
cargo test --workspace

# Run unit tests only
cargo test --test unit -p flovyn-sdk

# Run TCK (conformance) tests
cargo test --test tck -p flovyn-sdk

# Run E2E tests (requires Docker)
FLOVYN_E2E_USE_DEV_INFRA=1 cargo test --test e2e -p flovyn-sdk -- --include-ignored
```

### E2E Test Logging

E2E tests default to `warn` level to reduce noise. To see more detailed logs:

```bash
# Show INFO logs
RUST_LOG=info FLOVYN_E2E_USE_DEV_INFRA=1 cargo test --test e2e -p flovyn-sdk -- --include-ignored

# Show DEBUG logs for debugging
RUST_LOG=flovyn_sdk=debug FLOVYN_E2E_USE_DEV_INFRA=1 cargo test --test e2e -p flovyn-sdk -- --include-ignored --nocapture
```

## Deterministic APIs

Workflows must be deterministic. Use these APIs instead of standard library:

| Instead of | Use |
|------------|-----|
| `SystemTime::now()` | `ctx.current_time_millis()` |
| `Uuid::new_v4()` | `ctx.random_uuid()` |
| `rand::random()` | `ctx.random()` |

## Documentation

- [API Documentation](https://docs.rs/flovyn-sdk)
- [Flovyn Server](https://github.com/flovyn/flovyn)

## License

Apache-2.0
