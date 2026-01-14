//! Workflow E2E tests
//!
//! These tests verify workflow execution against a real Flovyn server.
//! All tests share a single TestHarness instance to avoid starting multiple containers.

use crate::fixtures::workflows::{DoublerWorkflow, EchoWorkflow, FailingWorkflow};
use crate::test_env::E2ETestEnvBuilder;
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use serde_json::json;
use std::time::Duration;

/// Test that the test harness can start all containers and create an org.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_harness_setup() {
    with_timeout(TEST_TIMEOUT, "test_harness_setup", async {
        let harness = get_harness().await;

        assert!(!harness.org_id().is_nil());
        assert!(!harness.org_slug().is_empty());
        assert!(!harness.worker_token().is_empty());
        assert!(harness.grpc_port() > 0);
        assert!(harness.http_port() > 0);
    })
    .await;
}

/// Test simple workflow execution with DoublerWorkflow.
/// The workflow doubles a numeric value.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_simple_workflow_execution() {
    with_timeout(TEST_TIMEOUT, "test_simple_workflow_execution", async {
        let env = E2ETestEnvBuilder::with_queue("e2e-test-worker", "workflow-simple-queue")
            .await
            .register_workflow(DoublerWorkflow)
            .build_and_start()
            .await;

        let result = env
            .start_and_await("doubler-workflow", json!({"value": 21}))
            .await
            .expect("Workflow execution failed");

        assert_eq!(result["result"], 42);
    })
    .await;
}

/// Test echo workflow that returns its input unchanged.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_echo_workflow() {
    with_timeout(TEST_TIMEOUT, "test_echo_workflow", async {
        let env = E2ETestEnvBuilder::with_queue("e2e-echo-worker", "workflow-echo-queue")
            .await
            .register_workflow(EchoWorkflow)
            .build_and_start()
            .await;

        let input = json!({
            "message": "Hello, World!",
            "count": 42,
            "nested": {
                "key": "value"
            }
        });

        let result = env
            .start_and_await("echo-workflow", input.clone())
            .await
            .expect("Workflow execution failed");

        assert_eq!(result, input);
    })
    .await;
}

/// Test that failing workflows produce failure events correctly.
/// Note: The server retries failed workflows, so we check for the failure event
/// rather than waiting for permanent failure.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_failing_workflow() {
    with_timeout(TEST_TIMEOUT, "test_failing_workflow", async {
        let env = E2ETestEnvBuilder::with_queue("e2e-failing-worker", "workflow-failing-queue")
            .await
            .register_workflow(FailingWorkflow::new("Test error message"))
            .build_and_start()
            .await;

        // Start the workflow without waiting (since it will be retried)
        let workflow_id = env.start_workflow("failing-workflow", json!({})).await;

        // Wait for a WORKFLOW_EXECUTION_FAILED event
        let event = env
            .await_event(
                workflow_id,
                "WORKFLOW_EXECUTION_FAILED",
                Duration::from_secs(10),
            )
            .await
            .expect("Workflow failure event not found within timeout");

        // Verify the error message is in the failure event
        let error_msg = event
            .payload
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("");
        assert!(
            error_msg.contains("Test error message"),
            "Expected error to contain 'Test error message', got: {}",
            error_msg
        );
    })
    .await;
}

/// Test starting a workflow without waiting for completion.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_start_workflow_async() {
    with_timeout(TEST_TIMEOUT, "test_start_workflow_async", async {
        let env = E2ETestEnvBuilder::with_queue("e2e-async-worker", "workflow-async-queue")
            .await
            .register_workflow(DoublerWorkflow)
            .build_and_start()
            .await;

        // Start workflow without waiting
        let workflow_id = env
            .start_workflow("doubler-workflow", json!({"value": 100}))
            .await;

        // Verify we got a valid workflow execution ID
        assert!(!workflow_id.is_nil());

        // Wait for completion using the helper
        let result = env.await_completion(workflow_id).await;
        env.assert_completed(&result);
        env.assert_output(&result, "result", &json!(200));
    })
    .await;
}
