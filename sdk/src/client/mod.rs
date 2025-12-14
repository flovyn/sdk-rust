//! Client for connecting to Flovyn server

pub mod auth;
pub mod builder;
pub mod flovyn_client;
pub mod hook;
pub mod task_execution;
pub mod worker_lifecycle;
pub mod workflow_dispatch;
pub mod workflow_query;

// Re-export gRPC client types
pub use task_execution::{SubmitTaskResult, TaskExecutionClient, TaskExecutionInfo};
pub use workflow_dispatch::StartWorkflowResult as GrpcStartWorkflowResult;
pub use workflow_dispatch::{WorkflowDispatch, WorkflowEvent, WorkflowExecutionInfo};
pub use workflow_query::WorkflowQueryClient;

// Re-export high-level client types
pub use auth::WorkerTokenInterceptor;
pub use builder::{FlovynClientBuilder, DEFAULT_TASK_QUEUE};
pub use flovyn_client::{FlovynClient, StartWorkflowOptions, StartWorkflowResult, WorkerHandle};
pub use hook::{CompositeWorkflowHook, LoggingHook, NoOpHook, WorkflowHook};
pub use worker_lifecycle::{
    RegistrationResult, TaskConflict, WorkerLifecycleClient, WorkerType, WorkflowConflict,
};
