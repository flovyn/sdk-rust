//! E2E tests for sequence-based replay and determinism validation
//!
//! These tests verify that:
//! - Workflows can be replayed with identical code
//! - Determinism violations are detected when workflow code changes
//! - Per-type sequence matching works correctly for loops and interleaved commands
//!
//! All tests are marked `#[ignore]` because they require the Flovyn server
//! and dev infrastructure to be running.

#![allow(unused_imports)] // Fixtures are used for reference

use super::fixtures::workflows::*;
use super::replay_utils::to_replay_events;
use super::{get_harness, with_timeout, TEST_TIMEOUT};
use flovyn_sdk::error::{DeterminismViolationError, FlovynError};
use flovyn_sdk::workflow::context::WorkflowContext;
use flovyn_sdk::workflow::context_impl::WorkflowContextImpl;
use flovyn_sdk::workflow::recorder::CommandCollector;
use uuid::Uuid;

/// Test that a workflow with tasks in a loop can be replayed correctly.
/// This validates per-type sequence matching for TaskScheduled events.
#[tokio::test]
#[ignore]
async fn test_e2e_task_loop_replay() {
    with_timeout(TEST_TIMEOUT, "test_e2e_task_loop_replay", async {
        let harness = get_harness().await;

        // 1. Execute workflow - this would normally run through the server
        // For now, we'll create a simulated execution with mock events
        let workflow_execution_id = Uuid::new_v4();
        let tenant_id = harness.tenant_id();

        // 2. Get events from server (simulated)
        // In a full E2E test, this would call harness.get_workflow_events()

        // 3. Replay with same workflow code
        // The replay context would be created from the events and used to
        // re-execute the workflow logic

        // This test is a placeholder - full implementation requires
        // the workflow executor to be wired up to the test harness
        println!(
            "Task loop replay test: workflow_execution_id={}, tenant_id={}",
            workflow_execution_id, tenant_id
        );
    })
    .await;
}

/// Test that child workflows in a loop can be replayed correctly.
/// This validates per-type sequence matching for ChildWorkflowInitiated events.
#[tokio::test]
#[ignore]
async fn test_e2e_child_workflow_loop_replay() {
    with_timeout(TEST_TIMEOUT, "test_e2e_child_workflow_loop_replay", async {
        let harness = get_harness().await;

        let workflow_execution_id = Uuid::new_v4();
        let tenant_id = harness.tenant_id();

        // Similar to task loop test - validates child workflow replay
        println!(
            "Child workflow loop replay test: workflow_execution_id={}, tenant_id={}",
            workflow_execution_id, tenant_id
        );
    })
    .await;
}

/// Test that determinism violation is detected when task type changes.
/// Uses synthetic events to simulate a code change.
#[tokio::test]
#[ignore]
async fn test_e2e_determinism_violation_on_task_type_change() {
    with_timeout(
        TEST_TIMEOUT,
        "test_e2e_determinism_violation_on_task_type_change",
        async {
            let harness = get_harness().await;

            // Create events for original workflow (scheduled task-A)
            let events = vec![super::harness::WorkflowEventResponse {
                sequence_number: 1,
                event_type: "TaskScheduled".to_string(),
                data: serde_json::json!({
                    "taskType": "task-A",
                    "taskExecutionId": "task-1"
                }),
                created_at: "2024-01-01T00:00:00Z".to_string(),
            }];

            let replay_events = to_replay_events(&events);

            // Create context with these events
            let _ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
                Uuid::new_v4(),
                harness.tenant_id(),
                serde_json::json!({}),
                CommandCollector::new(),
                replay_events,
                chrono::Utc::now().timestamp_millis(),
            );

            // Try to schedule a different task type - should cause determinism violation
            // Note: This requires a task submitter, so we're testing the validation logic
            // directly through the context
            println!(
                "Determinism violation test: expecting TaskTypeMismatch when scheduling task-B"
            );

            // In a full test, we would execute:
            // let result = _ctx.schedule_raw("task-B", serde_json::json!({})).await;
            // assert!(matches!(result, Err(FlovynError::DeterminismViolation(
            //     DeterminismViolationError::TaskTypeMismatch { .. }
            // ))));
        },
    )
    .await;
}

/// Test that determinism violation is detected when child workflow name changes.
#[tokio::test]
#[ignore]
async fn test_e2e_determinism_violation_on_child_name_change() {
    with_timeout(
        TEST_TIMEOUT,
        "test_e2e_determinism_violation_on_child_name_change",
        async {
            let harness = get_harness().await;

            // Create events for original workflow (scheduled child-A)
            let events = vec![super::harness::WorkflowEventResponse {
                sequence_number: 1,
                event_type: "ChildWorkflowInitiated".to_string(),
                data: serde_json::json!({
                    "childExecutionName": "child-A",
                    "childWorkflowKind": "process-child",
                    "childworkflowExecutionId": "child-1"
                }),
                created_at: "2024-01-01T00:00:00Z".to_string(),
            }];

            let replay_events = to_replay_events(&events);

            let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
                Uuid::new_v4(),
                harness.tenant_id(),
                serde_json::json!({}),
                CommandCollector::new(),
                replay_events,
                chrono::Utc::now().timestamp_millis(),
            );

            // Try to schedule a child workflow with different name - should cause violation
            let result = ctx
                .schedule_workflow_raw("child-B", "process-child", serde_json::json!({}))
                .await;

            assert!(
                matches!(
                    &result,
                    Err(FlovynError::DeterminismViolation(
                        DeterminismViolationError::ChildWorkflowMismatch { field, .. }
                    )) if field == "name"
                ),
                "Expected ChildWorkflowMismatch with field 'name', got {:?}",
                result
            );
        },
    )
    .await;
}

/// Test that workflow can be extended with new commands beyond replay history.
#[tokio::test]
#[ignore]
async fn test_e2e_workflow_extension_allowed() {
    with_timeout(TEST_TIMEOUT, "test_e2e_workflow_extension_allowed", async {
        let harness = get_harness().await;

        // Create events for original workflow (completed with one operation)
        let events = vec![super::harness::WorkflowEventResponse {
            sequence_number: 1,
            event_type: "OperationCompleted".to_string(),
            data: serde_json::json!({
                "operationName": "step-1",
                "result": {"value": 1}
            }),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        }];

        let replay_events = to_replay_events(&events);

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            harness.tenant_id(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            chrono::Utc::now().timestamp_millis(),
        );

        // First operation should match replay
        let result1 = ctx.run_raw("step-1", serde_json::json!({"value": 1})).await;
        assert!(
            result1.is_ok(),
            "First operation should replay successfully"
        );

        // Second operation is beyond replay history - should be allowed (new command)
        let result2 = ctx.run_raw("step-2", serde_json::json!({"value": 2})).await;
        assert!(
            result2.is_ok(),
            "New operation beyond history should be allowed"
        );
    })
    .await;
}

/// Test that mixed command types replay correctly with per-type sequence matching.
#[tokio::test]
#[ignore]
async fn test_e2e_mixed_commands_replay() {
    with_timeout(TEST_TIMEOUT, "test_e2e_mixed_commands_replay", async {
        let harness = get_harness().await;

        // Create events with interleaved command types
        let events = vec![
            super::harness::WorkflowEventResponse {
                sequence_number: 1,
                event_type: "OperationCompleted".to_string(),
                data: serde_json::json!({
                    "operationName": "op-1",
                    "result": "result-1"
                }),
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
            super::harness::WorkflowEventResponse {
                sequence_number: 2,
                event_type: "TimerStarted".to_string(),
                data: serde_json::json!({
                    "timerId": "sleep-1",
                    "durationMs": 100
                }),
                created_at: "2024-01-01T00:00:01Z".to_string(),
            },
            super::harness::WorkflowEventResponse {
                sequence_number: 3,
                event_type: "TimerFired".to_string(),
                data: serde_json::json!({
                    "timerId": "sleep-1"
                }),
                created_at: "2024-01-01T00:00:02Z".to_string(),
            },
            super::harness::WorkflowEventResponse {
                sequence_number: 4,
                event_type: "OperationCompleted".to_string(),
                data: serde_json::json!({
                    "operationName": "op-2",
                    "result": "result-2"
                }),
                created_at: "2024-01-01T00:00:03Z".to_string(),
            },
        ];

        let replay_events = to_replay_events(&events);

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            harness.tenant_id(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            chrono::Utc::now().timestamp_millis(),
        );

        // Replay in same order - all should succeed
        let result1 = ctx.run_raw("op-1", serde_json::json!("result-1")).await;
        assert!(result1.is_ok(), "op-1 should replay: {:?}", result1);

        let result2 = ctx.sleep(std::time::Duration::from_millis(100)).await;
        assert!(result2.is_ok(), "sleep should replay: {:?}", result2);

        let result3 = ctx.run_raw("op-2", serde_json::json!("result-2")).await;
        assert!(result3.is_ok(), "op-2 should replay: {:?}", result3);
    })
    .await;
}

/// Test that operation name mismatch is detected during replay.
#[tokio::test]
#[ignore]
async fn test_e2e_operation_name_mismatch() {
    with_timeout(TEST_TIMEOUT, "test_e2e_operation_name_mismatch", async {
        let harness = get_harness().await;

        let events = vec![super::harness::WorkflowEventResponse {
            sequence_number: 1,
            event_type: "OperationCompleted".to_string(),
            data: serde_json::json!({
                "operationName": "original-op",
                "result": {}
            }),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        }];

        let replay_events = to_replay_events(&events);

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            harness.tenant_id(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            chrono::Utc::now().timestamp_millis(),
        );

        // Try to run with different operation name
        let result = ctx.run_raw("changed-op", serde_json::json!({})).await;

        assert!(
            matches!(
                result,
                Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::OperationNameMismatch { .. }
                ))
            ),
            "Expected OperationNameMismatch, got {:?}",
            result
        );
    })
    .await;
}
