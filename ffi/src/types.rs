//! Re-exported and wrapped core types for FFI.
//!
//! This module provides FFI-compatible versions of core types that are
//! useful for foreign SDKs but need wrapping for uniffi compatibility.

/// Event types that can occur during workflow execution.
///
/// This enum mirrors `flovyn_sdk_core::EventType` but with uniffi support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiEventType {
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
}

impl From<flovyn_sdk_core::EventType> for FfiEventType {
    fn from(event_type: flovyn_sdk_core::EventType) -> Self {
        use flovyn_sdk_core::EventType;
        match event_type {
            EventType::WorkflowStarted => FfiEventType::WorkflowStarted,
            EventType::WorkflowCompleted => FfiEventType::WorkflowCompleted,
            EventType::WorkflowExecutionFailed => FfiEventType::WorkflowExecutionFailed,
            EventType::WorkflowSuspended => FfiEventType::WorkflowSuspended,
            EventType::CancellationRequested => FfiEventType::CancellationRequested,
            EventType::OperationCompleted => FfiEventType::OperationCompleted,
            EventType::StateSet => FfiEventType::StateSet,
            EventType::StateCleared => FfiEventType::StateCleared,
            EventType::TaskScheduled => FfiEventType::TaskScheduled,
            EventType::TaskCompleted => FfiEventType::TaskCompleted,
            EventType::TaskFailed => FfiEventType::TaskFailed,
            EventType::TaskCancelled => FfiEventType::TaskCancelled,
            EventType::PromiseCreated => FfiEventType::PromiseCreated,
            EventType::PromiseResolved => FfiEventType::PromiseResolved,
            EventType::PromiseRejected => FfiEventType::PromiseRejected,
            EventType::PromiseTimeout => FfiEventType::PromiseTimeout,
            EventType::ChildWorkflowInitiated => FfiEventType::ChildWorkflowInitiated,
            EventType::ChildWorkflowStarted => FfiEventType::ChildWorkflowStarted,
            EventType::ChildWorkflowCompleted => FfiEventType::ChildWorkflowCompleted,
            EventType::ChildWorkflowFailed => FfiEventType::ChildWorkflowFailed,
            EventType::ChildWorkflowCancelled => FfiEventType::ChildWorkflowCancelled,
            EventType::TimerStarted => FfiEventType::TimerStarted,
            EventType::TimerFired => FfiEventType::TimerFired,
            EventType::TimerCancelled => FfiEventType::TimerCancelled,
        }
    }
}

impl From<FfiEventType> for flovyn_sdk_core::EventType {
    fn from(event_type: FfiEventType) -> Self {
        use flovyn_sdk_core::EventType;
        match event_type {
            FfiEventType::WorkflowStarted => EventType::WorkflowStarted,
            FfiEventType::WorkflowCompleted => EventType::WorkflowCompleted,
            FfiEventType::WorkflowExecutionFailed => EventType::WorkflowExecutionFailed,
            FfiEventType::WorkflowSuspended => EventType::WorkflowSuspended,
            FfiEventType::CancellationRequested => EventType::CancellationRequested,
            FfiEventType::OperationCompleted => EventType::OperationCompleted,
            FfiEventType::StateSet => EventType::StateSet,
            FfiEventType::StateCleared => EventType::StateCleared,
            FfiEventType::TaskScheduled => EventType::TaskScheduled,
            FfiEventType::TaskCompleted => EventType::TaskCompleted,
            FfiEventType::TaskFailed => EventType::TaskFailed,
            FfiEventType::TaskCancelled => EventType::TaskCancelled,
            FfiEventType::PromiseCreated => EventType::PromiseCreated,
            FfiEventType::PromiseResolved => EventType::PromiseResolved,
            FfiEventType::PromiseRejected => EventType::PromiseRejected,
            FfiEventType::PromiseTimeout => EventType::PromiseTimeout,
            FfiEventType::ChildWorkflowInitiated => EventType::ChildWorkflowInitiated,
            FfiEventType::ChildWorkflowStarted => EventType::ChildWorkflowStarted,
            FfiEventType::ChildWorkflowCompleted => EventType::ChildWorkflowCompleted,
            FfiEventType::ChildWorkflowFailed => EventType::ChildWorkflowFailed,
            FfiEventType::ChildWorkflowCancelled => EventType::ChildWorkflowCancelled,
            FfiEventType::TimerStarted => EventType::TimerStarted,
            FfiEventType::TimerFired => EventType::TimerFired,
            FfiEventType::TimerCancelled => EventType::TimerCancelled,
        }
    }
}

/// Worker status enum for FFI.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiWorkerStatus {
    /// Worker is initializing.
    Initializing,
    /// Worker is registering with the server.
    Registering,
    /// Worker is active and polling.
    Running {
        server_worker_id: Option<String>,
        started_at_ms: i64,
    },
    /// Worker is paused.
    Paused { reason: String },
    /// Worker is reconnecting.
    Reconnecting {
        attempts: u32,
        disconnected_at_ms: i64,
        last_error: Option<String>,
    },
    /// Worker is shutting down.
    ShuttingDown {
        requested_at_ms: i64,
        in_flight_count: u32,
    },
    /// Worker has stopped.
    Stopped {
        stopped_at_ms: i64,
        reason: FfiStopReason,
    },
}

/// Reason for worker stopping.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiStopReason {
    /// Normal graceful shutdown.
    Graceful,
    /// Immediate stop requested.
    Immediate,
    /// Aborted.
    Aborted,
    /// Unrecoverable error.
    Error { msg: String },
}

impl From<flovyn_sdk_core::StopReason> for FfiStopReason {
    fn from(reason: flovyn_sdk_core::StopReason) -> Self {
        match reason {
            flovyn_sdk_core::StopReason::Graceful => FfiStopReason::Graceful,
            flovyn_sdk_core::StopReason::Immediate => FfiStopReason::Immediate,
            flovyn_sdk_core::StopReason::Aborted => FfiStopReason::Aborted,
            flovyn_sdk_core::StopReason::Error(m) => FfiStopReason::Error { msg: m },
        }
    }
}

/// Task execution result for FFI.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiTaskExecutionResult {
    /// Task completed successfully.
    Completed {
        /// Serialized output as JSON bytes.
        output: Vec<u8>,
    },
    /// Task failed with an error.
    Failed {
        /// Error message.
        error: String,
        /// Whether this is a retryable error.
        retryable: bool,
    },
    /// Task was cancelled.
    Cancelled,
}

impl From<flovyn_sdk_core::TaskExecutionResult> for FfiTaskExecutionResult {
    fn from(result: flovyn_sdk_core::TaskExecutionResult) -> Self {
        match result {
            flovyn_sdk_core::TaskExecutionResult::Completed { output } => {
                FfiTaskExecutionResult::Completed {
                    output: serde_json::to_vec(&output).unwrap_or_default(),
                }
            }
            flovyn_sdk_core::TaskExecutionResult::Failed {
                error_message,
                is_retryable,
                ..
            } => FfiTaskExecutionResult::Failed {
                error: error_message,
                retryable: is_retryable,
            },
            flovyn_sdk_core::TaskExecutionResult::Cancelled => FfiTaskExecutionResult::Cancelled,
            flovyn_sdk_core::TaskExecutionResult::TimedOut => FfiTaskExecutionResult::Failed {
                error: "Task timed out".to_string(),
                retryable: true,
            },
        }
    }
}

/// Replay event for FFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiReplayEvent {
    /// Sequence number of this event (1-indexed).
    pub sequence_number: i32,
    /// Type of the event.
    pub event_type: FfiEventType,
    /// Event data as JSON bytes.
    pub data: Vec<u8>,
    /// Timestamp in milliseconds since Unix epoch.
    pub timestamp_ms: i64,
}

impl FfiReplayEvent {
    /// Convert to core ReplayEvent for use with ReplayEngine.
    pub fn to_replay_event(&self) -> Option<flovyn_sdk_core::workflow::ReplayEvent> {
        use chrono::{DateTime, Utc};

        let data: serde_json::Value = serde_json::from_slice(&self.data).ok()?;
        let timestamp = DateTime::<Utc>::from_timestamp_millis(self.timestamp_ms)
            .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());

        Some(flovyn_sdk_core::workflow::ReplayEvent::new(
            self.sequence_number,
            self.event_type.into(),
            data,
            timestamp,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_roundtrip() {
        let event_type = flovyn_sdk_core::EventType::TaskCompleted;
        let ffi_type: FfiEventType = event_type.into();
        let back: flovyn_sdk_core::EventType = ffi_type.into();
        assert_eq!(event_type, back);
    }

    #[test]
    fn test_stop_reason_conversion() {
        let reason = flovyn_sdk_core::StopReason::Graceful;
        let ffi_reason: FfiStopReason = reason.into();
        assert!(matches!(ffi_reason, FfiStopReason::Graceful));

        let reason = flovyn_sdk_core::StopReason::Error("test error".to_string());
        let ffi_reason: FfiStopReason = reason.into();
        assert!(matches!(ffi_reason, FfiStopReason::Error { msg } if msg == "test error"));
    }
}
