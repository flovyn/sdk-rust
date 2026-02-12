//! gRPC client wrappers for Flovyn services.
//!
//! This module provides authenticated gRPC clients for interacting with
//! the Flovyn server. All clients use worker token authentication.

mod agent_dispatch;
mod auth;
mod task_execution;
mod worker_lifecycle;
mod workflow_dispatch;
mod workflow_query;

#[cfg(feature = "oauth2")]
pub mod oauth2;

pub use agent_dispatch::{
    AgentCheckpoint, AgentDispatch, AgentEntry, AgentExecutionInfo, AgentSignal,
    AppendEntryResult, BatchScheduleTaskInput, BatchScheduleTaskResultEntry, BatchTaskResult,
    CancelResult, ScheduleTaskResult as AgentScheduleTaskResult, SignalResult,
    TaskResult as AgentTaskResult, TokenUsage as AgentTokenUsage, WaitMode,
};
pub use auth::AuthInterceptor;
pub use task_execution::{SubmitTaskResult, TaskExecutionClient, TaskExecutionInfo};
pub use worker_lifecycle::{RegistrationResult, WorkerLifecycleClient, WorkerType};
pub use workflow_dispatch::{
    ReportExecutionSpansResult, StartWorkflowResult, WorkflowDispatch, WorkflowEvent,
    WorkflowExecutionInfo,
};
pub use workflow_query::WorkflowQueryClient;

#[cfg(feature = "oauth2")]
pub use oauth2::{fetch_access_token, CachedToken, OAuth2Credentials, TokenResponse};
