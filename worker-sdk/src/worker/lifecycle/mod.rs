//! Worker lifecycle management types and utilities.
//!
//! This module provides types for tracking worker status, registration info,
//! connection state, metrics, and lifecycle events.

mod events;
mod hooks;
mod internals;
mod metrics;
mod reconnection;
mod types;

pub use events::{WorkType, WorkerLifecycleEvent};
pub use hooks::{HookChain, WorkerLifecycleHook};
pub use internals::WorkerInternals;
pub use metrics::WorkerMetrics;
pub use reconnection::{ReconnectionPolicy, ReconnectionStrategy};
pub use types::{
    ConnectionInfo, RegistrationInfo, StopReason, TaskConflict, WorkerControlError, WorkerStatus,
    WorkflowConflict,
};
