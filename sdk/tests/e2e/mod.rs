//! End-to-end tests for flovyn-sdk
//!
//! These tests run against a real Flovyn server using Testcontainers.
//! The test harness starts PostgreSQL, NATS, and Flovyn server containers,
//! then creates a test tenant and worker token for SDK authentication.
//!
//! # Running E2E tests
//!
//! ```bash
//! cargo test --test e2e
//! ```
//!
//! # Requirements
//!
//! - Docker must be installed and running
//! - Flovyn server Docker image must be available (`flovyn/server:latest`)

mod harness;
mod fixtures;
mod workflow_tests;

pub use harness::TestHarness;
