//! State Management E2E tests
//!
//! These tests verify workflow state management (set/get/clear) against a real server.

use crate::fixtures::workflows::StatefulWorkflow;
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::Duration;

/// Test state set and get operations.
/// The workflow sets a value and retrieves it back.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_state_set_get() {
    with_timeout(TEST_TIMEOUT, "test_state_set_get", async {
        let harness = get_harness().await;

        let task_queue = "state-tests-queue";
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-state-worker")
            .worker_token(harness.worker_token())
            .task_queue(task_queue)
            .register_workflow(StatefulWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_task_queue(task_queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "stateful-workflow",
                json!({
                    "key": "test-key",
                    "value": {"nested": "data", "count": 123}
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Verify stored and retrieved values match
        assert_eq!(result["stored"], json!({"nested": "data", "count": 123}));
        assert_eq!(result["retrieved"], json!({"nested": "data", "count": 123}));

        handle.abort();
    })
    .await;
}
