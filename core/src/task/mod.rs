//! Task module - task metadata, execution utilities, and streaming
//!
//! This module provides language-agnostic types for task execution.

pub mod execution;
pub mod streaming;

pub use execution::{
    calculate_backoff, should_retry, BackoffConfig, TaskExecutionResult, TaskExecutorConfig,
    TaskMetadata,
};
pub use streaming::{StreamError, StreamEvent, StreamEventType, TaskStreamEvent};
