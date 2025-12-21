//! Client for connecting to Flovyn server

pub mod builder;
pub mod flovyn_client;
pub mod hook;

// Re-export gRPC client types from core
pub use flovyn_core::client::{
    RegistrationResult, ReportExecutionSpansResult, StartWorkflowResult as GrpcStartWorkflowResult,
    SubmitTaskResult, TaskExecutionClient, TaskExecutionInfo, WorkerLifecycleClient,
    WorkerTokenInterceptor, WorkerType, WorkflowDispatch, WorkflowEvent, WorkflowExecutionInfo,
    WorkflowQueryClient,
};

// Re-export conflict types from core worker module
pub use flovyn_core::worker::{TaskConflict, WorkflowConflict};

// Re-export high-level client types
pub use builder::{FlovynClientBuilder, DEFAULT_TASK_QUEUE};
pub use flovyn_client::{FlovynClient, StartWorkflowOptions, StartWorkflowResult, WorkerHandle};
pub use hook::{CompositeWorkflowHook, LoggingHook, NoOpHook, WorkflowHook};
