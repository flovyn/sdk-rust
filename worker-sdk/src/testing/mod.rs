//! Testing utilities for Flovyn workflows and tasks.
//!
//! This module provides utilities for testing workflows and tasks in isolation
//! without requiring a running Flovyn server.
//!
//! # Feature Flag
//!
//! These utilities are only available when the `testing` feature is enabled:
//!
//! ```toml
//! [dev-dependencies]
//! flovyn-sdk = { version = "0.1", features = ["testing"] }
//! ```
//!
//! # Components
//!
//! - [`MockWorkflowContext`] - Mock implementation of WorkflowContext for unit testing workflows
//! - [`MockTaskContext`] - Mock implementation of TaskContext for unit testing tasks
//! - [`TimeController`] - Control time progression in tests (advance timers, skip delays)
//! - [`TestWorkflowEnvironment`] - In-memory workflow execution environment
//! - [`WorkflowTestBuilder`] - Fluent API for setting up workflow tests
//! - [`TaskTestBuilder`] - Fluent API for setting up task tests

mod assertions;
mod builders;
mod mock_task_context;
mod mock_workflow_context;
mod test_environment;
mod time_controller;

pub use assertions::*;
pub use builders::*;
pub use mock_task_context::*;
pub use mock_workflow_context::*;
pub use test_environment::*;
pub use time_controller::*;
