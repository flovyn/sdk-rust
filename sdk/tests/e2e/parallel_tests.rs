//! Parallel Execution E2E tests
//!
//! These tests verify parallel execution patterns against a real Flovyn server.
//! Tests the parallel combinator patterns: join_all, select, with_timeout.

use crate::fixtures::tasks::{FetchDataTask, ProcessItemTask, SlowOperationTask};
use crate::fixtures::workflows::{
    FanOutFanInWorkflow, MixedParallelWorkflow, ParallelTasksWorkflow, RacingTasksWorkflow,
    TimeoutTaskWorkflow,
};
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::Duration;

/// Test basic parallel task scheduling with join_all.
/// Schedules 3 tasks in parallel and waits for all to complete.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_e2e_parallel_tasks_join_all() {
    with_timeout(TEST_TIMEOUT, "test_e2e_parallel_tasks_join_all", async {
        let harness = get_harness().await;

        let queue = "parallel-join-all-queue";
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-parallel-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(ParallelTasksWorkflow)
            .register_task(ProcessItemTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the worker time to register
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start a workflow that schedules 3 tasks in parallel
        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "parallel-tasks-workflow",
                json!({
                    "items": ["a", "b", "c"]
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Verify all 3 items were processed
        assert_eq!(result.get("itemCount"), Some(&json!(3)));
        let results = result
            .get("results")
            .and_then(|v| v.as_array())
            .expect("results should be an array");
        assert_eq!(results.len(), 3);

        // Verify each item was processed
        assert!(results.iter().any(|r| r == &json!("processed:a")));
        assert!(results.iter().any(|r| r == &json!("processed:b")));
        assert!(results.iter().any(|r| r == &json!("processed:c")));

        handle.abort();
    })
    .await;
}

/// Test fan-out/fan-in pattern with result aggregation.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_e2e_fan_out_fan_in() {
    with_timeout(TEST_TIMEOUT, "test_e2e_fan_out_fan_in", async {
        let harness = get_harness().await;

        let queue = "fan-out-fan-in-queue";
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-fan-out-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(FanOutFanInWorkflow)
            .register_task(ProcessItemTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "fan-out-fan-in-workflow",
                json!({
                    "items": ["apple", "banana", "cherry"]
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Verify input/output counts match
        assert_eq!(result.get("inputCount"), Some(&json!(3)));
        assert_eq!(result.get("outputCount"), Some(&json!(3)));

        // Verify processed items
        let processed = result
            .get("processedItems")
            .and_then(|v| v.as_array())
            .expect("processedItems should be an array");
        assert_eq!(processed.len(), 3);
        assert!(processed
            .iter()
            .any(|r| r.as_str() == Some("processed:apple")));
        assert!(processed
            .iter()
            .any(|r| r.as_str() == Some("processed:banana")));
        assert!(processed
            .iter()
            .any(|r| r.as_str() == Some("processed:cherry")));

        handle.abort();
    })
    .await;
}

/// Test racing tasks with select - first to complete wins.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_e2e_racing_tasks_select() {
    with_timeout(TEST_TIMEOUT, "test_e2e_racing_tasks_select", async {
        let harness = get_harness().await;

        let queue = "racing-select-queue";
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-racing-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(RacingTasksWorkflow)
            .register_task(FetchDataTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "racing-tasks-workflow",
                json!({
                    "primaryUrl": "http://primary.example.com",
                    "fallbackUrl": "http://fallback.example.com"
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Verify we got a winner
        let winner = result.get("winner").expect("winner should be present");
        assert!(winner.is_object());

        // Winner should have source and data fields
        assert!(winner.get("source").is_some());
        assert!(winner.get("data").is_some());

        handle.abort();
    })
    .await;
}

/// Test timeout protection for slow tasks - task completes in time.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_e2e_timeout_success() {
    with_timeout(TEST_TIMEOUT, "test_e2e_timeout_success", async {
        let harness = get_harness().await;

        let queue = "timeout-success-queue";
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-timeout-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(TimeoutTaskWorkflow)
            .register_task(SlowOperationTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "timeout-task-workflow",
                json!({
                    "timeoutMs": 10000,  // Long timeout
                    "operation": "fast-op"
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Task should complete before timeout
        assert_eq!(result.get("completed"), Some(&json!(true)));
        assert!(result.get("result").is_some());

        handle.abort();
    })
    .await;
}

/// Test mixed parallel operations with tasks and timers.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_e2e_mixed_parallel_operations() {
    let timeout = Duration::from_secs(90);
    with_timeout(timeout, "test_e2e_mixed_parallel_operations", async {
        let harness = get_harness().await;

        let queue = "mixed-parallel-queue";
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-mixed-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(MixedParallelWorkflow)
            .register_task(ProcessItemTask)
            .register_task(SlowOperationTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "mixed-parallel-workflow",
                json!({}),
                options,
                Duration::from_secs(60),
            )
            .await
            .expect("Workflow execution failed");

        // Verify all phases completed
        assert_eq!(result.get("success"), Some(&json!(true)));

        // Phase 1: 2 parallel tasks
        let phase1 = result
            .get("phase1")
            .and_then(|v| v.as_array())
            .expect("phase1 should be an array");
        assert_eq!(phase1.len(), 2);

        // Phase 2: Timer fired
        assert_eq!(result.get("timerFired"), Some(&json!(true)));

        // Phase 3: 3 parallel tasks
        let phase3 = result
            .get("phase3")
            .and_then(|v| v.as_array())
            .expect("phase3 should be an array");
        assert_eq!(phase3.len(), 3);

        handle.abort();
    })
    .await;
}

/// Test parallel tasks with larger batch size.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_e2e_parallel_large_batch() {
    let timeout = Duration::from_secs(90);
    with_timeout(timeout, "test_e2e_parallel_large_batch", async {
        let harness = get_harness().await;

        let queue = "parallel-large-batch-queue";
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-large-batch-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(ParallelTasksWorkflow)
            .register_task(ProcessItemTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Test with 10 items
        let items: Vec<String> = (1..=10).map(|i| format!("item-{}", i)).collect();
        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "parallel-tasks-workflow",
                json!({ "items": items }),
                options,
                Duration::from_secs(60),
            )
            .await
            .expect("Workflow execution failed");

        // Verify all 10 items were processed
        assert_eq!(result.get("itemCount"), Some(&json!(10)));
        let results = result
            .get("results")
            .and_then(|v| v.as_array())
            .expect("results should be an array");
        assert_eq!(results.len(), 10);

        handle.abort();
    })
    .await;
}
