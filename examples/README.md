# Flovyn Rust SDK Examples

This directory contains example applications demonstrating the Flovyn Rust SDK.

## Prerequisites

- Rust 1.82+
- Running Flovyn server (`localhost:9090`)
- PostgreSQL database

## Quick Start

```bash
# From the repository root
cargo build --workspace

# Start the hello-world example
cargo run -p hello-world-sample

# In another terminal, trigger a workflow (via REST API)
curl -X POST http://localhost:8080/api/workflows/greeting-workflow \
  -H "Content-Type: application/json" \
  -d '{"name": "World"}'
```

## Samples Overview

| Sample | Description | Patterns |
|--------|-------------|----------|
| [hello-world](hello-world/) | Minimal workflow example | Basic workflow, `ctx.run_raw()` |
| [ecommerce](ecommerce/) | Order processing saga | Saga, compensation, cancellation |
| [data-pipeline](data-pipeline/) | ETL pipeline with DAG | DAG, parallel tasks, state management |
| [patterns](patterns/) | Pattern showcase | Timers, promises, child workflows, retry |
| [standalone-tasks](standalone-tasks/) | Long-running tasks | Logging, progress reporting, cancellation |

## Sample Details

### Hello World

The simplest possible workflow demonstrating:

- Defining a workflow with `WorkflowDefinition`
- Using deterministic timestamps with `ctx.current_time_millis()`
- Caching side effects with `ctx.run_raw()`
- Defining tasks with `TaskDefinition`
- Starting a worker with `FlovynClient::builder()`

```bash
cargo run -p hello-world-sample
```

### E-commerce Order Processing

A complete order processing workflow using the saga pattern:

**Workflow Steps:**
1. Process payment
2. Reserve inventory
3. Create shipment

**Features:**
- Multi-step saga with compensation actions
- Task scheduling with `ctx.schedule_raw()`
- State management with `ctx.set_raw()` and `ctx.get_raw()`
- Cancellation handling with `ctx.check_cancellation()`
- Error handling with rollback

```bash
cargo run -p ecommerce-sample
```

### Data Pipeline

An ETL pipeline demonstrating the DAG (Directed Acyclic Graph) pattern:

**Pipeline Steps:**
1. **[Sequential]** Data Ingestion
2. **[Sequential]** Data Validation
3. **[Parallel]** Data Transformations
4. **[Sequential]** Aggregation

**Features:**
- Sequential and parallel task execution
- Long-running task orchestration
- Progress tracking throughout pipeline
- Execution time measurement

```bash
cargo run -p data-pipeline-sample
```

### Standalone Tasks

Long-running task examples demonstrating logging and progress reporting:

**Tasks Included:**
- `data-export-task`: Exports data in batches with progress tracking
- `report-generation-task`: Multi-phase report generation (data collection, analysis, rendering)
- `backup-task`: Creates backups with checkpoints and compression
- `indexing-task`: Indexes documents with batch progress

**Features:**
- Logging at all levels (debug, info, warn, error) using `ctx.log()`
- Progress reporting with `ctx.report_progress()`
- Random duration between 2-5 minutes per task
- Cancellation support with `ctx.check_cancellation()`
- Detailed progress messages throughout execution

```bash
cargo run -p standalone-tasks-sample
```

**Example curl commands:**

```bash
# Data Export
curl -X POST http://localhost:8000/api/tasks/data-export-task \
  -H "Content-Type: application/json" \
  -d '{"dataset_name":"users","format":"csv","record_count":10000}'

# Report Generation
curl -X POST http://localhost:8000/api/tasks/report-generation-task \
  -H "Content-Type: application/json" \
  -d '{"report_name":"monthly-sales","report_type":"monthly","include_charts":true}'

# Backup
curl -X POST http://localhost:8000/api/tasks/backup-task \
  -H "Content-Type: application/json" \
  -d '{"source_path":"/data/app","backup_type":"full","compress":true}'

# Indexing
curl -X POST http://localhost:8000/api/tasks/indexing-task \
  -H "Content-Type: application/json" \
  -d '{"collection_name":"products","index_type":"full-text","document_count":50000}'
```

### Pattern Showcase

Demonstrations of advanced workflow patterns:

**1. Durable Timers**
- `reminder-workflow`: Schedule reminders that survive restarts
- `multi-step-timer-workflow`: Multiple checkpoint timers

**2. Promises (External Signals)**
- `approval-workflow`: Human-in-the-loop approval
- `multi-approval-workflow`: Multiple approver requirements

**3. Child Workflows**
- `batch-processing-workflow`: Fan-out/fan-in pattern
- `controlled-parallel-workflow`: Controlled parallelism

**4. Retry Patterns**
- `retry-workflow`: Exponential backoff
- `circuit-breaker-workflow`: Circuit breaker pattern

**5. Parallel Execution**
- `fan-out-fan-in-workflow`: Process items in parallel, aggregate results using `join_all`
- `racing-workflow`: Race multiple operations, take first result using `select`
- `timeout-workflow`: Add timeouts to operations using `with_timeout`
- `batch-with-concurrency-workflow`: Process items with controlled parallelism
- `partial-completion-workflow`: Wait for N of M operations using `join_n`

```bash
cargo run -p patterns-sample
```

## Configuration

All samples support configuration via environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `FLOVYN_SERVER_HOST` | Server hostname | `localhost` |
| `FLOVYN_SERVER_PORT` | Server gRPC port | `9090` |
| `FLOVYN_ORG_ID` | Tenant UUID | Random UUID |

Example:

```bash
FLOVYN_SERVER_HOST=flovyn.example.com \
FLOVYN_SERVER_PORT=9090 \
FLOVYN_ORG_ID=550e8400-e29b-41d4-a716-446655440000 \
cargo run -p ecommerce-sample
```

## Project Structure

```
examples/
├── README.md                     # This file
├── hello-world/                  # Minimal example
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
├── ecommerce/                    # E-commerce order processing
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── models.rs
│       ├── workflows/
│       │   └── order_workflow.rs
│       └── tasks/
│           ├── mod.rs
│           ├── payment_task.rs
│           ├── inventory_task.rs
│           └── shipment_task.rs
├── data-pipeline/                # ETL pipeline with DAG
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── models.rs
│       ├── workflows/
│       │   └── pipeline_workflow.rs
│       └── tasks/
│           ├── mod.rs
│           ├── ingestion_task.rs
│           ├── validation_task.rs
│           ├── transformation_task.rs
│           └── aggregation_task.rs
├── patterns/                     # Pattern showcase
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── timer_workflow.rs
│       ├── promise_workflow.rs
│       ├── child_workflow.rs
│       ├── retry_workflow.rs
│       └── parallel_workflow.rs
└── standalone-tasks/             # Long-running task examples
    ├── Cargo.toml
    └── src/
        └── main.rs               # All task definitions
```

## Key Concepts

### Workflow Definition

Workflows are defined by implementing the `WorkflowDefinition` trait:

```rust
use flovyn_sdk::prelude::*;

struct MyWorkflow;

#[async_trait]
impl WorkflowDefinition for MyWorkflow {
    type Input = MyInput;
    type Output = MyOutput;

    fn kind(&self) -> &str { "my-workflow" }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        // Workflow logic here
    }
}
```

### Task Definition

Tasks are defined by implementing the `TaskDefinition` trait:

```rust
use flovyn_sdk::prelude::*;

struct MyTask;

#[async_trait]
impl TaskDefinition for MyTask {
    type Input = TaskInput;
    type Output = TaskOutput;

    fn kind(&self) -> &str { "my-task" }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        ctx.report_progress(0.5, Some("Processing...")).await?;
        // Task logic here
    }
}
```

### Deterministic Operations

All workflows must be deterministic. Use these APIs instead of standard library functions:

| Instead of | Use |
|------------|-----|
| `SystemTime::now()` | `ctx.current_time_millis()` |
| `Uuid::new_v4()` | `ctx.random_uuid()` |
| `rand::random()` | `ctx.random()` |

### Side Effect Caching

Use `ctx.run_raw()` to cache side effects. The result is recorded on first execution and replayed on subsequent runs:

```rust
let result = ctx.run_raw("external-api-call", serde_json::to_value(&data)?).await?;
```

### State Management

Workflows can maintain state that persists across restarts:

```rust
ctx.set_raw("status", serde_json::to_value("processing")?).await?;
let status: String = serde_json::from_value(ctx.get_raw("status").await?.unwrap())?;
```

## Running with Docker

```bash
# Start Flovyn server and dependencies
docker-compose up -d

# Run a sample
cargo run -p hello-world-sample --release
```

## License

Apache 2.0
