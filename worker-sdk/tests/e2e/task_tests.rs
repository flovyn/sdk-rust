//! Task E2E tests
//!
//! These tests verify task execution against a real Flovyn server.
//! Tests the full workflow → task → resume cycle.

use crate::fixtures::tasks::EchoTask;
use crate::fixtures::workflows::TaskSchedulingWorkflow;
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_worker_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::Duration;

/// Test basic task scheduling: workflow schedules task, receives result.
/// This tests the full flow:
/// 1. Workflow worker executes workflow
/// 2. Workflow calls ctx.schedule_raw() to schedule a task
/// 3. Workflow suspends (state = WAITING)
/// 4. Task worker polls and executes task
/// 5. Task completes and records TASK_COMPLETED event
/// 6. Workflow worker polls again and resumes workflow with task result
/// 7. Workflow completes with final output
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_basic_task_scheduling() {
    with_timeout(TEST_TIMEOUT, "test_basic_task_scheduling", async {
        let harness = get_harness().await;

        let queue = "task-basic-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-task-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(TaskSchedulingWorkflow)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the worker time to register
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start a workflow that schedules an echo task
        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(queue);
        let result = client
            .start_workflow_and_wait_with_options(
                "task-scheduling-workflow",
                json!({
                    "taskInput": {
                        "message": "Hello from task!",
                        "number": 42
                    }
                }),
                options,
                Duration::from_secs(30), // Timeout
            )
            .await
            .expect("Workflow execution failed");

        // Verify the workflow completed with the task result
        let task_result = result
            .get("taskResult")
            .expect("taskResult should be in output");

        // EchoTask echoes the input, so the result should match
        assert_eq!(task_result.get("message"), Some(&json!("Hello from task!")));
        assert_eq!(task_result.get("number"), Some(&json!(42)));

        handle.abort();
    })
    .await;
}

/// Test workflow scheduling multiple tasks sequentially.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_multiple_sequential_tasks() {
    // Use longer timeout for multi-task test
    let timeout = Duration::from_secs(90);
    with_timeout(timeout, "test_multiple_sequential_tasks", async {
        use crate::fixtures::tasks::SlowTask;
        use crate::fixtures::workflows::MultiTaskWorkflow;

        let harness = get_harness().await;

        let queue = "task-multi-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-multi-task-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(MultiTaskWorkflow)
            .register_task(EchoTask)
            .register_task(SlowTask)
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
                "multi-task-workflow",
                json!({}),
                options,
                Duration::from_secs(60),
            )
            .await
            .expect("Workflow execution failed");

        // Verify both tasks completed
        assert_eq!(result.get("taskCount"), Some(&json!(2)));
        assert!(result.get("echoResult").is_some());
        assert!(result.get("slowResult").is_some());

        handle.abort();
    })
    .await;
}
