//! Child Workflow E2E tests
//!
//! These tests verify child workflow scheduling and execution against a real server.

use crate::fixtures::workflows::{ChildWorkflow, GrandparentWorkflow, ParentWithFailingChildWorkflow, ParentWorkflow, FailingChildWorkflow};
use crate::test_env::E2ETestEnvBuilder;
use crate::with_timeout;
use serde_json::json;
use std::time::Duration;

/// Test basic child workflow: parent schedules child, receives result.
///
/// This tests the flow:
/// 1. Start parent workflow that calls ctx.schedule_workflow_raw()
/// 2. Child workflow executes and completes
/// 3. Parent receives child's result and completes
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_child_workflow_success() {
    with_timeout(Duration::from_secs(60), "test_child_workflow_success", async {
        let env = E2ETestEnvBuilder::new("e2e-child-workflow-worker")
            .await
            .register_workflow(ParentWorkflow)
            .register_workflow(ChildWorkflow)
            .build_and_start()
            .await;

        // Start parent workflow with child input
        let result = env
            .start_and_await(
                "parent-workflow",
                json!({
                    "childInput": {
                        "value": 21
                    }
                }),
            )
            .await
            .expect("Workflow execution failed");

        // Parent should have the child's result (21 * 2 = 42)
        assert!(result.get("childResult").is_some(), "Expected childResult in output");
        assert_eq!(result["childResult"]["result"], json!(42));
    })
    .await;
}

/// Test child workflow failure: child fails, parent receives error.
///
/// This tests the flow:
/// 1. Start parent workflow that schedules a failing child
/// 2. Child workflow fails with an error
/// 3. Parent handles the error (or propagates it)
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_child_workflow_failure() {
    with_timeout(Duration::from_secs(60), "test_child_workflow_failure", async {
        let env = E2ETestEnvBuilder::new("e2e-failing-child-worker")
            .await
            .register_workflow(ParentWithFailingChildWorkflow)
            .register_workflow(FailingChildWorkflow::default())
            .build_and_start()
            .await;

        // Start parent workflow
        let result = env
            .start_and_await("parent-failing-child-workflow", json!({}))
            .await
            .expect("Parent workflow should complete (handling child error)");

        // Parent should report child failure
        assert_eq!(result["childSucceeded"], json!(false));
        assert!(result.get("childError").is_some(), "Expected childError in output");
    })
    .await;
}

/// Test nested child workflows: grandparent -> parent -> child.
///
/// This tests the flow:
/// 1. Start grandparent workflow
/// 2. Grandparent schedules parent workflow
/// 3. Parent schedules child workflow
/// 4. Child completes and result propagates back
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_nested_child_workflows() {
    // Use longer timeout for nested workflows
    let timeout = Duration::from_secs(90);
    with_timeout(timeout, "test_nested_child_workflows", async {
        let env = E2ETestEnvBuilder::new("e2e-nested-workflow-worker")
            .await
            .register_workflow(GrandparentWorkflow)
            .register_workflow(ParentWorkflow)
            .register_workflow(ChildWorkflow)
            .build_and_start()
            .await;

        // Start grandparent workflow with a value
        let result = env
            .start_and_await_with_timeout(
                "grandparent-workflow",
                json!({
                    "value": 10
                }),
                Duration::from_secs(60),
            )
            .await
            .expect("Workflow execution failed");

        // Result should have nested structure from parent
        // Grandparent -> Parent -> Child (value * 2 = 20)
        assert!(result.get("parentResult").is_some(), "Expected parentResult in output");
        assert!(result["parentResult"].get("childResult").is_some(), "Expected childResult in parentResult");
        assert_eq!(result["parentResult"]["childResult"]["result"], json!(20));
    })
    .await;
}
