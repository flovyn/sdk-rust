//! Promise E2E tests
//!
//! These tests verify workflow promise (external signal) functionality against a real server.

use crate::fixtures::workflows::PromiseWorkflow;
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::Duration;

/// Test promise resolve: workflow waits for promise, external resolve resumes it.
///
/// This tests the flow:
/// 1. Start workflow that calls ctx.promise("approval")
/// 2. Workflow suspends waiting for promise
/// 3. External call to client.resolve_promise() resolves the promise
/// 4. Workflow resumes and completes with the promise value
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_promise_resolve() {
    with_timeout(TEST_TIMEOUT, "test_promise_resolve", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-promise-worker")
            .worker_token(harness.worker_token())
            .register_workflow(PromiseWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the worker time to register
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start the workflow (without waiting - it will suspend)
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_with_options(
                "promise-workflow",
                json!({
                    "promiseName": "user-approval"
                }),
                options,
            )
            .await
            .expect("Failed to start workflow");

        let workflow_id = result.workflow_execution_id;

        // Wait for workflow to suspend (should create the promise and wait)
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Resolve the promise externally
        client
            .resolve_promise(
                workflow_id,
                "user-approval",
                json!({
                    "approved": true,
                    "approver": "admin@example.com"
                }),
            )
            .await
            .expect("Failed to resolve promise");

        // Wait for workflow to complete
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(20);

        loop {
            if start.elapsed() > timeout {
                panic!("Workflow did not complete after promise resolution");
            }

            let events = client
                .get_workflow_events(workflow_id)
                .await
                .expect("Failed to get events");

            let completed = events
                .iter()
                .find(|e| e.event_type == "WORKFLOW_COMPLETED");

            if let Some(event) = completed {
                // Verify the output contains the promise value
                if let Some(output) = event.payload.get("output") {
                    assert_eq!(output["promiseName"], json!("user-approval"));
                    assert_eq!(output["promiseValue"]["approved"], json!(true));
                    assert_eq!(output["promiseValue"]["approver"], json!("admin@example.com"));
                }
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        handle.abort();
    })
    .await;
}

/// Test promise reject: workflow waits for promise, external reject causes error.
///
/// This tests the flow:
/// 1. Start workflow that calls ctx.promise("approval")
/// 2. Workflow suspends waiting for promise
/// 3. External call to client.reject_promise() rejects the promise
/// 4. Workflow receives error from promise and fails
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_promise_reject() {
    with_timeout(TEST_TIMEOUT, "test_promise_reject", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-promise-reject-worker")
            .worker_token(harness.worker_token())
            .register_workflow(PromiseWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start the workflow
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_with_options(
                "promise-workflow",
                json!({
                    "promiseName": "approval"
                }),
                options,
            )
            .await
            .expect("Failed to start workflow");

        let workflow_id = result.workflow_execution_id;

        // Wait for workflow to suspend
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Reject the promise externally
        client
            .reject_promise(
                workflow_id,
                "approval",
                "Request denied by admin",
            )
            .await
            .expect("Failed to reject promise");

        // Wait for workflow to fail (or handle the rejection)
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(20);

        loop {
            if start.elapsed() > timeout {
                panic!("Workflow did not respond to promise rejection");
            }

            let events = client
                .get_workflow_events(workflow_id)
                .await
                .expect("Failed to get events");

            // Check for either WORKFLOW_EXECUTION_FAILED or PROMISE_REJECTED event
            let rejected = events
                .iter()
                .find(|e| e.event_type == "PROMISE_REJECTED" || e.event_type == "WORKFLOW_EXECUTION_FAILED");

            if let Some(event) = rejected {
                // Verify we have a rejection/failure event
                if event.event_type == "PROMISE_REJECTED" {
                    let error = event
                        .payload
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("");
                    assert!(
                        error.contains("denied") || error.contains("admin"),
                        "Expected rejection message, got: {}",
                        error
                    );
                }
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        handle.abort();
    })
    .await;
}
