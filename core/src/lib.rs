//! # Flovyn Core
//!
//! Language-agnostic core library for the Flovyn workflow orchestration platform.
//!
//! This crate provides the foundational building blocks for workflow execution,
//! replay, and determinism validation. It is designed to be consumed by
//! language-specific SDKs (Rust, Kotlin, Python, etc.).
//!
//! ## What's in Core vs SDK
//!
//! **Core** contains language-agnostic components:
//! - Protocol buffer definitions and generated code
//! - Workflow commands and replay events
//! - Determinism validation logic
//! - Task metadata and execution result types
//! - Worker lifecycle status, events, and metrics
//! - Reconnection strategies
//! - gRPC client wrappers (task execution, workflow dispatch, worker lifecycle, query)
//! - Task streaming types
//!
//! **SDK** contains language-specific components:
//! - Async traits (WorkflowDefinition, TaskDefinition)
//! - WorkflowContext and TaskContext implementations
//! - Worker executor and polling coordination
//! - Testing utilities
//! - Workflow hooks for observability
//!
//! ## Modules
//!
//! - [`generated`] - gRPC/protobuf generated code
//! - [`workflow`] - Workflow commands, events, execution utilities, and recording
//! - [`task`] - Task metadata, execution utilities, and streaming types
//! - [`worker`] - Determinism validation and lifecycle types
//! - [`client`] - gRPC client wrappers for Flovyn services
//! - [`error`] - Core error types

pub mod client;
pub mod error;
pub mod generated;
pub mod task;
pub mod worker;
pub mod workflow;

// Re-export error types
pub use error::{CoreError, CoreResult, DeterminismViolationError};

// Re-export task types
pub use task::{
    calculate_backoff, should_retry, BackoffConfig, TaskExecutionResult, TaskExecutorConfig,
    TaskMetadata,
};

// Re-export worker types
pub use worker::{
    ConnectionInfo, DeterminismValidationResult, DeterminismValidator, ReconnectionStrategy,
    RegistrationInfo, StopReason, TaskConflict, WorkType, WorkerControlError, WorkerLifecycleEvent,
    WorkerMetrics, WorkerStatus, WorkflowConflict,
};

// Re-export workflow types
pub use workflow::{
    build_initial_state, build_operation_cache, CommandCollector, CommandRecorder,
    DeterministicRandom, EventLookup, EventType, ReplayEvent, SeededRandom,
    ValidatingCommandRecorder, WorkflowCommand, WorkflowMetadata,
};

// Re-export client types
pub use client::{
    RegistrationResult, ReportExecutionSpansResult, StartWorkflowResult, SubmitTaskResult,
    TaskExecutionClient, TaskExecutionInfo, WorkerLifecycleClient, WorkerTokenInterceptor,
    WorkerType, WorkflowDispatch, WorkflowEvent, WorkflowExecutionInfo, WorkflowQueryClient,
};

// Re-export task streaming types
pub use task::{StreamError, StreamEvent, StreamEventType, TaskStreamEvent};
