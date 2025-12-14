//! Timer E2E tests
//!
//! These tests verify workflow timer/sleep functionality against a real server.

use crate::fixtures::workflows::TimerWorkflow;
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_sdk::client::{FlovynClient, StartWorkflowOptions};
use serde_json::json;
use std::time::{Duration, Instant};

/// Test durable timer sleep.
/// The workflow sleeps for a short duration and returns timing information.
/// Note: We measure wall-clock time for the duration because ctx.current_time_millis()
/// returns the deterministic workflow task time which doesn't advance during sleep.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_durable_timer_sleep() {
    with_timeout(TEST_TIMEOUT, "test_durable_timer_sleep", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-timer-worker")
            .worker_token(harness.worker_token())
            .register_workflow(TimerWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the worker time to register
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start a workflow that sleeps for 2 seconds
        // Measure wall-clock time
        let start_time = Instant::now();
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_and_wait_with_options(
                "timer-workflow",
                json!({
                    "sleepMs": 2000
                }),
                options,
                Duration::from_secs(30), // Timeout
            )
            .await
            .expect("Workflow execution failed");
        let wall_elapsed_ms = start_time.elapsed().as_millis() as i64;

        // Verify the workflow completed with the expected slept duration
        assert_eq!(result["sleptMs"], json!(2000));

        // Wall-clock time should be at least 2000ms (timer duration)
        assert!(
            wall_elapsed_ms >= 2000,
            "Expected wall-clock elapsed >= 2000ms, got {}ms",
            wall_elapsed_ms
        );
        // And not too much more (allow 5 seconds for overhead)
        assert!(
            wall_elapsed_ms < 7000,
            "Workflow should complete within 5s of expected time, took {}ms",
            wall_elapsed_ms
        );

        handle.abort();
    })
    .await;
}

/// Test timer with very short duration (100ms).
/// This tests that even short timers work correctly through suspend/resume.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_short_timer() {
    with_timeout(TEST_TIMEOUT, "test_short_timer", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_address(harness.grpc_host(), harness.grpc_port())
            .tenant_id(harness.tenant_id())
            .worker_id("e2e-short-timer-worker")
            .worker_token(harness.worker_token())
            .register_workflow(TimerWorkflow)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Start a workflow that sleeps for 100ms
        let start_time = Instant::now();
        let options = StartWorkflowOptions::new().with_workflow_version("1.0.0");
        let result = client
            .start_workflow_and_wait_with_options(
                "timer-workflow",
                json!({
                    "sleepMs": 100
                }),
                options,
                Duration::from_secs(30),
            )
            .await
            .expect("Workflow execution failed");
        let wall_elapsed_ms = start_time.elapsed().as_millis() as i64;

        assert_eq!(result["sleptMs"], json!(100));

        // Wall-clock time should be at least 100ms (timer duration)
        assert!(
            wall_elapsed_ms >= 100,
            "Expected wall-clock elapsed >= 100ms, got {}ms",
            wall_elapsed_ms
        );

        handle.abort();
    })
    .await;
}
