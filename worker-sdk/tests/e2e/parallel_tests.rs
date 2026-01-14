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
use flovyn_worker_sdk::client::{FlovynClient, StartWorkflowOptions};
use flovyn_worker_sdk::workflow::definition::DynamicWorkflow;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

/// Test basic parallel task scheduling with join_all.
#[tokio::test]
#[ignore]
async fn test_e2e_parallel_tasks_join_all() {
    with_timeout(TEST_TIMEOUT, "test_e2e_parallel_tasks_join_all", async {
        let harness = get_harness().await;
        let prefix = format!("join-all-{}", Uuid::new_v4());
        let queue = format!("q:{}", prefix);

        let workflow = ParallelTasksWorkflow::with_prefix(&prefix);
        let workflow_kind = workflow.kind().to_string();

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id(format!("worker:{}", prefix))
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(workflow)
            .register_task(ProcessItemTask::with_prefix(&prefix))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);
        let result = client
            .start_workflow_and_wait_with_options(
                &workflow_kind,
                json!({ "items": ["a", "b", "c"] }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        assert_eq!(result.get("itemCount"), Some(&json!(3)));
        let results = result
            .get("results")
            .and_then(|v| v.as_array())
            .expect("results should be an array");
        assert_eq!(results.len(), 3);
        assert!(results.iter().any(|r| r == &json!("processed:a")));
        assert!(results.iter().any(|r| r == &json!("processed:b")));
        assert!(results.iter().any(|r| r == &json!("processed:c")));

        handle.abort();
    })
    .await;
}

/// Test fan-out/fan-in pattern with result aggregation.
#[tokio::test]
#[ignore]
async fn test_e2e_fan_out_fan_in() {
    with_timeout(TEST_TIMEOUT, "test_e2e_fan_out_fan_in", async {
        let harness = get_harness().await;
        let prefix = format!("fan-out-{}", Uuid::new_v4());
        let queue = format!("q:{}", prefix);

        let workflow = FanOutFanInWorkflow::with_prefix(&prefix);
        let workflow_kind = workflow.kind().to_string();

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id(format!("worker:{}", prefix))
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(workflow)
            .register_task(ProcessItemTask::with_prefix(&prefix))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);
        let result = client
            .start_workflow_and_wait_with_options(
                &workflow_kind,
                json!({ "items": ["apple", "banana", "cherry"] }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        assert_eq!(result.get("inputCount"), Some(&json!(3)));
        assert_eq!(result.get("outputCount"), Some(&json!(3)));

        let processed = result
            .get("processedItems")
            .and_then(|v| v.as_array())
            .expect("processedItems");
        assert_eq!(processed.len(), 3);

        handle.abort();
    })
    .await;
}

/// Test racing tasks with select - first to complete wins.
#[tokio::test]
#[ignore]
async fn test_e2e_racing_tasks_select() {
    with_timeout(TEST_TIMEOUT, "test_e2e_racing_tasks_select", async {
        let harness = get_harness().await;
        let prefix = format!("racing-{}", Uuid::new_v4());
        let queue = format!("q:{}", prefix);

        let workflow = RacingTasksWorkflow::with_prefix(&prefix);
        let workflow_kind = workflow.kind().to_string();

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id(format!("worker:{}", prefix))
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(workflow)
            .register_task(FetchDataTask::with_prefix(&prefix))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);
        let result = client
            .start_workflow_and_wait_with_options(
                &workflow_kind,
                json!({
                    "primaryUrl": "http://primary.example.com",
                    "fallbackUrl": "http://fallback.example.com"
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        let winner = result.get("winner").expect("winner should be present");
        assert!(winner.is_object());
        assert!(winner.get("source").is_some());
        assert!(winner.get("data").is_some());

        handle.abort();
    })
    .await;
}

/// Test timeout protection for slow tasks - task completes in time.
#[tokio::test]
#[ignore]
async fn test_e2e_timeout_success() {
    with_timeout(TEST_TIMEOUT, "test_e2e_timeout_success", async {
        let harness = get_harness().await;
        let prefix = format!("timeout-{}", Uuid::new_v4());
        let queue = format!("q:{}", prefix);

        let workflow = TimeoutTaskWorkflow::with_prefix(&prefix);
        let workflow_kind = workflow.kind().to_string();

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id(format!("worker:{}", prefix))
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(workflow)
            .register_task(SlowOperationTask::with_prefix(&prefix))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);
        let result = client
            .start_workflow_and_wait_with_options(
                &workflow_kind,
                json!({ "timeoutMs": 10000, "operation": "fast-op" }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        assert_eq!(result.get("completed"), Some(&json!(true)));
        assert!(result.get("result").is_some());

        handle.abort();
    })
    .await;
}

/// Test mixed parallel operations with tasks and timers.
#[tokio::test]
#[ignore]
async fn test_e2e_mixed_parallel_operations() {
    let timeout = Duration::from_secs(90);
    with_timeout(timeout, "test_e2e_mixed_parallel_operations", async {
        let harness = get_harness().await;
        let prefix = format!("mixed-{}", Uuid::new_v4());
        let queue = format!("q:{}", prefix);

        let workflow = MixedParallelWorkflow::with_prefix(&prefix);
        let workflow_kind = workflow.kind().to_string();

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id(format!("worker:{}", prefix))
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(workflow)
            .register_task(ProcessItemTask::with_prefix(&prefix))
            .register_task(SlowOperationTask::with_prefix(&prefix))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);
        let result = client
            .start_workflow_and_wait_with_options(
                &workflow_kind,
                json!({}),
                options,
                Duration::from_secs(60),
            )
            .await
            .expect("Workflow execution failed");

        assert_eq!(result.get("success"), Some(&json!(true)));
        assert_eq!(
            result
                .get("phase1")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(2)
        );
        assert_eq!(result.get("timerFired"), Some(&json!(true)));
        assert_eq!(
            result
                .get("phase3")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(3)
        );

        handle.abort();
    })
    .await;
}

/// Test parallel tasks with larger batch size.
#[tokio::test]
#[ignore]
async fn test_e2e_parallel_large_batch() {
    let timeout = Duration::from_secs(120);
    with_timeout(timeout, "test_e2e_parallel_large_batch", async {
        let harness = get_harness().await;
        let prefix = format!("large-batch-{}", Uuid::new_v4());
        let queue = format!("q:{}", prefix);

        let workflow = ParallelTasksWorkflow::with_prefix(&prefix);
        let workflow_kind = workflow.kind().to_string();

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id(format!("worker:{}", prefix))
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(workflow)
            .register_task(ProcessItemTask::with_prefix(&prefix))
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        let items: Vec<String> = (1..=10).map(|i| format!("item-{}", i)).collect();
        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);
        let result = client
            .start_workflow_and_wait_with_options(
                &workflow_kind,
                json!({ "items": items }),
                options,
                Duration::from_secs(90),
            )
            .await
            .expect("Workflow execution failed");

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
