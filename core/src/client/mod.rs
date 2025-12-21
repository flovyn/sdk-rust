//! gRPC client wrappers for Flovyn services.
//!
//! This module provides authenticated gRPC clients for interacting with
//! the Flovyn server. All clients use worker token authentication.

mod auth;
mod task_execution;
mod worker_lifecycle;
mod workflow_dispatch;
mod workflow_query;

pub use auth::WorkerTokenInterceptor;
pub use task_execution::{SubmitTaskResult, TaskExecutionClient, TaskExecutionInfo};
pub use worker_lifecycle::{RegistrationResult, WorkerLifecycleClient, WorkerType};
pub use workflow_dispatch::{
    ReportExecutionSpansResult, StartWorkflowResult, WorkflowDispatch, WorkflowEvent,
    WorkflowExecutionInfo,
};
pub use workflow_query::WorkflowQueryClient;
