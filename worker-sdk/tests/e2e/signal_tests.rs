//! Signal E2E tests
//!
//! These tests verify workflow signal functionality against a real server.

use crate::fixtures::workflows::{MultiSignalWorkflow, SignalCheckWorkflow, SignalWorkflow};
use crate::test_env::E2ETestEnvBuilder;
use crate::{with_timeout, TEST_TIMEOUT};
use flovyn_worker_sdk::client::SignalWithStartOptions;
use serde_json::json;
use std::time::Duration;

/// Test signal-with-start: atomically start workflow and send signal.
///
/// This tests the flow:
/// 1. Call signal_with_start_workflow() with new workflow
/// 2. Workflow is created and signal is delivered
/// 3. Workflow receives signal via wait_for_signal_raw()
/// 4. Workflow completes with signal value
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_signal_with_start_new_workflow() {
    with_timeout(TEST_TIMEOUT, "test_signal_with_start_new_workflow", async {
        let env = E2ETestEnvBuilder::with_queue("e2e-signal-worker", "signal-start-queue")
            .await
            .register_workflow(SignalWorkflow)
            .build_and_start()
            .await;

        // Use signal_with_start to create workflow and send initial signal
        // NOTE: Signal name must match what the workflow expects ("signal")
        let options = SignalWithStartOptions::new(
            "signal-test-workflow-1",
            "signal-workflow",
            json!({}),
            "signal", // Must match the signal name the workflow waits for
            json!({"message": "Hello from signal!"}),
        )
        .queue("signal-start-queue");

        let result = env
            .client()
            .signal_with_start_workflow(options)
            .await
            .expect("Failed to signal with start");

        assert!(result.workflow_created, "Workflow should have been created");

        // Wait for workflow to complete
        let workflow_result = env.await_completion(result.workflow_execution_id).await;
        env.assert_completed(&workflow_result);

        // Verify the output contains the signal
        let output = workflow_result.output.as_ref().expect("Expected output");
        assert_eq!(output["signalName"], json!("signal"));
        assert_eq!(
            output["signalValue"]["message"],
            json!("Hello from signal!")
        );
    })
    .await;
}

/// Test signal to existing workflow.
///
/// This tests the flow:
/// 1. Start workflow that waits for signal
/// 2. Workflow suspends waiting for signal
/// 3. Send signal via signal_workflow()
/// 4. Workflow receives signal and completes
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_signal_existing_workflow() {
    with_timeout(TEST_TIMEOUT, "test_signal_existing_workflow", async {
        let env =
            E2ETestEnvBuilder::with_queue("e2e-signal-existing-worker", "signal-existing-queue")
                .await
                .register_workflow(SignalWorkflow)
                .build_and_start()
                .await;

        // Start the workflow (it will suspend waiting for signal)
        let workflow_id = env.start_workflow("signal-workflow", json!({})).await;

        // Wait for workflow to suspend
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Send signal to the workflow (must use "signal" name that workflow expects)
        let signal_result = env
            .client()
            .signal_workflow(
                workflow_id,
                "signal", // Must match the signal name the workflow waits for
                json!({"action": "approve", "user": "admin"}),
            )
            .await
            .expect("Failed to send signal");

        assert!(signal_result.signal_event_sequence > 0);

        // Wait for workflow to complete
        let result = env.await_completion(workflow_id).await;
        env.assert_completed(&result);

        // Verify the output contains the signal
        let output = result.output.as_ref().expect("Expected output");
        assert_eq!(output["signalName"], json!("signal"));
        assert_eq!(output["signalValue"]["action"], json!("approve"));
    })
    .await;
}

/// Test multiple signals to workflow.
///
/// This tests queue semantics:
/// 1. Start workflow that waits for multiple signals
/// 2. Send multiple signals
/// 3. Workflow receives all signals in order
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_multiple_signals() {
    with_timeout(TEST_TIMEOUT, "test_multiple_signals", async {
        let env = E2ETestEnvBuilder::with_queue("e2e-multi-signal-worker", "multi-signal-queue")
            .await
            .register_workflow(MultiSignalWorkflow)
            .build_and_start()
            .await;

        // Start the workflow expecting 3 signals
        let workflow_id = env
            .start_workflow("multi-signal-workflow", json!({"signalCount": 3}))
            .await;

        // Wait for workflow to start and suspend
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Send 3 signals (all with same name "signal" - per-name FIFO queue)
        for i in 1..=3 {
            env.client()
                .signal_workflow(
                    workflow_id,
                    "signal", // Must match the signal name the workflow waits for
                    json!({"content": format!("Message {}", i), "seq": i}),
                )
                .await
                .expect("Failed to send signal");

            // Small delay between signals
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Wait for workflow to complete
        let result = env.await_completion(workflow_id).await;
        env.assert_completed(&result);

        // Verify all signals were received in FIFO order
        let output = result.output.as_ref().expect("Expected output");
        assert_eq!(output["count"], json!(3));

        let signals = output["signals"]
            .as_array()
            .expect("Expected signals array");
        assert_eq!(signals.len(), 3);
        // All signals have the same name "signal"
        assert_eq!(signals[0]["name"], json!("signal"));
        assert_eq!(signals[1]["name"], json!("signal"));
        assert_eq!(signals[2]["name"], json!("signal"));
        // Verify FIFO ordering via seq field
        assert_eq!(signals[0]["value"]["seq"], json!(1));
        assert_eq!(signals[1]["value"]["seq"], json!(2));
        assert_eq!(signals[2]["value"]["seq"], json!(3));
    })
    .await;
}

/// Test signal-with-start to existing workflow.
///
/// This tests idempotency:
/// 1. Start workflow via signal_with_start
/// 2. Call signal_with_start again with same workflow_id
/// 3. Second call should NOT create new workflow, just add signal
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_signal_with_start_existing() {
    with_timeout(TEST_TIMEOUT, "test_signal_with_start_existing", async {
        let env = E2ETestEnvBuilder::with_queue(
            "e2e-signal-existing-start-worker",
            "signal-existing-start-queue",
        )
        .await
        .register_workflow(MultiSignalWorkflow)
        .build_and_start()
        .await;

        let workflow_id = "signal-existing-test-workflow";

        // First signal_with_start creates the workflow (use "signal" name)
        let options1 = SignalWithStartOptions::new(
            workflow_id,
            "multi-signal-workflow",
            json!({"signalCount": 2}),
            "signal", // Must match the signal name the workflow waits for
            json!({"seq": 1}),
        )
        .queue("signal-existing-start-queue");

        let result1 = env
            .client()
            .signal_with_start_workflow(options1)
            .await
            .expect("Failed to signal with start (first)");

        assert!(
            result1.workflow_created,
            "First call should create workflow"
        );
        let execution_id = result1.workflow_execution_id;

        // Small delay
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Second signal_with_start to same workflow_id (same signal name)
        let options2 = SignalWithStartOptions::new(
            workflow_id,
            "multi-signal-workflow",
            json!({"signalCount": 2}),
            "signal", // Must match the signal name the workflow waits for
            json!({"seq": 2}),
        )
        .queue("signal-existing-start-queue");

        let result2 = env
            .client()
            .signal_with_start_workflow(options2)
            .await
            .expect("Failed to signal with start (second)");

        assert!(
            !result2.workflow_created,
            "Second call should NOT create new workflow"
        );
        assert_eq!(
            result2.workflow_execution_id, execution_id,
            "Should be same execution"
        );

        // Wait for workflow to complete (it expects 2 signals)
        let result = env.await_completion(execution_id).await;
        env.assert_completed(&result);

        // Verify both signals were received
        let output = result.output.as_ref().expect("Expected output");
        let signals = output["signals"]
            .as_array()
            .expect("Expected signals array");
        assert_eq!(signals.len(), 2);
    })
    .await;
}

/// Test has_signal and drain_signals APIs.
///
/// This tests non-blocking signal checking:
/// 1. Send signals via signal_with_start
/// 2. Workflow checks has_signal() and drains all
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_signal_check_and_drain() {
    with_timeout(TEST_TIMEOUT, "test_signal_check_and_drain", async {
        let env = E2ETestEnvBuilder::with_queue("e2e-signal-check-worker", "signal-check-queue")
            .await
            .register_workflow(SignalCheckWorkflow)
            .build_and_start()
            .await;

        // Use signal_with_start to create workflow with initial signal
        // NOTE: Signal name must match what the workflow expects ("test")
        let options = SignalWithStartOptions::new(
            "signal-check-workflow-1",
            "signal-check-workflow",
            json!({}),
            "test", // Must match the signal name the workflow checks
            json!({"data": "first"}),
        )
        .queue("signal-check-queue");

        let result = env
            .client()
            .signal_with_start_workflow(options)
            .await
            .expect("Failed to signal with start");

        // Send another signal immediately with the same name
        env.client()
            .signal_workflow(
                result.workflow_execution_id,
                "test", // Must match the signal name the workflow checks
                json!({"data": "second"}),
            )
            .await
            .expect("Failed to send second signal");

        // Wait for workflow to complete
        let workflow_result = env.await_completion(result.workflow_execution_id).await;
        env.assert_completed(&workflow_result);

        // Verify the output
        let output = workflow_result.output.as_ref().expect("Expected output");

        // has_signal should have been true initially
        assert_eq!(output["hasSignal"], json!(true));

        // Should have drained both signals
        let signals = output["signals"]
            .as_array()
            .expect("Expected signals array");
        assert!(!signals.is_empty(), "Should have at least 1 signal");
    })
    .await;
}
