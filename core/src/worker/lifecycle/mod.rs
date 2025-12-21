//! Worker lifecycle management types - language-agnostic components.
//!
//! This module provides core types for tracking worker status, registration info,
//! connection state, metrics, and lifecycle events.

mod events;
mod metrics;
mod reconnection;
mod types;

pub use events::{WorkType, WorkerLifecycleEvent};
pub use metrics::WorkerMetrics;
pub use reconnection::ReconnectionStrategy;
pub use types::{
    ConnectionInfo, RegistrationInfo, StopReason, TaskConflict, WorkerControlError, WorkerStatus,
    WorkflowConflict,
};
