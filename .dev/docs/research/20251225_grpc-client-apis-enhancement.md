# Design: Enhanced gRPC Client APIs

**Status**: Draft
**Created**: 2025-12-21
**Author**: Claude Code

## Overview

This document describes the design for exposing additional gRPC client capabilities in the Rust SDK. Currently, `workflow_dispatch_client` is well-utilized, but `workflow_query_client`, `task_execution_client`, and `worker_lifecycle_client` have limited public exposure despite having rich underlying gRPC services.

## Current State

### Used APIs

| Client | Public APIs | Internal Usage |
|--------|-------------|----------------|
| `WorkflowDispatch` | `start_workflow()`, `start_workflow_with_options()`, `get_workflow_events()`, `resolve_promise()`, `reject_promise()` | Poll workflows, submit commands, report spans |
| `WorkflowQuery` | `query()`, `query_typed()` | - |
| `TaskExecution` | `get_task_state()`, `set_task_state()`, `clear_task_state()`, `clear_all_task_state()`, `get_task_state_keys()` | Complete/fail tasks, heartbeat, progress |
| `WorkerLifecycle` | - | Register worker, send heartbeat |

### Missing APIs (Compared to Kotlin SDK)

1. **Workflow Status & Lifecycle**
   - Get workflow execution status
   - Wait for workflow completion with timeout
   - Cancel workflow execution

2. **Enhanced Query System**
   - `QueryableWorkflow` trait for type-safe queries
   - Built-in queries: `getStatus`, `getResult`, `getError`
   - Query error handling with proper types

3. **Task Streaming**
   - Real-time streaming of tokens, progress, data events
   - Subscribe to task execution updates

4. **Workflow Signals**
   - Send external signals to running workflows
   - Signal handlers in workflow definition

5. **Worker Management**
   - Graceful shutdown
   - Worker status introspection
   - Re-registration on connection recovery

## Proposed Design

### 1. Workflow Status & Lifecycle APIs

#### 1.1 WorkflowHandle

Introduce a `WorkflowHandle` type for interacting with a specific workflow execution:

```rust
/// Handle to a running or completed workflow execution
pub struct WorkflowHandle {
    client: FlovynClient,
    workflow_execution_id: Uuid,
}

impl WorkflowHandle {
    /// Get the workflow execution ID
    pub fn id(&self) -> Uuid {
        self.workflow_execution_id
    }

    /// Query the workflow state
    pub async fn query<T: DeserializeOwned>(
        &self,
        query_name: &str,
        params: Value,
    ) -> Result<T, WorkflowQueryError>;

    /// Get the workflow status
    pub async fn status(&self) -> Result<WorkflowStatus, ClientError>;

    /// Wait for the workflow to complete and return the result
    pub async fn result<T: DeserializeOwned>(
        &self,
        timeout: Duration,
    ) -> Result<T, WorkflowResultError>;

    /// Cancel the workflow execution
    pub async fn cancel(&self) -> Result<(), ClientError>;

    /// Get the workflow event history
    pub async fn events(&self) -> Result<Vec<WorkflowEvent>, ClientError>;

    /// Resolve a promise in this workflow
    pub async fn resolve_promise(
        &self,
        promise_name: &str,
        value: Value,
    ) -> Result<(), ClientError>;

    /// Reject a promise in this workflow
    pub async fn reject_promise(
        &self,
        promise_name: &str,
        error: &str,
    ) -> Result<(), ClientError>;
}
```

#### 1.2 WorkflowStatus Enum

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowStatus {
    /// Workflow is queued but not yet started
    Pending,
    /// Workflow is currently executing
    Running,
    /// Workflow completed successfully
    Completed,
    /// Workflow failed with an error
    Failed,
    /// Workflow was cancelled
    Cancelled,
    /// Workflow timed out
    TimedOut,
}
```

#### 1.3 FlovynClient Updates

```rust
impl FlovynClient {
    /// Start a workflow and return a handle for interaction
    pub async fn start_workflow_handle(
        &self,
        workflow_kind: &str,
        input: Value,
    ) -> Result<WorkflowHandle, ClientError>;

    /// Start a workflow with options and return a handle
    pub async fn start_workflow_handle_with_options(
        &self,
        workflow_kind: &str,
        input: Value,
        options: StartWorkflowOptions,
    ) -> Result<WorkflowHandle, ClientError>;

    /// Get a handle to an existing workflow execution
    pub fn workflow(&self, workflow_execution_id: Uuid) -> WorkflowHandle;

    /// Cancel a workflow by ID
    pub async fn cancel_workflow(
        &self,
        workflow_execution_id: Uuid,
    ) -> Result<(), ClientError>;

    /// Get workflow status by ID
    pub async fn get_workflow_status(
        &self,
        workflow_execution_id: Uuid,
    ) -> Result<WorkflowStatus, ClientError>;
}
```

### 2. QueryableWorkflow Trait

Allow workflows to define custom query handlers:

```rust
/// Trait for workflows that support custom queries
pub trait QueryableWorkflow: WorkflowDefinition {
    /// Handle a query request
    ///
    /// The default implementation returns an error for unknown queries.
    /// Override this to provide custom query handlers.
    fn query(
        &self,
        ctx: &QueryContext,
        query_name: &str,
        params: Value,
    ) -> Result<Value, QueryError> {
        Err(QueryError::UnknownQuery(query_name.to_string()))
    }
}

/// Read-only context for query execution
pub struct QueryContext {
    run_id: String,
    workflow_id: Uuid,
    state: HashMap<String, Value>,
    status: WorkflowStatus,
}

impl QueryContext {
    /// Get a value from workflow state
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T>;

    /// Get all state keys
    pub fn state_keys(&self) -> Vec<String>;

    /// Check if workflow is completed
    pub fn is_completed(&self) -> bool;

    /// Check if workflow is running
    pub fn is_running(&self) -> bool;

    /// Check if workflow failed
    pub fn is_failed(&self) -> bool;

    /// Get the current workflow status
    pub fn status(&self) -> WorkflowStatus;
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("Unknown query: {0}")]
    UnknownQuery(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("Query execution failed: {0}")]
    ExecutionFailed(String),
}
```

#### Built-in Queries

All workflows automatically support these queries:

| Query Name | Description | Return Type |
|------------|-------------|-------------|
| `getStatus` | Get workflow execution status | `WorkflowStatus` |
| `getResult` | Get workflow result (if completed) | `Value` |
| `getError` | Get error message (if failed) | `String` |
| `getState` | Get all workflow state | `Map<String, Value>` |

### 3. Task Streaming

Enable real-time streaming from tasks:

```rust
/// Stream events from task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Text token for LLM-style streaming
    Token { text: String },

    /// Progress update
    Progress {
        progress: f64,
        details: Option<String>,
    },

    /// Arbitrary data payload
    Data { data: Value },

    /// Error during streaming
    Error {
        message: String,
        code: Option<String>,
    },
}

impl TaskContext {
    /// Stream an event to subscribers (ephemeral, not persisted)
    pub async fn stream(&self, event: StreamEvent) -> Result<(), TaskError>;
}
```

#### Client-Side Streaming Subscription

```rust
impl FlovynClient {
    /// Subscribe to task streaming events
    pub fn subscribe_task_stream(
        &self,
        task_execution_id: Uuid,
    ) -> impl Stream<Item = Result<StreamEvent, StreamError>>;

    /// Subscribe to all task events for a workflow
    pub fn subscribe_workflow_task_streams(
        &self,
        workflow_execution_id: Uuid,
    ) -> impl Stream<Item = Result<TaskStreamEvent, StreamError>>;
}

pub struct TaskStreamEvent {
    pub task_execution_id: Uuid,
    pub task_kind: String,
    pub event: StreamEvent,
}
```

### 4. Worker Lifecycle Management

Expose worker management APIs:

```rust
impl FlovynClient {
    /// Check if the worker is currently running
    pub fn is_running(&self) -> bool;

    /// Get the worker's assigned ID from the server
    pub fn worker_id(&self) -> &str;

    /// Get the current worker registration status
    pub fn registration_status(&self) -> WorkerRegistrationStatus;

    /// Request graceful shutdown
    pub async fn shutdown(&self) -> Result<(), ClientError>;

    /// Force immediate stop
    pub fn stop(&self);
}

#[derive(Debug, Clone)]
pub enum WorkerRegistrationStatus {
    /// Not yet registered
    Unregistered,
    /// Successfully registered with server
    Registered {
        worker_id: Uuid,
        registered_at: SystemTime,
    },
    /// Registration failed
    Failed {
        error: String,
        last_attempt: SystemTime,
    },
    /// Disconnected, attempting to reconnect
    Reconnecting {
        last_connected: SystemTime,
        attempts: u32,
    },
}
```

### 5. Enhanced Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum WorkflowResultError {
    #[error("Workflow not found: {0}")]
    NotFound(Uuid),

    #[error("Workflow still running")]
    StillRunning,

    #[error("Workflow failed: {0}")]
    Failed(String),

    #[error("Workflow was cancelled")]
    Cancelled,

    #[error("Timed out waiting for completion")]
    Timeout,

    #[error("Deserialization error: {0}")]
    DeserializationError(#[from] serde_json::Error),

    #[error("Client error: {0}")]
    ClientError(#[from] ClientError),
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowQueryError {
    #[error("Workflow not found: {0}")]
    NotFound(Uuid),

    #[error("Query not found: {0}")]
    QueryNotFound(String),

    #[error("Query execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Deserialization error: {0}")]
    DeserializationError(#[from] serde_json::Error),

    #[error("Client error: {0}")]
    ClientError(#[from] ClientError),
}
```

## Implementation Plan

### Phase 1: Core APIs

1. **WorkflowHandle implementation**
   - Create `WorkflowHandle` struct
   - Implement `status()`, `result()`, `cancel()`
   - Add `start_workflow_handle()` to `FlovynClient`

2. **Workflow cancellation**
   - Add gRPC call for cancellation
   - Handle cancellation in workflow executor
   - Add `isCancellationRequested()` to `WorkflowContext`

3. **Enhanced error types**
   - Create `WorkflowResultError`, `WorkflowQueryError`
   - Update existing methods to use proper error types

### Phase 2: Query System

1. **QueryableWorkflow trait**
   - Define trait and `QueryContext`
   - Implement built-in queries

2. **Query execution in worker**
   - Reconstruct state from events
   - Execute query handlers

### Phase 3: Task Streaming

1. **StreamEvent types**
   - Define event enum and serialization

2. **TaskContext streaming**
   - Add `stream()` method
   - Implement gRPC streaming call

3. **Client subscription**
   - Implement `subscribe_task_stream()`
   - Handle backpressure

### Phase 4: Worker Lifecycle

1. **Status tracking**
   - Track registration status
   - Expose via public API

2. **Graceful shutdown**
   - Implement `shutdown()` with drain period
   - Complete in-flight work before stopping

## API Examples

### Example 1: Start and Wait for Workflow

```rust
let client = FlovynClient::builder()
    .server_address("localhost", 9090)
    .org_id(org_id)
    .worker_token(token)
    .register_workflow(OrderWorkflow)
    .build()
    .await?;

// Option 1: Using start_workflow_and_wait (existing)
let result: OrderOutput = client
    .start_workflow_and_wait("order-workflow", json!({"order_id": "123"}), Duration::from_secs(30))
    .await?;

// Option 2: Using WorkflowHandle (new)
let handle = client
    .start_workflow_handle("order-workflow", json!({"order_id": "123"}))
    .await?;

// Check status
let status = handle.status().await?;
println!("Status: {:?}", status);

// Wait for result
let result: OrderOutput = handle.result(Duration::from_secs(30)).await?;
```

### Example 2: Query Workflow State

```rust
// Get handle to existing workflow
let handle = client.workflow(workflow_id);

// Query status (built-in)
let status: WorkflowStatus = handle.query("getStatus", json!({})).await?;

// Query custom state
let order_items: Vec<OrderItem> = handle.query("getOrderItems", json!({})).await?;
```

### Example 3: Cancel Workflow

```rust
let handle = client.workflow(workflow_id);

// Request cancellation
handle.cancel().await?;

// Workflow can check for cancellation
impl WorkflowDefinition for OrderWorkflow {
    async fn execute(&self, ctx: &WorkflowContext, input: Value) -> Result<Value, WorkflowError> {
        // Long-running work...

        // Check if cancellation was requested
        if ctx.is_cancellation_requested() {
            // Perform cleanup
            ctx.run("cleanup", || async {
                // Compensating actions
            }).await?;

            return Err(WorkflowError::Cancelled);
        }

        // Continue execution...
    }
}
```

### Example 4: Task Streaming

```rust
// In task implementation
impl TaskDefinition for LlmTask {
    async fn execute(&self, ctx: &TaskContext, input: Value) -> Result<Value, TaskError> {
        let prompt = input["prompt"].as_str().unwrap();

        ctx.stream(StreamEvent::Progress {
            progress: 0.0,
            details: Some("Starting LLM generation".into()),
        }).await?;

        // Stream tokens as they're generated
        for token in llm.generate_stream(prompt) {
            ctx.stream(StreamEvent::Token { text: token }).await?;
        }

        ctx.stream(StreamEvent::Progress {
            progress: 1.0,
            details: Some("Complete".into()),
        }).await?;

        Ok(json!({"status": "complete"}))
    }
}

// Client subscribing to stream
let mut stream = client.subscribe_task_stream(task_id);
while let Some(event) = stream.next().await {
    match event? {
        StreamEvent::Token { text } => print!("{}", text),
        StreamEvent::Progress { progress, details } => {
            println!("\nProgress: {:.0}% - {:?}", progress * 100.0, details);
        }
        _ => {}
    }
}
```

### Example 5: Queryable Workflow

```rust
#[derive(Clone)]
struct OrderWorkflow;

impl WorkflowDefinition for OrderWorkflow {
    const KIND: &'static str = "order-workflow";
    type Input = OrderInput;
    type Output = OrderOutput;

    async fn execute(&self, ctx: &WorkflowContext, input: OrderInput) -> Result<OrderOutput, WorkflowError> {
        ctx.set("status", "processing").await?;
        ctx.set("order_id", &input.order_id).await?;
        ctx.set("items", &input.items).await?;

        // ... workflow logic

        ctx.set("status", "completed").await?;
        Ok(OrderOutput { /* ... */ })
    }
}

impl QueryableWorkflow for OrderWorkflow {
    fn query(
        &self,
        ctx: &QueryContext,
        query_name: &str,
        _params: Value,
    ) -> Result<Value, QueryError> {
        match query_name {
            "getOrderItems" => {
                let items: Vec<OrderItem> = ctx.get("items").unwrap_or_default();
                Ok(serde_json::to_value(items)?)
            }
            "getOrderStatus" => {
                let status: String = ctx.get("status").unwrap_or_default();
                Ok(json!({ "status": status }))
            }
            _ => Err(QueryError::UnknownQuery(query_name.to_string())),
        }
    }
}
```

### Example 6: Worker Lifecycle

```rust
let client = FlovynClient::builder()
    .server_address("localhost", 9090)
    .org_id(org_id)
    .worker_token(token)
    .register_workflow(OrderWorkflow)
    .build()
    .await?;

// Start workers
let worker_handle = client.start().await?;

// Check registration status
match client.registration_status() {
    WorkerRegistrationStatus::Registered { worker_id, .. } => {
        println!("Registered with ID: {}", worker_id);
    }
    WorkerRegistrationStatus::Failed { error, .. } => {
        eprintln!("Registration failed: {}", error);
    }
    _ => {}
}

// Handle shutdown signal
tokio::signal::ctrl_c().await?;

// Graceful shutdown (waits for in-flight work)
client.shutdown().await?;
```

## E2E Test Plan

### Test Suite: `test_workflow_handle.rs`

```rust
#[tokio::test]
async fn test_workflow_handle_status() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("slow-workflow", json!({"delay_ms": 1000}))
        .await
        .unwrap();

    // Should be running initially
    let status = handle.status().await.unwrap();
    assert_eq!(status, WorkflowStatus::Running);

    // Wait for completion
    sleep(Duration::from_secs(2)).await;

    let status = handle.status().await.unwrap();
    assert_eq!(status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn test_workflow_handle_result() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("echo-workflow", json!({"message": "hello"}))
        .await
        .unwrap();

    let result: EchoOutput = handle.result(Duration::from_secs(5)).await.unwrap();
    assert_eq!(result.message, "hello");
}

#[tokio::test]
async fn test_workflow_handle_result_timeout() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("slow-workflow", json!({"delay_ms": 10000}))
        .await
        .unwrap();

    let result = handle.result::<Value>(Duration::from_millis(100)).await;
    assert!(matches!(result, Err(WorkflowResultError::Timeout)));
}

#[tokio::test]
async fn test_workflow_cancellation() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("cancellable-workflow", json!({}))
        .await
        .unwrap();

    // Cancel after brief delay
    sleep(Duration::from_millis(100)).await;
    handle.cancel().await.unwrap();

    // Should eventually show cancelled
    sleep(Duration::from_secs(1)).await;
    let status = handle.status().await.unwrap();
    assert_eq!(status, WorkflowStatus::Cancelled);
}
```

### Test Suite: `test_workflow_query.rs`

```rust
#[tokio::test]
async fn test_query_workflow_state() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("stateful-workflow", json!({"items": ["a", "b", "c"]}))
        .await
        .unwrap();

    // Wait for workflow to set state
    sleep(Duration::from_millis(500)).await;

    // Query custom state
    let items: Vec<String> = handle.query("getItems", json!({})).await.unwrap();
    assert_eq!(items, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn test_query_builtin_status() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("echo-workflow", json!({}))
        .await
        .unwrap();

    // Wait for completion
    let _: Value = handle.result(Duration::from_secs(5)).await.unwrap();

    // Query built-in status
    let status: WorkflowStatus = handle.query("getStatus", json!({})).await.unwrap();
    assert_eq!(status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn test_query_unknown_returns_error() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("echo-workflow", json!({}))
        .await
        .unwrap();

    let result = handle.query::<Value>("nonexistent", json!({})).await;
    assert!(matches!(result, Err(WorkflowQueryError::QueryNotFound(_))));
}
```

### Test Suite: `test_task_streaming.rs`

```rust
#[tokio::test]
async fn test_task_stream_tokens() {
    let client = setup_test_client().await;

    // Start workflow that runs streaming task
    let handle = client
        .start_workflow_handle("streaming-workflow", json!({}))
        .await
        .unwrap();

    // Get task execution ID from workflow state
    sleep(Duration::from_millis(200)).await;
    let task_id: Uuid = handle.query("getCurrentTaskId", json!({})).await.unwrap();

    // Subscribe to stream
    let stream = client.subscribe_task_stream(task_id);
    let events: Vec<StreamEvent> = stream.take(5).collect().await;

    // Should have received tokens
    assert!(events.iter().any(|e| matches!(e, StreamEvent::Token { .. })));
}

#[tokio::test]
async fn test_task_stream_progress() {
    let client = setup_test_client().await;

    let handle = client
        .start_workflow_handle("progress-workflow", json!({}))
        .await
        .unwrap();

    sleep(Duration::from_millis(200)).await;
    let task_id: Uuid = handle.query("getCurrentTaskId", json!({})).await.unwrap();

    let stream = client.subscribe_task_stream(task_id);
    let events: Vec<StreamEvent> = stream.collect().await;

    // Should have progress events from 0.0 to 1.0
    let progress_events: Vec<f64> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Progress { progress, .. } => Some(*progress),
            _ => None,
        })
        .collect();

    assert!(!progress_events.is_empty());
    assert!(progress_events.last().unwrap() >= &1.0);
}
```

### Test Suite: `test_worker_lifecycle.rs`

```rust
#[tokio::test]
async fn test_worker_registration_status() {
    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .org_id(test_org_id())
        .worker_token(test_token())
        .register_workflow(EchoWorkflow)
        .build()
        .await
        .unwrap();

    // Before start, should be unregistered
    assert!(matches!(
        client.registration_status(),
        WorkerRegistrationStatus::Unregistered
    ));

    // Start workers
    let _handle = client.start().await.unwrap();

    // After start, should be registered
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(matches!(
        client.registration_status(),
        WorkerRegistrationStatus::Registered { .. }
    ));
}

#[tokio::test]
async fn test_graceful_shutdown() {
    let client = setup_test_client().await;
    let _handle = client.start().await.unwrap();

    // Start a slow workflow
    client
        .start_workflow("slow-workflow", json!({"delay_ms": 2000}))
        .await
        .unwrap();

    // Wait for workflow to start processing
    sleep(Duration::from_millis(500)).await;

    // Request graceful shutdown
    client.shutdown().await.unwrap();

    // Workflow should complete despite shutdown
    // (verify via server-side check or event log)
}

#[tokio::test]
async fn test_is_running() {
    let client = setup_test_client().await;

    assert!(!client.is_running());

    let _handle = client.start().await.unwrap();
    assert!(client.is_running());

    client.stop();

    // Brief delay for shutdown
    sleep(Duration::from_millis(100)).await;
    assert!(!client.is_running());
}
```

## Example Applications

### Updated Hello World Example

```rust
// examples/hello-world/src/main.rs

use flovyn_sdk::prelude::*;

#[derive(Clone)]
struct GreetWorkflow;

impl WorkflowDefinition for GreetWorkflow {
    const KIND: &'static str = "greet";
    type Input = GreetInput;
    type Output = GreetOutput;

    async fn execute(
        &self,
        ctx: &WorkflowContext,
        input: GreetInput,
    ) -> Result<GreetOutput, WorkflowError> {
        ctx.set("status", "greeting").await?;
        ctx.set("name", &input.name).await?;

        let greeting = format!("Hello, {}!", input.name);

        ctx.set("status", "completed").await?;
        Ok(GreetOutput { greeting })
    }
}

impl QueryableWorkflow for GreetWorkflow {
    fn query(
        &self,
        ctx: &QueryContext,
        query_name: &str,
        _params: Value,
    ) -> Result<Value, QueryError> {
        match query_name {
            "getName" => {
                let name: String = ctx.get("name").unwrap_or_default();
                Ok(json!(name))
            }
            _ => Err(QueryError::UnknownQuery(query_name.to_string())),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .org_id(/* ... */)
        .worker_token(/* ... */)
        .register_workflow(GreetWorkflow)
        .build()
        .await?;

    let _worker = client.start().await?;

    // Start workflow and get handle
    let handle = client
        .start_workflow_handle("greet", json!({"name": "World"}))
        .await?;

    println!("Started workflow: {}", handle.id());

    // Query name while running
    let name: String = handle.query("getName", json!({})).await?;
    println!("Greeting: {}", name);

    // Wait for result
    let result: GreetOutput = handle.result(Duration::from_secs(10)).await?;
    println!("Result: {}", result.greeting);

    Ok(())
}
```

### Streaming LLM Example

```rust
// examples/llm-streaming/src/main.rs

use flovyn_sdk::prelude::*;
use futures::StreamExt;

#[derive(Clone)]
struct LlmTask;

impl TaskDefinition for LlmTask {
    const KIND: &'static str = "llm-generate";
    type Input = LlmInput;
    type Output = LlmOutput;

    async fn execute(
        &self,
        ctx: &TaskContext,
        input: LlmInput,
    ) -> Result<LlmOutput, TaskError> {
        ctx.stream(StreamEvent::Progress {
            progress: 0.0,
            details: Some("Starting generation".into()),
        }).await?;

        let mut full_text = String::new();

        // Simulate LLM token streaming
        for (i, word) in input.prompt.split_whitespace().enumerate() {
            ctx.stream(StreamEvent::Token {
                text: format!("{} ", word.to_uppercase()),
            }).await?;

            full_text.push_str(&format!("{} ", word.to_uppercase()));

            ctx.stream(StreamEvent::Progress {
                progress: (i as f64) / 10.0,
                details: None,
            }).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        ctx.stream(StreamEvent::Progress {
            progress: 1.0,
            details: Some("Complete".into()),
        }).await?;

        Ok(LlmOutput { text: full_text.trim().to_string() })
    }
}

#[derive(Clone)]
struct LlmWorkflow;

impl WorkflowDefinition for LlmWorkflow {
    const KIND: &'static str = "llm-workflow";
    type Input = LlmWorkflowInput;
    type Output = LlmWorkflowOutput;

    async fn execute(
        &self,
        ctx: &WorkflowContext,
        input: LlmWorkflowInput,
    ) -> Result<LlmWorkflowOutput, WorkflowError> {
        let task_id = ctx.schedule_task::<LlmTask>(LlmInput {
            prompt: input.prompt,
        }).await?;

        ctx.set("current_task_id", task_id).await?;

        let result: LlmOutput = ctx.await_task(task_id).await?;

        Ok(LlmWorkflowOutput { response: result.text })
    }
}

impl QueryableWorkflow for LlmWorkflow {
    fn query(
        &self,
        ctx: &QueryContext,
        query_name: &str,
        _params: Value,
    ) -> Result<Value, QueryError> {
        match query_name {
            "getCurrentTaskId" => {
                let id: Option<Uuid> = ctx.get("current_task_id");
                Ok(json!(id))
            }
            _ => Err(QueryError::UnknownQuery(query_name.to_string())),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .org_id(/* ... */)
        .worker_token(/* ... */)
        .register_workflow(LlmWorkflow)
        .register_task(LlmTask)
        .build()
        .await?;

    let _worker = client.start().await?;

    let handle = client
        .start_workflow_handle("llm-workflow", json!({"prompt": "hello world"}))
        .await?;

    // Wait for task to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Get task ID and subscribe to stream
    if let Some(task_id) = handle.query::<Option<Uuid>>("getCurrentTaskId", json!({})).await? {
        let mut stream = client.subscribe_task_stream(task_id);

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::Token { text } => print!("{}", text),
                StreamEvent::Progress { progress, details } => {
                    println!("\n[Progress: {:.0}%] {:?}", progress * 100.0, details);
                }
                _ => {}
            }
        }
    }

    let result: LlmWorkflowOutput = handle.result(Duration::from_secs(30)).await?;
    println!("\nFinal result: {}", result.response);

    Ok(())
}
```

## Dependencies

### Required gRPC Service Methods

The following gRPC methods need to be available on the server:

| Service | Method | Purpose |
|---------|--------|---------|
| `WorkflowDispatch` | `CancelWorkflow` | Cancel a running workflow |
| `WorkflowQuery` | `GetWorkflowStatus` | Get execution status |
| `WorkflowQuery` | `GetWorkflowResult` | Get completed result |
| `TaskExecution` | `StreamTaskData` | Subscribe to task events |

### Server Compatibility

This design requires server version X.Y.Z or later with the following features:
- Workflow cancellation support
- Status query endpoint
- Task streaming endpoint

## Alternatives Considered

### 1. Single `Workflow` Struct vs `WorkflowHandle`

**Alternative**: Return a `Workflow` struct from `start_workflow()` that automatically provides the handle.

**Decision**: Use separate `WorkflowHandle` type to:
- Keep `start_workflow()` backward-compatible (returns just UUID)
- Allow getting handles to existing workflows
- Clearer separation of concerns

### 2. Polling vs Server Push for Status Updates

**Alternative**: Use server-sent events or WebSocket for real-time status updates.

**Decision**: Start with polling via `status()` method because:
- Simpler implementation
- Works with existing gRPC infrastructure
- Can add push notifications later if needed

### 3. Trait-Based vs Method-Based Queries

**Alternative**: Define queries as methods on workflow structs.

**Decision**: Use `QueryableWorkflow` trait with string-based dispatch because:
- More flexible for dynamic queries
- Matches Kotlin SDK pattern
- Allows queries without knowing all query types at compile time

## Open Questions

1. **Streaming backpressure**: How should we handle slow consumers of task streams?

2. **Query caching**: Should query results be cached? For how long?

3. **Cancellation semantics**: Should cancellation be immediate (abort) or cooperative (request)?

4. **Worker re-registration**: How should we handle server restarts or network partitions?

## References

- Kotlin SDK implementation: `/Users/manhha/Developer/manhha/leanapp/flovyn/sdk/kotlin`
- gRPC service definitions: `sdk/proto/flovyn/v1/`
- Current Rust SDK: `sdk/src/client/`
