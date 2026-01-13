//! E2ETestEnvironment - Unified test environment for E2E tests
//!
//! Provides:
//! - Client builder with fluent workflow/task registration
//! - Worker lifecycle management (start, await_ready, shutdown)
//! - Workflow execution helpers (start_and_await, await_completion)
//! - Assertions (assert_completed, assert_failed, assert_output)
//!
//! Example usage:
//! ```rust,ignore
//! #[tokio::test]
//! async fn test_example() {
//!     let env = E2ETestEnvironment::new().await;
//!
//!     let result = env
//!         .register_workflow(DoublerWorkflow)
//!         .start_worker("test-worker")
//!         .await
//!         .start_and_await("doubler-workflow", json!({"value": 21}))
//!         .await
//!         .expect("Workflow failed");
//!
//!     env.assert_output(&result, "result", &json!(42));
//! }
//! ```

use crate::get_harness;
use crate::harness::TestHarness;
use flovyn_sdk::client::{
    FlovynClient, FlovynClientBuilder, StartWorkflowOptions, WorkerHandle, WorkflowEvent,
};
use flovyn_sdk::{TaskDefinition, WorkflowDefinition};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

/// Default timeout for awaiting workflow completion
pub const DEFAULT_AWAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default delay for worker registration
pub const WORKER_REGISTRATION_DELAY: Duration = Duration::from_secs(2);

/// Workflow completion status
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    pub workflow_id: Uuid,
    pub status: WorkflowStatus,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub events: Vec<WorkflowEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowStatus {
    Completed,
    Failed,
    Pending,
}

/// Unified test environment for E2E tests.
///
/// This struct wraps the test harness and provides a fluent API
/// for registering workflows/tasks, starting workers, and awaiting
/// workflow completion with assertions.
pub struct E2ETestEnvironment {
    harness: &'static TestHarness,
    client: Option<FlovynClient>,
    handle: Option<WorkerHandle>,
    queue: String,
}

impl E2ETestEnvironment {
    /// Create a new test environment using the global harness.
    pub async fn new() -> Self {
        Self::with_queue("default").await
    }

    /// Create a new test environment with a specific task queue.
    pub async fn with_queue(queue: &str) -> Self {
        let harness = get_harness().await;
        Self {
            harness,
            client: None,
            handle: None,
            queue: queue.to_string(),
        }
    }

    /// Get a client builder preconfigured with harness settings.
    pub fn client_builder(&self) -> FlovynClientBuilder {
        FlovynClient::builder()
            .server_address(self.harness.grpc_host(), self.harness.grpc_port())
            .org_id(self.harness.org_id())
            .worker_token(self.harness.worker_token())
            .queue(&self.queue)
    }

    /// Build and store the client. Returns self for chaining.
    pub async fn build_client(&mut self, builder: FlovynClientBuilder) -> &mut Self {
        self.client = Some(builder.build().await.expect("Failed to build FlovynClient"));
        self
    }

    /// Start the worker and wait for it to be ready.
    pub async fn start_worker(&mut self) -> &mut Self {
        let client = self
            .client
            .as_ref()
            .expect("Client not built. Call build_client first.");
        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the server time to process worker registration
        tokio::time::sleep(WORKER_REGISTRATION_DELAY).await;

        self.handle = Some(handle);
        self
    }

    /// Get a reference to the client.
    pub fn client(&self) -> &FlovynClient {
        self.client.as_ref().expect("Client not built")
    }

    /// Start a workflow without waiting for completion.
    pub async fn start_workflow(&self, kind: &str, input: Value) -> Uuid {
        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&self.queue);
        let result = self
            .client()
            .start_workflow_with_options(kind, input, options)
            .await
            .expect("Failed to start workflow");
        result.workflow_execution_id
    }

    /// Start a workflow and wait for it to complete (uses FlovynClient's built-in method).
    pub async fn start_and_await(&self, kind: &str, input: Value) -> Result<Value, String> {
        self.start_and_await_with_timeout(kind, input, DEFAULT_AWAIT_TIMEOUT)
            .await
    }

    /// Start a workflow and wait for completion with custom timeout.
    pub async fn start_and_await_with_timeout(
        &self,
        kind: &str,
        input: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&self.queue);
        self.client()
            .start_workflow_and_wait_with_options(kind, input, options, timeout)
            .await
            .map_err(|e| e.to_string())
    }

    /// Poll for workflow completion and return full result with events.
    pub async fn await_completion(&self, workflow_id: Uuid) -> WorkflowResult {
        self.await_completion_with_timeout(workflow_id, DEFAULT_AWAIT_TIMEOUT)
            .await
    }

    /// Poll for workflow completion with custom timeout.
    pub async fn await_completion_with_timeout(
        &self,
        workflow_id: Uuid,
        timeout: Duration,
    ) -> WorkflowResult {
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return WorkflowResult {
                    workflow_id,
                    status: WorkflowStatus::Pending,
                    output: None,
                    error: Some(format!("Timeout after {:?}", timeout)),
                    events: vec![],
                };
            }

            let events = self
                .client()
                .get_workflow_events(workflow_id)
                .await
                .unwrap_or_default();

            // Check for completion
            if let Some(event) = events.iter().find(|e| e.event_type == "WORKFLOW_COMPLETED") {
                return WorkflowResult {
                    workflow_id,
                    status: WorkflowStatus::Completed,
                    output: event.payload.get("output").cloned(),
                    error: None,
                    events,
                };
            }

            // Check for failure
            if let Some(event) = events
                .iter()
                .find(|e| e.event_type == "WORKFLOW_EXECUTION_FAILED")
            {
                let error = event
                    .payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .map(|s| s.to_string());
                return WorkflowResult {
                    workflow_id,
                    status: WorkflowStatus::Failed,
                    output: None,
                    error,
                    events,
                };
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Wait for a specific event type to appear.
    pub async fn await_event(
        &self,
        workflow_id: Uuid,
        event_type: &str,
        timeout: Duration,
    ) -> Option<WorkflowEvent> {
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return None;
            }

            let events = self
                .client()
                .get_workflow_events(workflow_id)
                .await
                .unwrap_or_default();

            if let Some(event) = events.iter().find(|e| e.event_type == event_type) {
                return Some(event.clone());
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    // ========== ASSERTIONS ==========

    /// Assert that a workflow result indicates completion.
    pub fn assert_completed(&self, result: &WorkflowResult) {
        if result.status == WorkflowStatus::Failed {
            panic!("Workflow failed: {:?}", result.error);
        }
        assert_eq!(
            result.status,
            WorkflowStatus::Completed,
            "Expected workflow to complete, but status was {:?}",
            result.status
        );
    }

    /// Assert that a workflow result indicates failure.
    pub fn assert_failed(&self, result: &WorkflowResult) {
        assert_eq!(
            result.status,
            WorkflowStatus::Failed,
            "Expected workflow to fail, but status was {:?}",
            result.status
        );
    }

    /// Assert that workflow output contains expected key-value.
    pub fn assert_output(&self, result: &WorkflowResult, key: &str, expected: &Value) {
        let output = result
            .output
            .as_ref()
            .expect("Workflow output should not be None");
        assert_eq!(
            output.get(key),
            Some(expected),
            "Output[{}] mismatch: expected {:?}, got {:?}",
            key,
            expected,
            output.get(key)
        );
    }

    /// Assert that workflow output equals expected value.
    pub fn assert_output_eq(&self, result: &WorkflowResult, expected: &Value) {
        let output = result
            .output
            .as_ref()
            .expect("Workflow output should not be None");
        assert_eq!(output, expected, "Output mismatch");
    }

    /// Assert that error contains expected substring.
    pub fn assert_error_contains(&self, result: &WorkflowResult, expected: &str) {
        let error = result.error.as_ref().expect("Expected error message");
        assert!(
            error.contains(expected),
            "Expected error to contain '{}', got: {}",
            expected,
            error
        );
    }
}

impl Drop for E2ETestEnvironment {
    fn drop(&mut self) {
        // Abort the worker handle if it exists
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Builder for E2ETestEnvironment with fluent API for workflow/task registration.
pub struct E2ETestEnvBuilder {
    harness: &'static TestHarness,
    builder: FlovynClientBuilder,
    queue: String,
    /// Test prefix for generating unique kinds
    test_prefix: String,
}

impl E2ETestEnvBuilder {
    /// Create a new builder from the global harness.
    pub async fn new(worker_id: &str) -> Self {
        Self::with_queue(worker_id, "default").await
    }

    /// Create a new builder with a specific task queue.
    pub async fn with_queue(worker_id: &str, queue: &str) -> Self {
        let harness = get_harness().await;
        let builder = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id(worker_id)
            .queue(queue);

        Self {
            harness,
            builder,
            queue: queue.to_string(),
            test_prefix: String::new(),
        }
    }

    /// Create a new builder with auto-generated unique queue and test prefix.
    /// This ensures test isolation when running tests in parallel.
    pub async fn unique(test_name: &str) -> Self {
        let harness = get_harness().await;
        let test_id = uuid::Uuid::new_v4();
        let test_prefix = format!("{}:{}", test_name, test_id);
        let queue = format!("q:{}", test_prefix);
        let worker_id = format!("worker:{}", test_prefix);

        let builder = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id(&worker_id)
            .queue(&queue);

        Self {
            harness,
            builder,
            queue,
            test_prefix,
        }
    }

    /// Get the test prefix for generating unique kinds.
    /// Use this with workflow/task input to coordinate kinds.
    pub fn test_prefix(&self) -> &str {
        &self.test_prefix
    }

    /// Generate a unique workflow kind using the test prefix.
    pub fn workflow_kind(&self, base_kind: &str) -> String {
        if self.test_prefix.is_empty() {
            base_kind.to_string()
        } else {
            format!("{}:{}", base_kind, self.test_prefix)
        }
    }

    /// Generate a unique task kind using the test prefix.
    pub fn task_kind(&self, base_kind: &str) -> String {
        if self.test_prefix.is_empty() {
            base_kind.to_string()
        } else {
            format!("{}:{}", base_kind, self.test_prefix)
        }
    }

    /// Register a typed workflow definition.
    pub fn register_workflow<W, I, O>(mut self, workflow: W) -> Self
    where
        W: WorkflowDefinition<Input = I, Output = O> + 'static,
        I: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
        O: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
    {
        self.builder = self.builder.register_workflow(workflow);
        self
    }

    /// Register a typed task definition.
    pub fn register_task<T, I, O>(mut self, task: T) -> Self
    where
        T: TaskDefinition<Input = I, Output = O> + 'static,
        I: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
        O: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
    {
        self.builder = self.builder.register_task(task);
        self
    }

    /// Build the client and start the worker.
    pub async fn build_and_start(self) -> E2ETestEnvironment {
        let client = self
            .builder
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the server time to process worker registration
        tokio::time::sleep(WORKER_REGISTRATION_DELAY).await;

        E2ETestEnvironment {
            harness: self.harness,
            client: Some(client),
            handle: Some(handle),
            queue: self.queue,
        }
    }
}
