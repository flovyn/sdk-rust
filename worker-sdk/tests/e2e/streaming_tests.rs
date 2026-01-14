//! Streaming E2E tests
//!
//! These tests verify task streaming functionality against a real Flovyn server.
//! Tests focus on verifying that streaming tasks complete successfully and produce
//! the expected output. Since Phase 5 (SSE subscription) is deferred, we don't
//! verify actual stream event delivery, but we verify:
//! - Tasks that use streaming complete successfully
//! - Streaming calls don't fail the task
//! - Task output includes expected results

use crate::fixtures::tasks::{
    StreamingAllTypesTask, StreamingDataTask, StreamingErrorTask, StreamingProgressTask,
    StreamingTokenTask,
};
use crate::fixtures::workflows::{
    StreamingAllTypesWorkflow, StreamingDataWorkflow, StreamingErrorWorkflow,
    StreamingProgressWorkflow, StreamingTokenWorkflow,
};
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_worker_sdk::client::{FlovynClient, StartWorkflowOptions, WorkerHandle};
use flovyn_worker_sdk::task::definition::DynamicTask;
use flovyn_worker_sdk::worker::lifecycle::WorkerLifecycleEvent;
use serde_json::json;
use std::time::Duration;

/// Wait for worker registration by subscribing to lifecycle events.
/// This is more reliable than sleeping for a fixed duration.
async fn await_registration(handle: &WorkerHandle) {
    let mut rx = handle.subscribe();
    // Wait for the Registered event with a timeout
    let timeout = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await {
                Ok(WorkerLifecycleEvent::Registered { .. }) => break,
                Ok(_) => continue, // Wait for more events
                Err(_) => break,   // Channel closed or lagged
            }
        }
    });
    let _ = timeout.await;
}

/// Test basic token streaming: workflow schedules a streaming token task.
/// Verifies the task completes successfully and produces expected output.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_task_streams_tokens() {
    with_timeout(TEST_TIMEOUT, "test_task_streams_tokens", async {
        let harness = get_harness().await;

        let queue = format!("stream-tokens-{}", uuid::Uuid::new_v4());
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id("e2e-streaming-worker")
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(StreamingTokenWorkflow)
            .register_task(StreamingTokenTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        await_registration(&handle).await;

        // Start a workflow that schedules a streaming token task
        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);

        let result = client
            .start_workflow_and_wait_with_options(
                "streaming-token-workflow",
                json!({
                    "taskInput": {
                        "tokens": ["Hello", ", ", "streaming", " ", "world", "!"]
                    }
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        // Verify the workflow completed with the task result
        let task_result = result
            .get("taskResult")
            .expect("taskResult should be in output");

        // Verify token count and full text
        assert_eq!(task_result.get("tokenCount"), Some(&json!(6)));
        assert_eq!(
            task_result.get("fullText"),
            Some(&json!("Hello, streaming world!"))
        );

        handle.abort();
    })
    .await;
}

/// Test progress streaming: workflow schedules a streaming progress task.
/// Verifies the task completes successfully after streaming progress events.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_task_streams_progress() {
    with_timeout(TEST_TIMEOUT, "test_task_streams_progress", async {
        let harness = get_harness().await;

        let queue = format!("stream-progress-{}", uuid::Uuid::new_v4());
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id("e2e-progress-worker")
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(StreamingProgressWorkflow)
            .register_task(StreamingProgressTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        await_registration(&handle).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);

        let result = client
            .start_workflow_and_wait_with_options(
                "streaming-progress-workflow",
                json!({
                    "taskInput": {
                        "steps": 5
                    }
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        let task_result = result
            .get("taskResult")
            .expect("taskResult should be in output");

        // Verify completed steps
        assert_eq!(task_result.get("completedSteps"), Some(&json!(5)));

        handle.abort();
    })
    .await;
}

/// Test data streaming: workflow schedules a streaming data task.
/// Verifies the task completes successfully after streaming data events.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_task_streams_data() {
    with_timeout(TEST_TIMEOUT, "test_task_streams_data", async {
        let harness = get_harness().await;

        let queue = format!("stream-data-{}", uuid::Uuid::new_v4());
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id("e2e-data-worker")
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(StreamingDataWorkflow)
            .register_task(StreamingDataTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        await_registration(&handle).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);

        let result = client
            .start_workflow_and_wait_with_options(
                "streaming-data-workflow",
                json!({
                    "taskInput": {
                        "count": 5
                    }
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        let task_result = result
            .get("taskResult")
            .expect("taskResult should be in output");

        // Verify record count
        assert_eq!(task_result.get("recordCount"), Some(&json!(5)));

        handle.abort();
    })
    .await;
}

/// Test error streaming: workflow schedules a streaming error task.
/// Verifies the task completes successfully even after streaming error notifications.
/// This confirms that stream_error() doesn't fail the task.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_task_streams_errors() {
    with_timeout(TEST_TIMEOUT, "test_task_streams_errors", async {
        let harness = get_harness().await;

        let queue = format!("stream-errors-{}", uuid::Uuid::new_v4());
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id("e2e-error-worker")
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(StreamingErrorWorkflow)
            .register_task(StreamingErrorTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        await_registration(&handle).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);

        let result = client
            .start_workflow_and_wait_with_options(
                "streaming-error-workflow",
                json!({
                    "taskInput": {
                        "errorCount": 3
                    }
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        let task_result = result
            .get("taskResult")
            .expect("taskResult should be in output");

        // Verify errors were streamed but task still succeeded
        assert_eq!(task_result.get("errorCount"), Some(&json!(3)));
        assert_eq!(task_result.get("success"), Some(&json!(true)));

        handle.abort();
    })
    .await;
}

/// Test all streaming types: workflow schedules a task that streams all event types.
/// Verifies the task completes successfully after streaming tokens, progress, data, and errors.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_task_streams_all_types() {
    with_timeout(TEST_TIMEOUT, "test_task_streams_all_types", async {
        let harness = get_harness().await;

        let queue = format!("stream-all-{}", uuid::Uuid::new_v4());
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id("e2e-allstreaming-worker")
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(StreamingAllTypesWorkflow)
            .register_task(StreamingAllTypesTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        await_registration(&handle).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);

        let result = client
            .start_workflow_and_wait_with_options(
                "streaming-all-types-workflow",
                json!({}),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        let task_result = result
            .get("taskResult")
            .expect("taskResult should be in output");

        // Verify task succeeded
        assert_eq!(task_result.get("success"), Some(&json!(true)));

        handle.abort();
    })
    .await;
}

/// Test uses_streaming() flag on TaskDefinition.
/// This is a unit test that doesn't require a server.
#[tokio::test]
async fn test_streaming_task_definition_flag() {
    // Default task should not use streaming
    use crate::fixtures::tasks::EchoTask;
    let echo_task = EchoTask;
    assert!(
        !echo_task.uses_streaming(),
        "EchoTask should not use streaming"
    );

    // Streaming tasks should indicate they use streaming
    let streaming_task = StreamingTokenTask;
    assert!(
        streaming_task.uses_streaming(),
        "StreamingTokenTask should use streaming"
    );

    let streaming_progress_task = StreamingProgressTask;
    assert!(
        streaming_progress_task.uses_streaming(),
        "StreamingProgressTask should use streaming"
    );

    let streaming_data_task = StreamingDataTask;
    assert!(
        streaming_data_task.uses_streaming(),
        "StreamingDataTask should use streaming"
    );

    let streaming_error_task = StreamingErrorTask;
    assert!(
        streaming_error_task.uses_streaming(),
        "StreamingErrorTask should use streaming"
    );

    let streaming_all_types_task = StreamingAllTypesTask;
    assert!(
        streaming_all_types_task.uses_streaming(),
        "StreamingAllTypesTask should use streaming"
    );
}

/// Test that streaming works with custom tokens from input.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_task_streams_custom_tokens() {
    with_timeout(TEST_TIMEOUT, "test_task_streams_custom_tokens", async {
        let harness = get_harness().await;

        let queue = format!("stream-custom-{}", uuid::Uuid::new_v4());
        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .org_id(harness.org_id())
            .worker_id("e2e-custom-worker")
            .worker_token(harness.worker_token())
            .queue(&queue)
            .register_workflow(StreamingTokenWorkflow)
            .register_task(StreamingTokenTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        await_registration(&handle).await;

        let options = StartWorkflowOptions::new()
            .with_workflow_version("1.0.0")
            .with_queue(&queue);

        // Test with custom tokens
        let custom_tokens = vec!["The", " quick", " brown", " fox"];
        let result = client
            .start_workflow_and_wait_with_options(
                "streaming-token-workflow",
                json!({
                    "taskInput": {
                        "tokens": custom_tokens
                    }
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");

        let task_result = result
            .get("taskResult")
            .expect("taskResult should be in output");

        assert_eq!(task_result.get("tokenCount"), Some(&json!(4)));
        assert_eq!(
            task_result.get("fullText"),
            Some(&json!("The quick brown fox"))
        );

        handle.abort();
    })
    .await;
}
