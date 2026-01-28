//! Re-exported and wrapped core types for NAPI bindings.
//!
//! This module provides NAPI-compatible versions of core types.

use napi_derive::napi;

/// Event types that can occur during workflow execution.
#[napi(string_enum)]
#[derive(Debug, PartialEq, Eq)]
pub enum NapiEventType {
    // Workflow lifecycle events
    WorkflowStarted,
    WorkflowCompleted,
    WorkflowExecutionFailed,
    WorkflowSuspended,
    CancellationRequested,

    // Operation events
    OperationCompleted,

    // State events
    StateSet,
    StateCleared,

    // Task events
    TaskScheduled,
    TaskCompleted,
    TaskFailed,
    TaskCancelled,

    // Promise events
    PromiseCreated,
    PromiseResolved,
    PromiseRejected,
    PromiseTimeout,

    // Child workflow events
    ChildWorkflowInitiated,
    ChildWorkflowStarted,
    ChildWorkflowCompleted,
    ChildWorkflowFailed,
    ChildWorkflowCancelled,

    // Timer events
    TimerStarted,
    TimerFired,
    TimerCancelled,

    // Signal events
    SignalReceived,
}

impl From<flovyn_worker_core::EventType> for NapiEventType {
    fn from(event_type: flovyn_worker_core::EventType) -> Self {
        use flovyn_worker_core::EventType;
        match event_type {
            EventType::WorkflowStarted => NapiEventType::WorkflowStarted,
            EventType::WorkflowCompleted => NapiEventType::WorkflowCompleted,
            EventType::WorkflowExecutionFailed => NapiEventType::WorkflowExecutionFailed,
            EventType::WorkflowSuspended => NapiEventType::WorkflowSuspended,
            EventType::CancellationRequested => NapiEventType::CancellationRequested,
            EventType::OperationCompleted => NapiEventType::OperationCompleted,
            EventType::StateSet => NapiEventType::StateSet,
            EventType::StateCleared => NapiEventType::StateCleared,
            EventType::TaskScheduled => NapiEventType::TaskScheduled,
            EventType::TaskCompleted => NapiEventType::TaskCompleted,
            EventType::TaskFailed => NapiEventType::TaskFailed,
            EventType::TaskCancelled => NapiEventType::TaskCancelled,
            EventType::PromiseCreated => NapiEventType::PromiseCreated,
            EventType::PromiseResolved => NapiEventType::PromiseResolved,
            EventType::PromiseRejected => NapiEventType::PromiseRejected,
            EventType::PromiseTimeout => NapiEventType::PromiseTimeout,
            EventType::ChildWorkflowInitiated => NapiEventType::ChildWorkflowInitiated,
            EventType::ChildWorkflowStarted => NapiEventType::ChildWorkflowStarted,
            EventType::ChildWorkflowCompleted => NapiEventType::ChildWorkflowCompleted,
            EventType::ChildWorkflowFailed => NapiEventType::ChildWorkflowFailed,
            EventType::ChildWorkflowCancelled => NapiEventType::ChildWorkflowCancelled,
            EventType::TimerStarted => NapiEventType::TimerStarted,
            EventType::TimerFired => NapiEventType::TimerFired,
            EventType::TimerCancelled => NapiEventType::TimerCancelled,
            EventType::SignalReceived => NapiEventType::SignalReceived,
        }
    }
}

impl From<NapiEventType> for flovyn_worker_core::EventType {
    fn from(event_type: NapiEventType) -> Self {
        use flovyn_worker_core::EventType;
        match event_type {
            NapiEventType::WorkflowStarted => EventType::WorkflowStarted,
            NapiEventType::WorkflowCompleted => EventType::WorkflowCompleted,
            NapiEventType::WorkflowExecutionFailed => EventType::WorkflowExecutionFailed,
            NapiEventType::WorkflowSuspended => EventType::WorkflowSuspended,
            NapiEventType::CancellationRequested => EventType::CancellationRequested,
            NapiEventType::OperationCompleted => EventType::OperationCompleted,
            NapiEventType::StateSet => EventType::StateSet,
            NapiEventType::StateCleared => EventType::StateCleared,
            NapiEventType::TaskScheduled => EventType::TaskScheduled,
            NapiEventType::TaskCompleted => EventType::TaskCompleted,
            NapiEventType::TaskFailed => EventType::TaskFailed,
            NapiEventType::TaskCancelled => EventType::TaskCancelled,
            NapiEventType::PromiseCreated => EventType::PromiseCreated,
            NapiEventType::PromiseResolved => EventType::PromiseResolved,
            NapiEventType::PromiseRejected => EventType::PromiseRejected,
            NapiEventType::PromiseTimeout => EventType::PromiseTimeout,
            NapiEventType::ChildWorkflowInitiated => EventType::ChildWorkflowInitiated,
            NapiEventType::ChildWorkflowStarted => EventType::ChildWorkflowStarted,
            NapiEventType::ChildWorkflowCompleted => EventType::ChildWorkflowCompleted,
            NapiEventType::ChildWorkflowFailed => EventType::ChildWorkflowFailed,
            NapiEventType::ChildWorkflowCancelled => EventType::ChildWorkflowCancelled,
            NapiEventType::TimerStarted => EventType::TimerStarted,
            NapiEventType::TimerFired => EventType::TimerFired,
            NapiEventType::TimerCancelled => EventType::TimerCancelled,
            NapiEventType::SignalReceived => EventType::SignalReceived,
        }
    }
}

/// Replay event for NAPI bindings.
#[napi(object)]
#[derive(Clone)]
pub struct NapiReplayEvent {
    /// Sequence number of this event (1-indexed).
    pub sequence_number: i32,
    /// Type of the event.
    pub event_type: NapiEventType,
    /// Event data as JSON string.
    pub data: String,
    /// Timestamp in milliseconds since Unix epoch.
    pub timestamp_ms: i64,
}

impl NapiReplayEvent {
    /// Convert to core ReplayEvent for use with ReplayEngine.
    pub fn to_replay_event(&self) -> Option<flovyn_worker_core::workflow::ReplayEvent> {
        use chrono::{DateTime, Utc};

        let data: serde_json::Value = serde_json::from_str(&self.data).ok()?;
        let timestamp = DateTime::<Utc>::from_timestamp_millis(self.timestamp_ms)
            .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        Some(flovyn_worker_core::workflow::ReplayEvent::new(
            self.sequence_number,
            self.event_type.into(),
            data,
            timestamp,
        ))
    }
}

/// Worker metrics for NAPI bindings.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct WorkerMetrics {
    /// Uptime in milliseconds.
    pub uptime_ms: i64,
    /// Current worker status.
    pub status: String,
    /// Server-assigned worker ID (if registered).
    pub worker_id: Option<String>,
    /// Total workflows processed.
    pub workflows_processed: i64,
    /// Total tasks processed.
    pub tasks_processed: i64,
    /// Currently active workflows.
    pub active_workflows: u32,
    /// Currently active tasks.
    pub active_tasks: u32,
}

/// Registration information for NAPI bindings.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    /// Server-assigned worker ID.
    pub worker_id: String,

    /// Whether registration was successful.
    pub success: bool,

    /// When the worker was registered (ms since epoch).
    pub registered_at_ms: i64,

    /// Registered workflow kinds.
    pub workflow_kinds: Vec<String>,

    /// Registered task kinds.
    pub task_kinds: Vec<String>,

    /// Whether there are any registration conflicts.
    pub has_conflicts: bool,
}

/// Connection information for NAPI bindings.
#[napi(object)]
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    /// Whether currently connected.
    pub connected: bool,

    /// Time of last successful heartbeat (ms since epoch, if any).
    pub last_heartbeat_ms: Option<i64>,

    /// Time of last successful poll (ms since epoch, if any).
    pub last_poll_ms: Option<i64>,

    /// Number of consecutive heartbeat failures.
    pub heartbeat_failures: u32,

    /// Number of consecutive poll failures.
    pub poll_failures: u32,

    /// Current reconnection attempt (if reconnecting).
    pub reconnect_attempt: Option<u32>,
}

/// Lifecycle event for worker status changes.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct LifecycleEvent {
    /// Event name (e.g., "starting", "registered", "ready", "paused", "resumed", "stopped")
    pub event_name: String,
    /// Timestamp in milliseconds since Unix epoch
    pub timestamp_ms: i64,
    /// Optional additional data as JSON string
    pub data: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_roundtrip() {
        let event_type = flovyn_worker_core::EventType::TaskCompleted;
        let napi_type: NapiEventType = event_type.into();
        let back: flovyn_worker_core::EventType = napi_type.into();
        assert_eq!(event_type, back);
    }
}
