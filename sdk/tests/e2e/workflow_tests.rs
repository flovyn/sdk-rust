//! Workflow E2E tests
//!
//! These tests verify workflow execution against a real Flovyn server.
//! All tests share a single TestHarness instance to avoid starting multiple containers.

use crate::fixtures::workflows::{DoublerWorkflow, EchoWorkflow, FailingWorkflow};
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::Duration;

/// Test that the test harness can start all containers and create a tenant.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_harness_setup() {
    with_timeout(TEST_TIMEOUT, "test_harness_setup", async {
        let harness = get_harness().await;

        assert!(!harness.tenant_id().is_nil());
        assert!(!harness.tenant_slug().is_empty());
        assert!(harness.worker_token().starts_with("fwt_"));
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
        let harness = get_harness().await;

        // Build SDK client with tenant from harness
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-test-worker")
            .worker_token(harness.worker_token())
            .register_workflow(DoublerWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        // Start the worker and wait for it to be ready
        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the server time to process worker registration
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Execute workflow and wait for completion
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_and_wait_with_options(
                "doubler-workflow",
                json!({"value": 21}),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Verify the result
        assert_eq!(result["result"], 42);

        // Clean up
        handle.abort();
    })
    .await;
}

/// Test echo workflow that returns its input unchanged.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_echo_workflow() {
    with_timeout(TEST_TIMEOUT, "test_echo_workflow", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-echo-worker")
            .worker_token(harness.worker_token())
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the server time to process worker registration
        tokio::time::sleep(Duration::from_secs(2)).await;

        let input = json!({
            "message": "Hello, World!",
            "count": 42,
            "nested": {
                "key": "value"
            }
        });

        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_and_wait_with_options(
                "echo-workflow",
                input.clone(),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Echo should return the same input
        assert_eq!(result, input);

        handle.abort();
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
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-failing-worker")
            .worker_token(harness.worker_token())
            .register_workflow(FailingWorkflow::new("Test error message"))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the server time to process worker registration
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start the workflow (without waiting for completion, since it will retry)
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_with_options("failing-workflow", json!({}), options)
            .await
            .expect("Failed to start workflow");

        let workflow_id = result.workflow_execution_id;

        // Poll for a WORKFLOW_EXECUTION_FAILED event (the workflow will be retried,
        // but we just need to verify it fails at least once with our error message)
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(10);

        loop {
            if start.elapsed() > timeout {
                panic!("Workflow failure event not found within timeout");
            }

            let events = client
                .get_workflow_events(workflow_id)
                .await
                .expect("Failed to get events");

            let failure_event = events
                .iter()
                .find(|e| e.event_type == "WORKFLOW_EXECUTION_FAILED");
            if let Some(event) = failure_event {
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
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        handle.abort();
    })
    .await;
}

/// Test starting a workflow without waiting for completion.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_start_workflow_async() {
    with_timeout(TEST_TIMEOUT, "test_start_workflow_async", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-async-worker")
            .worker_token(harness.worker_token())
            .register_workflow(DoublerWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the server time to process worker registration
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start workflow without waiting
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_with_options("doubler-workflow", json!({"value": 100}), options)
            .await
            .expect("Failed to start workflow");
        let workflow_id = result.workflow_execution_id;

        // Verify we got a valid workflow execution ID
        assert!(!workflow_id.is_nil());

        // Now wait for completion by polling events
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(30);

        loop {
            if start.elapsed() > timeout {
                panic!("Workflow did not complete within timeout");
            }

            let events = client
                .get_workflow_events(workflow_id)
                .await
                .expect("Failed to get events");

            let completed = events
                .iter()
                .any(|e| e.event_type == "WORKFLOW_COMPLETED");
            if completed {
                // Find the completion event and verify output
                let completion_event = events
                    .iter()
                    .find(|e| e.event_type == "WORKFLOW_COMPLETED")
                    .expect("Completion event not found");

                if let Some(output) = completion_event.payload.get("output") {
                    assert_eq!(output["result"], 200);
                }
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        handle.abort();
    })
    .await;
}
