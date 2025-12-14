//! Error Handling E2E tests
//!
//! These tests verify workflow error handling and failure scenarios against a real server.

use crate::fixtures::workflows::FailingWorkflow;
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::Duration;

/// Test workflow failure: workflow throws error, marked as failed.
///
/// This tests that when a workflow returns an error, it's properly:
/// 1. Recorded as a WORKFLOW_EXECUTION_FAILED event
/// 2. Contains the error message
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_workflow_failure() {
    with_timeout(TEST_TIMEOUT, "test_workflow_failure", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-error-worker")
            .worker_token(harness.worker_token())
            .register_workflow(FailingWorkflow::new("Intentional test failure"))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start the failing workflow
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_with_options("failing-workflow", json!({}), options)
            .await
            .expect("Failed to start workflow");

        let workflow_id = result.workflow_execution_id;

        // Wait for the workflow to fail
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(20);

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
                // Verify the error information is present
                assert!(
                    event.payload.get("error").is_some(),
                    "Expected error in failure event"
                );
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        handle.abort();
    })
    .await;
}

/// Test error message preserved: verify error message is preserved in failure event.
///
/// This tests that specific error messages from workflows are:
/// 1. Preserved in the failure event
/// 2. Accessible to clients querying workflow status
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_error_message_preserved() {
    with_timeout(TEST_TIMEOUT, "test_error_message_preserved", async {
        let harness = get_harness().await;

        let specific_error = "Custom error message with specific details XYZ-123";

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-error-message-worker")
            .worker_token(harness.worker_token())
            .register_workflow(FailingWorkflow::new(specific_error))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start the failing workflow
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_with_options("failing-workflow", json!({}), options)
            .await
            .expect("Failed to start workflow");

        let workflow_id = result.workflow_execution_id;

        // Wait for the workflow to fail and check error message
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(20);

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
                let error_msg = event
                    .payload
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("");

                // Verify the specific error message is preserved
                assert!(
                    error_msg.contains("XYZ-123"),
                    "Expected error message to contain 'XYZ-123', got: {}",
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
