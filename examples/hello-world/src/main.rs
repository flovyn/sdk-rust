//! Hello World Sample
//!
//! This is a minimal example demonstrating the basic usage of the Flovyn Rust SDK.
//! It shows how to:
//! - Define a simple workflow
//! - Use deterministic timestamps
//! - Execute side effects with `run_raw`
//! - Start a worker

use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::info;

/// Input for the greeting workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GreetingInput {
    pub name: String,
}

/// Output from the greeting workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GreetingOutput {
    pub message: String,
    pub timestamp: i64,
}

/// A simple greeting workflow that demonstrates basic SDK features
pub struct GreetingWorkflow;

#[async_trait]
impl WorkflowDefinition for GreetingWorkflow {
    type Input = GreetingInput;
    type Output = GreetingOutput;

    fn kind(&self) -> &str {
        "greeting-workflow"
    }

    fn name(&self) -> &str {
        "Greeting Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("A simple workflow that greets users by name")
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(name = %input.name, "Starting greeting workflow");

        // Get deterministic timestamp (same on replay)
        let timestamp = ctx.current_time_millis();

        // Execute a side effect - the result is cached for replay
        // On subsequent replays, this won't re-execute but return the cached result
        let greeting = format!("Hello, {}!", input.name);
        let greeting_value = serde_json::to_value(&greeting).map_err(FlovynError::Serialization)?;

        let result = ctx.run_raw("create-greeting", greeting_value).await?;
        let message: String = serde_json::from_value(result).map_err(FlovynError::Serialization)?;

        info!(message = %message, timestamp = %timestamp, "Greeting created");

        Ok(GreetingOutput { message, timestamp })
    }
}

/// A simple task that demonstrates task definition
pub struct EchoTask;

#[async_trait]
impl TaskDefinition for EchoTask {
    type Input = serde_json::Map<String, serde_json::Value>;
    type Output = serde_json::Map<String, serde_json::Value>;

    fn kind(&self) -> &str {
        "echo-task"
    }

    fn name(&self) -> &str {
        "Echo Task"
    }

    fn description(&self) -> Option<&str> {
        Some("Echoes the input back with a processed flag")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(30)
    }

    async fn execute(&self, mut input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        info!(task_id = %ctx.task_execution_id(), "Executing echo task");

        // Report progress
        ctx.report_progress(0.5, Some("Processing input")).await?;

        // Add processed flag
        input.insert("processed".to_string(), serde_json::Value::Bool(true));
        input.insert(
            "processed_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );

        ctx.report_progress(1.0, Some("Complete")).await?;

        Ok(input)
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("hello_world_sample=info".parse()?)
                .add_directive("flovyn_sdk=info".parse()?),
        )
        .init();

    info!("Starting Hello World Sample");

    // Parse tenant ID from environment or use default
    let tenant_id = std::env::var("FLOVYN_TENANT_ID")
        .ok()
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .unwrap_or_else(uuid::Uuid::new_v4);

    let server_host =
        std::env::var("FLOVYN_SERVER_HOST").unwrap_or_else(|_| "localhost".to_string());
    let server_port: u16 = std::env::var("FLOVYN_SERVER_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9090);
    let worker_token = std::env::var("FLOVYN_WORKER_TOKEN")
        .expect("FLOVYN_WORKER_TOKEN environment variable is required");

    info!(
        tenant_id = %tenant_id,
        server = %format!("{}:{}", server_host, server_port),
        "Connecting to Flovyn server"
    );

    // Build the client with fluent registration
    let client = FlovynClient::builder()
        .server_address(&server_host, server_port)
        .tenant_id(tenant_id)
        .worker_token(worker_token)
        .task_queue("hello-world")
        .register_workflow(GreetingWorkflow)
        .register_task(EchoTask)
        .build()
        .await?;

    info!("Client built successfully, starting workers...");

    // Start the workers
    let handle = client.start().await?;

    info!("Workers started. Press Ctrl+C to stop.");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    info!("Shutting down...");
    handle.stop().await;

    info!("Goodbye!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greeting_input_serialization() {
        let input = GreetingInput {
            name: "World".to_string(),
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("World"));
    }

    #[test]
    fn test_greeting_output_serialization() {
        let output = GreetingOutput {
            message: "Hello, World!".to_string(),
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("Hello, World!"));
        assert!(json.contains("1234567890"));
    }

    #[test]
    fn test_workflow_kind() {
        let workflow = GreetingWorkflow;
        assert_eq!(workflow.kind(), "greeting-workflow");
    }

    #[test]
    fn test_workflow_version() {
        let workflow = GreetingWorkflow;
        assert_eq!(workflow.version(), SemanticVersion::new(1, 0, 0));
    }

    #[test]
    fn test_task_kind() {
        let task = EchoTask;
        assert_eq!(task.kind(), "echo-task");
    }
}

/// Integration tests using SDK testing utilities
#[cfg(test)]
mod integration_tests {
    use super::*;
    use flovyn_sdk::testing::{MockTaskContext, MockWorkflowContext};

    #[tokio::test]
    async fn test_greeting_workflow_execution() {
        // Create a mock context with a specific timestamp
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1700000000000) // Fixed timestamp for testing
            .build();

        // Execute the workflow
        let input = GreetingInput {
            name: "Alice".to_string(),
        };
        let workflow = GreetingWorkflow;
        let result = workflow.execute(&ctx, input).await;

        // Verify the result
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.message, "Hello, Alice!");
        assert_eq!(output.timestamp, 1700000000000);

        // Verify that run_raw was called for create-greeting
        assert!(ctx.was_operation_recorded("create-greeting"));
    }

    #[tokio::test]
    async fn test_greeting_workflow_with_different_names() {
        let test_cases = vec![
            ("Bob", "Hello, Bob!"),
            ("Charlie", "Hello, Charlie!"),
            ("", "Hello, !"),
        ];

        for (name, expected_message) in test_cases {
            let ctx = MockWorkflowContext::builder()
                .initial_time_millis(1000)
                .build();

            let input = GreetingInput {
                name: name.to_string(),
            };
            let workflow = GreetingWorkflow;
            let result = workflow.execute(&ctx, input).await.unwrap();

            assert_eq!(result.message, expected_message);
        }
    }

    #[tokio::test]
    async fn test_echo_task_execution() {
        // Create a mock task context
        let ctx = MockTaskContext::builder().attempt(1).build();

        // Create input
        let mut input = serde_json::Map::new();
        input.insert(
            "key".to_string(),
            serde_json::Value::String("value".to_string()),
        );

        // Execute the task
        let task = EchoTask;
        let result = task.execute(input, &ctx).await;

        // Verify the result
        assert!(result.is_ok());
        let output = result.unwrap();

        // Check that processed flag was added
        assert_eq!(
            output.get("processed"),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(output.contains_key("processed_at"));
        // Original key should still be there
        assert_eq!(
            output.get("key"),
            Some(&serde_json::Value::String("value".to_string()))
        );

        // Verify progress was reported
        let reports = ctx.progress_reports();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].progress, 0.5);
        assert_eq!(reports[0].message, Some("Processing input".to_string()));
        assert_eq!(reports[1].progress, 1.0);
        assert_eq!(reports[1].message, Some("Complete".to_string()));
    }

    #[tokio::test]
    async fn test_echo_task_preserves_all_input_keys() {
        let ctx = MockTaskContext::new();

        let mut input = serde_json::Map::new();
        input.insert("a".to_string(), serde_json::json!(1));
        input.insert("b".to_string(), serde_json::json!("two"));
        input.insert("c".to_string(), serde_json::json!([1, 2, 3]));

        let task = EchoTask;
        let output = task.execute(input, &ctx).await.unwrap();

        // Original keys preserved
        assert_eq!(output.get("a"), Some(&serde_json::json!(1)));
        assert_eq!(output.get("b"), Some(&serde_json::json!("two")));
        assert_eq!(output.get("c"), Some(&serde_json::json!([1, 2, 3])));
        // Plus new keys
        assert!(output.contains_key("processed"));
        assert!(output.contains_key("processed_at"));
    }

    #[tokio::test]
    async fn test_echo_task_retry_attempt() {
        // Simulate a retry (attempt 3)
        let ctx = MockTaskContext::builder().attempt(3).build();

        let input = serde_json::Map::new();
        let task = EchoTask;
        let result = task.execute(input, &ctx).await;

        assert!(result.is_ok());
        assert_eq!(ctx.attempt(), 3);
    }
}
