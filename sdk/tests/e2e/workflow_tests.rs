//! Workflow E2E tests
//!
//! These tests verify workflow execution against a real Flovyn server.

use crate::harness::TestHarness;

/// Test that the test harness can start all containers and create a tenant.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_harness_setup() {
    let harness = TestHarness::new().await;

    assert!(!harness.tenant_id().is_nil());
    assert!(!harness.tenant_slug().is_empty());
    assert!(harness.worker_token().starts_with("fwt_"));
    assert!(harness.grpc_port() > 0);
    assert!(harness.http_port() > 0);
}

// TODO: Add more workflow tests once the SDK client is integrated
//
// #[tokio::test]
// #[ignore]
// async fn test_simple_workflow_execution() {
//     let harness = TestHarness::new().await;
//
//     // Build SDK client with worker token
//     let client = FlovynClient::builder()
//         .server_address(harness.grpc_host(), harness.grpc_port())
//         .tenant_id(harness.tenant_id())
//         .worker_token(harness.worker_token())
//         .register_workflow(DoublerWorkflow)
//         .build();
//
//     // Start the worker
//     client.start().await.unwrap();
//
//     // Trigger workflow and wait for completion
//     let result = client
//         .execute_workflow("doubler-workflow", json!({"value": 21}))
//         .await
//         .unwrap();
//
//     assert_eq!(result["result"], 42);
//
//     client.stop().await.unwrap();
// }
