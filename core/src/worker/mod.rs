//! Worker module - lifecycle, polling, and determinism validation

pub mod determinism;
pub mod lifecycle;

pub use determinism::{DeterminismValidationResult, DeterminismValidator};
pub use lifecycle::{
    ConnectionInfo, ReconnectionStrategy, RegistrationInfo, StopReason, TaskConflict, WorkType,
    WorkerControlError, WorkerLifecycleEvent, WorkerMetrics, WorkerStatus, WorkflowConflict,
};
