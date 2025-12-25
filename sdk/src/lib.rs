//! # Flovyn SDK for Rust
//!
//! This SDK provides workflow and task execution capabilities for Flovyn,
//! enabling Rust applications to define workflows, execute tasks, and
//! communicate with the Flovyn server via gRPC.
//!
//! ## Architecture
//!
//! The SDK is built on top of `flovyn-core`, which provides:
//! - Protocol buffer definitions and generated code
//! - gRPC client wrappers for server communication
//! - Workflow commands, events, and replay logic
//! - Task metadata and streaming types
//! - Worker lifecycle types and determinism validation
//!
//! The SDK adds Rust-specific functionality:
//! - [`WorkflowDefinition`] and [`TaskDefinition`] traits for type-safe definitions
//! - [`WorkflowContext`] and [`TaskContext`] for execution APIs
//! - Worker executors for polling and task/workflow execution
//! - [`FlovynClient`] builder pattern for easy setup
//! - Workflow hooks for observability
//! - Testing utilities (with `testing` feature)
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use flovyn_sdk::prelude::*;
//!
//! // Define a workflow
//! struct MyWorkflow;
//!
//! #[async_trait]
//! impl WorkflowDefinition for MyWorkflow {
//!     type Input = String;
//!     type Output = String;
//!
//!     fn kind(&self) -> &str { "my-workflow" }
//!
//!     async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
//!         Ok(format!("Hello, {}!", input))
//!     }
//! }
//!
//! // Build client and start worker
//! let client = FlovynClientBuilder::new("http://localhost:9090", "fwt_token")
//!     .tenant_id(tenant_id)
//!     .register_workflow(MyWorkflow)
//!     .build()
//!     .await?;
//!
//! let handle = client.start_worker().await?;
//! ```
//!
//! ## Modules
//!
//! - [`client`] - Client builder, hooks, and high-level API
//! - [`workflow`] - Workflow definitions, context, and commands
//! - [`task`] - Task definitions, context, and streaming
//! - [`worker`] - Worker executors and lifecycle management
//! - [`config`] - Configuration types
//! - [`error`] - Error types

#![allow(clippy::result_large_err)]

pub mod client;
pub mod common;
pub mod config;
pub mod error;
pub mod task;
pub mod telemetry;
pub mod worker;
pub mod workflow;

// Re-export generated module from core
pub use flovyn_core::generated;

/// Testing utilities for workflows and tasks.
/// Available only with the `testing` feature enabled.
#[cfg(feature = "testing")]
pub mod testing;

// Re-export commonly used types
pub use common::version::SemanticVersion;
pub use error::{DeterminismViolationError, FlovynError, Result};

// Re-export config types
pub use config::{
    FlovynClientConfig, TaskExecutorConfig as ClientTaskExecutorConfig,
    WorkflowExecutorConfig as ClientWorkflowExecutorConfig,
};

// Re-export client types
pub use client::{
    FlovynClient, FlovynClientBuilder, StartWorkflowOptions, StartWorkflowResult, WorkerHandle,
};
pub use client::{LoggingHook, NoOpHook, WorkflowHook};

// Re-export workflow types
pub use workflow::command::WorkflowCommand;
pub use workflow::context::{ScheduleTaskOptions, WorkflowContext, WorkflowContextExt};
pub use workflow::context_impl::WorkflowContextImpl;
pub use workflow::definition::WorkflowDefinition;
pub use workflow::event::{EventType, ReplayEvent};
pub use workflow::recorder::{CommandCollector, CommandRecorder, ValidatingCommandRecorder};

// Re-export task types
pub use task::context::{LogLevel, TaskContext};
pub use task::context_impl::TaskContextImpl;
pub use task::definition::{DynamicTask, RetryConfig, TaskDefinition};
pub use task::executor::{
    TaskExecutionResult, TaskExecutor, TaskExecutorCallbacks, TaskExecutorConfig,
};
pub use task::registry::{RegisteredTask, TaskMetadata, TaskRegistry};
pub use task::streaming::{StreamError, StreamEvent, StreamEventType, TaskStreamEvent};

// Re-export worker types
pub use worker::determinism::{DeterminismValidationResult, DeterminismValidator};
pub use worker::executor::{
    WorkflowExecutor, WorkflowExecutorConfig, WorkflowStatus, WorkflowTaskResult,
};
pub use worker::lifecycle::{
    ConnectionInfo, HookChain, ReconnectionPolicy, ReconnectionStrategy, RegistrationInfo,
    StopReason, TaskConflict, WorkType, WorkerControlError, WorkerInternals, WorkerLifecycleEvent,
    WorkerLifecycleHook, WorkerMetrics, WorkerStatus, WorkflowConflict,
};
pub use worker::registry::{RegisteredWorkflow, WorkflowMetadata, WorkflowRegistry};
pub use worker::task_worker::{TaskExecutorWorker, TaskWorkerConfig};
pub use worker::workflow_worker::{WorkflowExecutorWorker, WorkflowWorkerConfig};

// Re-export telemetry types
pub use telemetry::{RecordingSpan, SpanCollector};

/// Prelude module for convenient imports
pub mod prelude {
    #[cfg(feature = "oauth2")]
    pub use crate::client::OAuth2Credentials;
    pub use crate::client::{
        FlovynClient, FlovynClientBuilder, StartWorkflowOptions, StartWorkflowResult, WorkerHandle,
    };
    pub use crate::client::{LoggingHook, NoOpHook, WorkflowHook};
    pub use crate::common::version::SemanticVersion;
    pub use crate::config::{
        FlovynClientConfig, TaskExecutorConfig as ClientTaskExecutorConfig,
        WorkflowExecutorConfig as ClientWorkflowExecutorConfig,
    };
    pub use crate::error::{DeterminismViolationError, FlovynError, Result};
    pub use crate::task::context::{LogLevel, TaskContext};
    pub use crate::task::context_impl::TaskContextImpl;
    pub use crate::task::definition::{DynamicTask, RetryConfig, TaskDefinition};
    pub use crate::task::executor::{
        TaskExecutionResult, TaskExecutor, TaskExecutorCallbacks, TaskExecutorConfig,
    };
    pub use crate::task::registry::{RegisteredTask, TaskMetadata, TaskRegistry};
    pub use crate::task::streaming::{StreamError, StreamEvent, StreamEventType, TaskStreamEvent};
    pub use crate::telemetry::{RecordingSpan, SpanCollector};
    pub use crate::worker::determinism::{DeterminismValidationResult, DeterminismValidator};
    pub use crate::worker::executor::{
        WorkflowExecutor, WorkflowExecutorConfig, WorkflowStatus, WorkflowTaskResult,
    };
    pub use crate::worker::lifecycle::{
        ConnectionInfo, HookChain, ReconnectionPolicy, ReconnectionStrategy, RegistrationInfo,
        StopReason, TaskConflict, WorkType, WorkerControlError, WorkerInternals,
        WorkerLifecycleEvent, WorkerLifecycleHook, WorkerMetrics, WorkerStatus, WorkflowConflict,
    };
    pub use crate::worker::registry::{RegisteredWorkflow, WorkflowMetadata, WorkflowRegistry};
    pub use crate::worker::task_worker::{TaskExecutorWorker, TaskWorkerConfig};
    pub use crate::worker::workflow_worker::{WorkflowExecutorWorker, WorkflowWorkerConfig};
    pub use crate::workflow::command::WorkflowCommand;
    pub use crate::workflow::context::{ScheduleTaskOptions, WorkflowContext, WorkflowContextExt};
    pub use crate::workflow::context_impl::WorkflowContextImpl;
    pub use crate::workflow::definition::{DynamicWorkflow, WorkflowDefinition};
    pub use crate::workflow::event::{EventType, ReplayEvent};
    pub use crate::workflow::recorder::{
        CommandCollector, CommandRecorder, ValidatingCommandRecorder,
    };
    pub use async_trait::async_trait;
    pub use serde::{Deserialize, Serialize};
    pub use serde_json::{json, Map, Value};
    pub use uuid::Uuid;
}
