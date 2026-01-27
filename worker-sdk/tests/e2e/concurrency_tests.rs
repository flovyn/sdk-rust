//! Concurrency E2E tests
//!
//! These tests verify concurrent workflow execution against a real server.

use crate::fixtures::workflows::DoublerWorkflow;
use crate::{get_harness, with_timeout};
use flovyn_worker_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::Duration;

/// Test concurrent workflow execution: multiple workflows executing simultaneously.
///
/// This tests that the server can handle multiple workflows being:
/// 1. Started concurrently
/// 2. Executed in parallel by workers
/// 3. Completed independently
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_concurrent_workflow_execution() {
    // Longer timeout for concurrent tests
    let test_timeout = Duration::from_secs(90);
    with_timeout(test_timeout, "test_concurrent_workflow_execution", async {
        let harness = get_harness().await;

        let queue = "concurrency-exec-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-concurrent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(DoublerWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start multiple workflows concurrently
        let num_workflows = 5;
        let mut workflow_ids = Vec::new();

        for i in 0..num_workflows {
            let options = StartWorkflowOptions::new()
                .with_workflow_version("1.0.0")
                .with_queue(queue);
            let result = client
                .start_workflow_with_options("doubler-workflow", json!({ "value": i }), options)
                .await
                .expect("Failed to start workflow");
            workflow_ids.push((i, result.workflow_execution_id));
        }

        // Wait for all workflows to complete
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(60);
        let mut completed = vec![false; num_workflows as usize];

        loop {
            if start.elapsed() > timeout {
                let pending: Vec<_> = completed
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !*c)
                    .map(|(i, _)| i)
                    .collect();
                panic!(
                    "Workflows did not complete within timeout. Pending: {:?}",
                    pending
                );
            }

            for (idx, (input_value, workflow_id)) in workflow_ids.iter().enumerate() {
                if completed[idx] {
                    continue;
                }

                let events = client
                    .get_workflow_events(*workflow_id)
                    .await
                    .expect("Failed to get events");

                let completion = events.iter().find(|e| e.event_type == "WORKFLOW_COMPLETED");

                if let Some(event) = completion {
                    // Verify result is correct (input * 2)
                    if let Some(output) = event.payload.get("output") {
                        let expected = *input_value * 2;
                        assert_eq!(
                            output["result"],
                            json!(expected),
                            "Workflow {} should have result {}",
                            workflow_id,
                            expected
                        );
                    }
                    completed[idx] = true;
                }
            }

            if completed.iter().all(|c| *c) {
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Verify all workflows completed
        assert_eq!(
            completed.iter().filter(|c| **c).count(),
            num_workflows as usize
        );

        handle.abort();
    })
    .await;
}

/// Test multiple workers: two workers polling the same queue.
///
/// This tests that multiple workers can:
/// 1. Connect to the same task queue
/// 2. Share work between them
/// 3. Complete workflows correctly
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_multiple_workers() {
    // Longer timeout for multi-worker tests
    let test_timeout = Duration::from_secs(120);
    with_timeout(test_timeout, "test_multiple_workers", async {
        let harness = get_harness().await;

        let queue = "concurrency-multi-worker-queue";

        // Create first worker
        let client1 = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-multi-worker-1")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(DoublerWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient 1");

        // Create second worker (same workflow, same queue)
        let client2 = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-multi-worker-2")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(DoublerWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient 2");

        // Start both workers
        let handle1 = client1.start().await.expect("Failed to start worker 1");
        let handle2 = client2.start().await.expect("Failed to start worker 2");

        handle1.await_ready().await;
        handle2.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start multiple workflows
        let num_workflows = 6;
        let mut workflow_ids = Vec::new();

        for i in 0..num_workflows {
            let options = StartWorkflowOptions::new()
                .with_workflow_version("1.0.0")
                .with_queue(queue);
            // Use client1 to start all workflows (they go to the shared queue)
            let result = client1
                .start_workflow_with_options(
                    "doubler-workflow",
                    json!({ "value": i * 10 }),
                    options,
                )
                .await
                .expect("Failed to start workflow");
            workflow_ids.push((i * 10, result.workflow_execution_id));
        }

        // Wait for all workflows to complete
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(60);
        let mut completed = vec![false; num_workflows as usize];

        loop {
            if start.elapsed() > timeout {
                let pending: Vec<_> = completed
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !*c)
                    .map(|(i, _)| i)
                    .collect();
                panic!(
                    "Workflows did not complete within timeout. Pending: {:?}",
                    pending
                );
            }

            for (idx, (input_value, workflow_id)) in workflow_ids.iter().enumerate() {
                if completed[idx] {
                    continue;
                }

                let events = client1
                    .get_workflow_events(*workflow_id)
                    .await
                    .expect("Failed to get events");

                let completion = events.iter().find(|e| e.event_type == "WORKFLOW_COMPLETED");

                if let Some(event) = completion {
                    if let Some(output) = event.payload.get("output") {
                        let expected = *input_value * 2;
                        assert_eq!(
                            output["result"],
                            json!(expected),
                            "Workflow {} should have result {}",
                            workflow_id,
                            expected
                        );
                    }
                    completed[idx] = true;
                }
            }

            if completed.iter().all(|c| *c) {
                break;
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Verify all workflows completed
        assert_eq!(
            completed.iter().filter(|c| **c).count(),
            num_workflows as usize
        );

        handle1.abort();
        handle2.abort();
    })
    .await;
}
