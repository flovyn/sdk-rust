//! Comprehensive E2E tests
//!
//! These tests exercise multiple SDK features in single workflow executions
//! to minimize Docker rebuild overhead. Each comprehensive test validates
//! multiple features in one go.

use crate::fixtures::tasks::EchoTask;
use crate::fixtures::workflows::{
    ComprehensiveWithTaskWorkflow, ComprehensiveWorkflow, DoublerWorkflow, EchoWorkflow,
    FailingWorkflow, StatefulWorkflow,
};
use crate::test_env::E2ETestEnvBuilder;
use crate::{with_timeout, TEST_TIMEOUT};
use serde_json::json;
use std::time::Duration;

/// Single comprehensive test that validates:
/// - Basic workflow execution
/// - Input/output handling
/// - Operation recording (ctx.run_raw)
/// - State set/get operations
/// - Multiple operations in sequence
///
/// This test replaces multiple smaller tests to reduce Docker rebuild overhead.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_comprehensive_workflow_features() {
    with_timeout(
        TEST_TIMEOUT,
        "test_comprehensive_workflow_features",
        async {
            let env = E2ETestEnvBuilder::with_task_queue("e2e-comprehensive-worker", "comprehensive-features-queue")
                .await
                .register_workflow(ComprehensiveWorkflow)
                .build_and_start()
                .await;

            let result = env
                .start_and_await("comprehensive-workflow", json!({"value": 21}))
                .await
                .expect("Workflow execution failed");

            // Validate all features tested by the comprehensive workflow
            assert_eq!(result["inputValue"], 21, "Basic input should work");
            assert_eq!(result["runResult"], 42, "ctx.run_raw should record operation");
            assert_eq!(result["stateSet"], true, "State set should succeed");
            assert_eq!(
                result["stateMatches"], true,
                "State get should return what was set"
            );
            assert_eq!(result["tripleResult"], 63, "Multiple operations should work");
            assert_eq!(
                result["testsPassedCount"], 5,
                "All 5 feature tests should pass"
            );

            // Verify specific state content
            let state_retrieved = &result["stateRetrieved"];
            assert_eq!(state_retrieved["counter"], 21);
            assert_eq!(state_retrieved["message"], "state test");
            assert_eq!(state_retrieved["nested"]["a"], 1);
            assert_eq!(state_retrieved["nested"]["b"], 2);

            println!("Comprehensive test passed all {} checks", result["testsPassedCount"]);
        },
    )
    .await;
}

/// Test that validates task scheduling alongside other features.
/// Tests: basic input, state, operations, and task scheduling.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_comprehensive_with_task_scheduling() {
    // Use longer timeout for task-based test
    let timeout = Duration::from_secs(90);
    with_timeout(timeout, "test_comprehensive_with_task_scheduling", async {
        let env = E2ETestEnvBuilder::with_task_queue("e2e-comprehensive-task-worker", "comprehensive-task-queue")
            .await
            .register_workflow(ComprehensiveWithTaskWorkflow)
            .register_task(EchoTask)
            .build_and_start()
            .await;

        let result = env
            .start_and_await_with_timeout(
                "comprehensive-with-task-workflow",
                json!({"value": 50}),
                Duration::from_secs(60),
            )
            .await
            .expect("Workflow execution failed");

        // Validate all features
        assert_eq!(result["inputValue"], 50, "Basic input should work");
        assert_eq!(result["opResult"], 100, "Operation should double value");

        // Validate task result
        let task_result = &result["taskResult"];
        assert_eq!(
            task_result["message"], "Task for value 50",
            "Task should echo message"
        );
        assert_eq!(task_result["number"], 50, "Task should echo number");

        // Validate state persistence across task suspension
        let state_after = &result["stateAfterTask"];
        assert_eq!(state_after["step"], 1, "State should persist after task");

        assert_eq!(
            result["testsPassedCount"], 5,
            "All 5 feature tests should pass"
        );

        println!(
            "Comprehensive task test passed all {} checks",
            result["testsPassedCount"]
        );
    })
    .await;
}

/// All-in-one test that runs multiple workflow types in sequence.
/// This is the most efficient test as it reuses the same worker for multiple workflows.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_all_basic_workflows() {
    let timeout = Duration::from_secs(120);
    with_timeout(timeout, "test_all_basic_workflows", async {
        let env = E2ETestEnvBuilder::with_task_queue("e2e-all-workflows-worker", "comprehensive-all-queue")
            .await
            .register_workflow(DoublerWorkflow)
            .register_workflow(EchoWorkflow)
            .register_workflow(StatefulWorkflow)
            .register_workflow(FailingWorkflow::new("Test error"))
            .build_and_start()
            .await;

        // Test 1: Doubler workflow
        println!("Testing DoublerWorkflow...");
        let doubler_result = env
            .start_and_await("doubler-workflow", json!({"value": 21}))
            .await
            .expect("Doubler workflow failed");
        assert_eq!(doubler_result["result"], 42, "Doubler should return 42");
        println!("  ✓ DoublerWorkflow passed");

        // Test 2: Echo workflow
        println!("Testing EchoWorkflow...");
        let echo_input = json!({
            "message": "Hello",
            "count": 123,
            "nested": {"key": "value"}
        });
        let echo_result = env
            .start_and_await("echo-workflow", echo_input.clone())
            .await
            .expect("Echo workflow failed");
        assert_eq!(echo_result, echo_input, "Echo should return input unchanged");
        println!("  ✓ EchoWorkflow passed");

        // Test 3: Stateful workflow
        println!("Testing StatefulWorkflow...");
        let state_result = env
            .start_and_await(
                "stateful-workflow",
                json!({
                    "key": "my-key",
                    "value": {"data": "test", "number": 42}
                }),
            )
            .await
            .expect("Stateful workflow failed");
        assert_eq!(
            state_result["stored"], state_result["retrieved"],
            "State should be retrievable"
        );
        assert_eq!(
            state_result["stored"]["data"], "test",
            "State data should match"
        );
        println!("  ✓ StatefulWorkflow passed");

        // Test 4: Failing workflow (test error handling)
        println!("Testing FailingWorkflow...");
        let workflow_id = env.start_workflow("failing-workflow", json!({})).await;

        // Wait for failure event
        let event = env
            .await_event(
                workflow_id,
                "WORKFLOW_EXECUTION_FAILED",
                Duration::from_secs(10),
            )
            .await
            .expect("Should have failure event");

        let error_msg = event
            .payload
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("");
        assert!(
            error_msg.contains("Test error"),
            "Error should contain message"
        );
        println!("  ✓ FailingWorkflow passed");

        println!("\n=== All 4 basic workflow tests passed! ===");
    })
    .await;
}
