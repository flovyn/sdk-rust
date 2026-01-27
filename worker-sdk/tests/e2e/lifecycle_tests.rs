//! Worker lifecycle E2E tests
//!
//! These tests verify worker lifecycle management against a real Flovyn server.
//! Tests cover status transitions, registration info, connection info, metrics,
//! lifecycle events, and pause/resume functionality.

use crate::fixtures::workflows::{DoublerWorkflow, EchoWorkflow};
use crate::test_env::E2ETestEnvBuilder;
use crate::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_worker_sdk::client::FlovynClient;
use flovyn_worker_sdk::worker::lifecycle::{WorkerLifecycleEvent, WorkerStatus};
use serde_json::json;
use std::time::Duration;
use tokio_stream::StreamExt;

/// Test that worker status transitions correctly through lifecycle states.
/// Expected: Initializing -> Registering -> Running
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_worker_status_transitions() {
    with_timeout(TEST_TIMEOUT, "test_worker_status_transitions", async {
        let harness = get_harness().await;

        // Build client manually to observe status transitions
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-status-test-worker")
            .queue("lifecycle-status-queue")
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        // Before start, there should be no worker internals
        assert!(
            client.worker_internals().is_none(),
            "Worker internals should be None before start"
        );

        // Start the worker
        let handle = client.start().await.expect("Failed to start worker");

        // After start, worker internals should be available
        assert!(
            client.worker_internals().is_some(),
            "Worker internals should be Some after start"
        );

        // Wait for worker to be ready (Running state)
        handle.await_ready().await;

        // Check final status is Running
        let status = handle.status();
        assert!(
            matches!(status, WorkerStatus::Running { .. }),
            "Expected Running status, got: {:?}",
            status
        );

        // Also verify through the client
        assert!(
            client.has_running_worker(),
            "has_running_worker should return true"
        );

        // Stop worker
        handle.stop().await;
    })
    .await;
}

/// Test that registration info is populated after worker starts.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_registration_info() {
    with_timeout(TEST_TIMEOUT, "test_registration_info", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-registration-test-worker")
            .queue("lifecycle-registration-queue")
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give server time to process registration
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Check registration info is populated
        let registration = handle.registration();
        assert!(
            registration.is_some(),
            "Registration info should be populated after worker starts"
        );

        let reg_info = registration.unwrap();
        assert!(reg_info.success, "Registration should be successful");
        assert!(
            !reg_info.worker_id.is_nil(),
            "Server worker ID should be assigned"
        );

        // Verify server_worker_id convenience method
        let server_id = handle.server_worker_id();
        assert!(server_id.is_some(), "server_worker_id() should return Some");
        assert_eq!(
            server_id.unwrap(),
            reg_info.worker_id,
            "server_worker_id should match registration info"
        );

        handle.stop().await;
    })
    .await;
}

/// Test that connection info is tracked correctly.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_connection_info() {
    with_timeout(TEST_TIMEOUT, "test_connection_info", async {
        let harness = get_harness().await;

        // Use short heartbeat interval so we can test heartbeat tracking
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-connection-test-worker")
            .queue("lifecycle-connection-queue")
            .heartbeat_interval(Duration::from_secs(2))
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give time for initial heartbeat (heartbeat interval + buffer)
        tokio::time::sleep(Duration::from_secs(4)).await;

        // Check connection info
        let conn_info = handle.connection();
        assert!(conn_info.connected, "Worker should be connected");
        assert_eq!(
            conn_info.heartbeat_failures, 0,
            "Should have no heartbeat failures"
        );
        assert!(
            conn_info.last_heartbeat.is_some(),
            "Should have recorded a heartbeat"
        );

        // Verify convenience methods
        assert!(handle.is_connected(), "is_connected() should return true");

        let time_since = handle.time_since_heartbeat();
        assert!(
            time_since.is_some(),
            "time_since_heartbeat should return Some"
        );
        assert!(
            time_since.unwrap() < Duration::from_secs(30),
            "Last heartbeat should be recent"
        );

        handle.stop().await;
    })
    .await;
}

/// Test that worker metrics are tracked during workflow execution.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_worker_metrics() {
    with_timeout(TEST_TIMEOUT, "test_worker_metrics", async {
        let env =
            E2ETestEnvBuilder::with_queue("lifecycle-metrics-worker", "lifecycle-metrics-queue")
                .await
                .register_workflow(DoublerWorkflow)
                .build_and_start()
                .await;

        // Check initial metrics
        let initial_metrics = env.client().worker_internals().unwrap().metrics();
        let initial_workflows = initial_metrics.workflows_executed;

        // Execute a workflow
        let _result = env
            .start_and_await("doubler-workflow", json!({"value": 10}))
            .await
            .expect("Workflow execution failed");

        // Give time for metrics to update
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Check updated metrics
        let updated_metrics = env.client().worker_internals().unwrap().metrics();

        assert!(
            updated_metrics.workflows_executed > initial_workflows,
            "workflows_executed should have increased. Initial: {}, Updated: {}",
            initial_workflows,
            updated_metrics.workflows_executed
        );

        // Verify uptime is being tracked
        assert!(
            updated_metrics.uptime > Duration::ZERO,
            "Uptime should be greater than zero"
        );

        // Execute another workflow and verify counter increments
        let _ = env
            .start_and_await("doubler-workflow", json!({"value": 20}))
            .await
            .expect("Second workflow execution failed");

        tokio::time::sleep(Duration::from_millis(500)).await;

        let final_metrics = env.client().worker_internals().unwrap().metrics();
        assert!(
            final_metrics.workflows_executed > updated_metrics.workflows_executed,
            "workflows_executed should have increased again"
        );
    })
    .await;
}

/// Test that lifecycle events are emitted and can be subscribed to.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_lifecycle_events() {
    with_timeout(TEST_TIMEOUT, "test_lifecycle_events", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-events-test-worker")
            .queue("lifecycle-events-queue")
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        let handle = client.start().await.expect("Failed to start worker");

        // Subscribe to events
        let mut event_rx = handle.subscribe();

        // Wait for worker to be ready
        handle.await_ready().await;

        // Collect events received so far
        let mut events_received = Vec::new();

        // Try to receive events with a timeout
        loop {
            match tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await {
                Ok(Ok(event)) => {
                    events_received.push(event.event_name().to_string());
                }
                Ok(Err(_)) => break, // Channel closed or lagged
                Err(_) => break,     // Timeout - no more events
            }
        }

        // Should have received at least Starting and Registered events
        assert!(
            !events_received.is_empty(),
            "Should have received at least one event"
        );

        // Check that we received expected event types
        // Note: exact events depend on server behavior, but we should see registration
        let has_starting_or_registered = events_received
            .iter()
            .any(|e| e == "starting" || e == "registered" || e == "ready");
        assert!(
            has_starting_or_registered,
            "Should have received starting, registered, or ready event. Got: {:?}",
            events_received
        );

        handle.stop().await;
    })
    .await;
}

/// Test that filtered event subscription works correctly.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_filtered_event_subscription() {
    with_timeout(TEST_TIMEOUT, "test_filtered_event_subscription", async {
        let env = E2ETestEnvBuilder::with_queue(
            "lifecycle-filtered-events-worker",
            "lifecycle-filtered-events-queue",
        )
        .await
        .register_workflow(DoublerWorkflow)
        .build_and_start()
        .await;

        let internals = env.client().worker_internals().unwrap();

        // Create a filtered subscription for work-related events only
        let mut work_events = {
            use tokio_stream::wrappers::BroadcastStream;
            let rx = internals.subscribe();
            BroadcastStream::new(rx)
                .filter_map(|result| result.ok())
                .filter(|event| {
                    matches!(
                        event,
                        WorkerLifecycleEvent::WorkReceived { .. }
                            | WorkerLifecycleEvent::WorkCompleted { .. }
                    )
                })
        };

        // Execute a workflow
        let _ = env
            .start_and_await("doubler-workflow", json!({"value": 5}))
            .await
            .expect("Workflow execution failed");

        // Should receive WorkReceived and WorkCompleted events
        let mut received_work_received = false;
        let mut received_work_completed = false;

        // Collect work events
        loop {
            match tokio::time::timeout(Duration::from_secs(2), work_events.next()).await {
                Ok(Some(event)) => match event {
                    WorkerLifecycleEvent::WorkReceived { .. } => received_work_received = true,
                    WorkerLifecycleEvent::WorkCompleted { .. } => received_work_completed = true,
                    _ => {}
                },
                Ok(None) => break,
                Err(_) => break, // Timeout
            }
        }

        assert!(
            received_work_received,
            "Should have received WorkReceived event"
        );
        assert!(
            received_work_completed,
            "Should have received WorkCompleted event"
        );
    })
    .await;
}

/// Test pause and resume functionality.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_pause_resume() {
    with_timeout(TEST_TIMEOUT, "test_pause_resume", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-pause-resume-worker")
            .queue("lifecycle-pause-resume-queue")
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give server time to register
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Verify worker is running
        assert!(handle.is_running(), "Worker should be running initially");
        assert!(!handle.is_paused(), "Worker should not be paused initially");

        // Pause the worker
        let pause_result = handle.pause("test pause reason").await;
        assert!(
            pause_result.is_ok(),
            "Pause should succeed: {:?}",
            pause_result
        );

        // Verify paused state
        assert!(handle.is_paused(), "Worker should be paused after pause()");
        assert!(
            !handle.is_running(),
            "Worker should not be running while paused"
        );

        match handle.status() {
            WorkerStatus::Paused { reason } => {
                assert_eq!(
                    reason, "test pause reason",
                    "Pause reason should be preserved"
                );
            }
            other => panic!("Expected Paused status, got: {:?}", other),
        }

        // Resume the worker
        let resume_result = handle.resume().await;
        assert!(
            resume_result.is_ok(),
            "Resume should succeed: {:?}",
            resume_result
        );

        // Verify running state
        assert!(
            handle.is_running(),
            "Worker should be running after resume()"
        );
        assert!(
            !handle.is_paused(),
            "Worker should not be paused after resume()"
        );

        handle.stop().await;
    })
    .await;
}

/// Test that pause fails when worker is not in Running state.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_pause_invalid_state() {
    with_timeout(TEST_TIMEOUT, "test_pause_invalid_state", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-pause-invalid-worker")
            .queue("lifecycle-pause-invalid-queue")
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Pause first time - should succeed
        handle
            .pause("first pause")
            .await
            .expect("First pause should succeed");

        // Try to pause again while already paused - should fail
        let result = handle.pause("second pause").await;
        assert!(result.is_err(), "Pause while already paused should fail");

        // Resume to clean up
        handle.resume().await.expect("Resume should succeed");
        handle.stop().await;
    })
    .await;
}

/// Test that resume fails when worker is not in Paused state.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_resume_invalid_state() {
    with_timeout(TEST_TIMEOUT, "test_resume_invalid_state", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-resume-invalid-worker")
            .queue("lifecycle-resume-invalid-queue")
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Try to resume when not paused - should fail
        let result = handle.resume().await;
        assert!(result.is_err(), "Resume while not paused should fail");

        handle.stop().await;
    })
    .await;
}

/// Test worker uptime tracking.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_worker_uptime() {
    with_timeout(TEST_TIMEOUT, "test_worker_uptime", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-uptime-worker")
            .queue("lifecycle-uptime-queue")
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Record initial uptime
        let initial_uptime = handle.uptime();

        // Wait a bit
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Check uptime has increased
        let later_uptime = handle.uptime();
        assert!(
            later_uptime > initial_uptime,
            "Uptime should increase over time. Initial: {:?}, Later: {:?}",
            initial_uptime,
            later_uptime
        );

        // Uptime should be at least 2 seconds
        assert!(
            later_uptime >= Duration::from_secs(2),
            "Uptime should be at least 2 seconds: {:?}",
            later_uptime
        );

        handle.stop().await;
    })
    .await;
}

/// Test configuration accessor methods on FlovynClient.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_client_config_accessors() {
    with_timeout(TEST_TIMEOUT, "test_client_config_accessors", async {
        let harness = get_harness().await;

        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_token(harness.worker_token())
            .worker_id("lifecycle-config-worker")
            .queue("lifecycle-config-queue")
            .heartbeat_interval(Duration::from_secs(15))
            .poll_timeout(Duration::from_secs(20))
            .register_workflow(EchoWorkflow)
            .build()
            .await
            .expect("Failed to build client");

        // Verify configuration accessors
        assert_eq!(
            client.heartbeat_interval(),
            Duration::from_secs(15),
            "Heartbeat interval should match configured value"
        );
        assert_eq!(
            client.poll_timeout(),
            Duration::from_secs(20),
            "Poll timeout should match configured value"
        );

        // Max concurrent should have default values
        assert!(
            client.max_concurrent_workflows() > 0,
            "Max concurrent workflows should be positive"
        );
        assert!(
            client.max_concurrent_tasks() > 0,
            "Max concurrent tasks should be positive"
        );
    })
    .await;
}
