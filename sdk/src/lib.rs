//! Flovyn SDK for Rust
//!
//! This SDK provides workflow and task execution capabilities for Flovyn,
//! enabling Rust applications to define workflows, execute tasks, and
//! communicate with the Flovyn server via gRPC.

#![allow(clippy::result_large_err)]

pub mod client;
pub mod common;
pub mod config;
pub mod error;
pub mod generated;
pub mod task;
pub mod telemetry;
pub mod worker;
pub mod workflow;

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
