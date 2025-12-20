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

mod child_workflow_tests;
mod comprehensive_tests;
mod concurrency_tests;
mod error_tests;
mod fixtures;
mod harness;
mod parallel_tests;
mod promise_tests;
mod replay_tests;
mod replay_utils;
mod state_tests;
mod task_tests;
mod test_env;
mod timer_tests;
mod workflow_tests;

pub use harness::{TestHarness, WorkflowEventResponse, WorkflowExecutionResponse};
pub use replay_utils::{
    create_replay_context, parse_event_type, to_replay_event, to_replay_events,
};
pub use test_env::{E2ETestEnvBuilder, E2ETestEnvironment, WorkflowResult, WorkflowStatus};

use harness::register_atexit_handler;
use std::time::Duration;
use tokio::sync::OnceCell;

/// Default timeout for E2E tests (60 seconds per test)
pub const TEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Global shared harness for all E2E tests.
/// This ensures all tests share the same server container.
static GLOBAL_HARNESS: OnceCell<TestHarness> = OnceCell::const_new();

/// Initialize tracing once for all tests
static TRACING_INITIALIZED: std::sync::Once = std::sync::Once::new();

fn init_tracing() {
    TRACING_INITIALIZED.call_once(|| {
        use tracing_subscriber::{fmt, EnvFilter};

        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .init();
    });
}

/// Get or initialize the global test harness.
pub async fn get_harness() -> &'static TestHarness {
    // Initialize tracing before anything else
    init_tracing();

    // Register atexit handler before creating any containers
    register_atexit_handler();

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
