//! Promise E2E tests
//!
//! These tests verify workflow promise (external signal) functionality against a real server.

use crate::fixtures::workflows::PromiseWorkflow;
use crate::test_env::E2ETestEnvBuilder;
use crate::{with_timeout, TEST_TIMEOUT};
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
        let env = E2ETestEnvBuilder::new("e2e-promise-worker")
            .await
            .register_workflow(PromiseWorkflow)
            .build_and_start()
            .await;

        // Start the workflow (without waiting - it will suspend)
        let workflow_id = env
            .start_workflow(
                "promise-workflow",
                json!({
                    "promiseName": "user-approval"
                }),
            )
            .await;

        // Wait for workflow to suspend (should create the promise and wait)
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Resolve the promise externally
        env.client()
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
        let result = env.await_completion(workflow_id).await;
        env.assert_completed(&result);

        // Verify the output contains the promise value
        let output = result.output.as_ref().expect("Expected output");
        assert_eq!(output["promiseName"], json!("user-approval"));
        assert_eq!(output["promiseValue"]["approved"], json!(true));
        assert_eq!(
            output["promiseValue"]["approver"],
            json!("admin@example.com")
        );
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
        let env = E2ETestEnvBuilder::new("e2e-promise-reject-worker")
            .await
            .register_workflow(PromiseWorkflow)
            .build_and_start()
            .await;

        // Start the workflow
        let workflow_id = env
            .start_workflow(
                "promise-workflow",
                json!({
                    "promiseName": "approval"
                }),
            )
            .await;

        // Wait for workflow to suspend
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Reject the promise externally
        env.client()
            .reject_promise(workflow_id, "approval", "Request denied by admin")
            .await
            .expect("Failed to reject promise");

        // Wait for PROMISE_REJECTED or WORKFLOW_EXECUTION_FAILED event
        let timeout = Duration::from_secs(20);
        let rejected_event = env
            .await_event(workflow_id, "PROMISE_REJECTED", timeout)
            .await;
        let failed_event = env
            .await_event(workflow_id, "WORKFLOW_EXECUTION_FAILED", timeout)
            .await;

        // Verify we have a rejection/failure event
        assert!(
            rejected_event.is_some() || failed_event.is_some(),
            "Expected PROMISE_REJECTED or WORKFLOW_EXECUTION_FAILED event"
        );

        if let Some(event) = rejected_event {
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
    })
    .await;
}
