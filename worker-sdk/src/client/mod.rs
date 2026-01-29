//! Client for connecting to Flovyn server

pub mod builder;
pub mod flovyn_client;
pub mod hook;

// Re-export gRPC client types from core
pub use flovyn_worker_core::client::{
    AuthInterceptor, RegistrationResult, ReportExecutionSpansResult,
    StartWorkflowResult as GrpcStartWorkflowResult, SubmitTaskResult, TaskExecutionClient,
    TaskExecutionInfo, WorkerLifecycleClient, WorkerType, WorkflowDispatch, WorkflowEvent,
    WorkflowExecutionInfo, WorkflowQueryClient,
};

// Re-export conflict types from core worker module
pub use flovyn_worker_core::worker::{TaskConflict, WorkflowConflict};

// Re-export high-level client types
pub use builder::{FlovynClientBuilder, DEFAULT_TASK_QUEUE};
pub use flovyn_client::{
    FlovynClient, SignalResult, SignalWithStartOptions, SignalWithStartResult, StartWorkflowOptions,
    StartWorkflowResult, WorkerHandle,
};
pub use hook::{CompositeWorkflowHook, LoggingHook, NoOpHook, WorkflowHook};

// Re-export OAuth2 types from core when feature is enabled
#[cfg(feature = "oauth2")]
pub use flovyn_worker_core::client::OAuth2Credentials;
