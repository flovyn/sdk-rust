//! End-to-end tests for flovyn-sdk
//!
//! These tests run against a real Flovyn server using Testcontainers.
//! The test harness starts PostgreSQL, NATS, and Flovyn server containers,
//! then creates a test tenant and worker token for SDK authentication.
//!
//! # Running E2E tests
//!
//! ```bash
//! # Run all E2E tests (requires Docker and dev infrastructure)
//! FLOVYN_E2E_USE_DEV_INFRA=1 cargo test --test e2e -p flovyn-sdk -- --include-ignored --test-threads=1
//! ```
//!
//! # Requirements
//!
//! - Docker must be installed and running
//! - Flovyn server Docker image must be available (`flovyn-server-test:latest`)
//! - Dev infrastructure running (PostgreSQL on 5435, NATS on 4222)

mod harness;
mod fixtures;
mod workflow_tests;
mod task_tests;
mod state_tests;
mod timer_tests;
mod promise_tests;
mod child_workflow_tests;
mod error_tests;
mod concurrency_tests;

pub use harness::TestHarness;

use std::time::Duration;
use tokio::sync::OnceCell;

/// Default timeout for E2E tests (60 seconds per test)
pub const TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Global shared harness for all E2E tests.
/// This ensures all tests share the same server container.
static GLOBAL_HARNESS: OnceCell<TestHarness> = OnceCell::const_new();

/// Get or initialize the global test harness.
pub async fn get_harness() -> &'static TestHarness {
    GLOBAL_HARNESS
        .get_or_init(|| async { TestHarness::new().await })
        .await
}

/// Run a test with a timeout. Panics if the test takes longer than the specified duration.
pub async fn with_timeout<F, T>(timeout: Duration, test_name: &str, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match tokio::time::timeout(timeout, f).await {
        Ok(result) => result,
        Err(_) => panic!("Test '{}' timed out after {:?}", test_name, timeout),
    }
}
