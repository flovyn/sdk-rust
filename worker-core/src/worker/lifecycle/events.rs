//! Worker lifecycle events.

use super::types::{RegistrationInfo, StopReason};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of work being executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkType {
    /// Workflow execution.
    Workflow,
    /// Task execution.
    Task,
}

/// Events emitted during worker lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerLifecycleEvent {
    /// Worker has started initializing.
    Starting {
        /// Worker identifier.
        worker_id: String,
        /// Worker name if configured.
        worker_name: Option<String>,
    },

    /// Worker has registered with server.
    Registered {
        /// Registration information.
        info: RegistrationInfo,
    },

    /// Registration failed.
    RegistrationFailed {
        /// Error message.
        error: String,
        /// Whether registration will be retried.
        will_retry: bool,
    },

    /// Worker is now polling for work.
    Ready {
        /// Server-assigned worker ID.
        server_worker_id: Option<Uuid>,
    },

    /// Worker received work.
    WorkReceived {
        /// Type of work received.
        work_type: WorkType,
        /// Execution ID.
        execution_id: Uuid,
    },

    /// Work completed successfully.
    WorkCompleted {
        /// Type of work completed.
        work_type: WorkType,
        /// Execution ID.
        execution_id: Uuid,
        /// Duration of execution in milliseconds.
        duration_ms: u64,
    },

    /// Work failed.
    WorkFailed {
        /// Type of work that failed.
        work_type: WorkType,
        /// Execution ID.
        execution_id: Uuid,
        /// Error message.
        error: String,
    },

    /// Connection to server lost.
    Disconnected {
        /// Error that caused disconnection.
        error: String,
    },

    /// Attempting to reconnect.
    Reconnecting {
        /// Current reconnection attempt number.
        attempt: u32,
    },

    /// Successfully reconnected.
    Reconnected,

    /// Heartbeat sent successfully.
    HeartbeatSent,

    /// Heartbeat failed.
    HeartbeatFailed {
        /// Error message.
        error: String,
        /// Number of consecutive failures.
        consecutive_failures: u32,
    },

    /// Worker paused.
    Paused {
        /// Reason for pausing.
        reason: String,
    },

    /// Worker resumed.
    Resumed,

    /// Shutdown requested.
    ShuttingDown {
        /// Whether this is a graceful shutdown.
        graceful: bool,
    },

    /// Worker has stopped.
    Stopped {
        /// Reason for stopping.
        reason: StopReason,
        /// Total uptime in milliseconds.
        uptime_ms: u64,
    },
}

impl WorkerLifecycleEvent {
    /// Returns true if this is an error event.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Self::RegistrationFailed { .. }
                | Self::WorkFailed { .. }
                | Self::Disconnected { .. }
                | Self::HeartbeatFailed { .. }
        )
    }

    /// Returns the event name as a string.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Starting { .. } => "starting",
            Self::Registered { .. } => "registered",
            Self::RegistrationFailed { .. } => "registration_failed",
            Self::Ready { .. } => "ready",
            Self::WorkReceived { .. } => "work_received",
            Self::WorkCompleted { .. } => "work_completed",
            Self::WorkFailed { .. } => "work_failed",
            Self::Disconnected { .. } => "disconnected",
            Self::Reconnecting { .. } => "reconnecting",
            Self::Reconnected => "reconnected",
            Self::HeartbeatSent => "heartbeat_sent",
            Self::HeartbeatFailed { .. } => "heartbeat_failed",
            Self::Paused { .. } => "paused",
            Self::Resumed => "resumed",
            Self::ShuttingDown { .. } => "shutting_down",
            Self::Stopped { .. } => "stopped",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_work_type_equality() {
        assert_eq!(WorkType::Workflow, WorkType::Workflow);
        assert_eq!(WorkType::Task, WorkType::Task);
        assert_ne!(WorkType::Workflow, WorkType::Task);
    }

    #[test]
    fn test_work_type_clone() {
        let work_type = WorkType::Workflow;
        let cloned = work_type;
        assert_eq!(work_type, cloned);
    }

    #[test]
    fn test_event_is_error() {
        assert!(WorkerLifecycleEvent::RegistrationFailed {
            error: "test".to_string(),
            will_retry: false,
        }
        .is_error());

        assert!(WorkerLifecycleEvent::WorkFailed {
            work_type: WorkType::Workflow,
            execution_id: Uuid::nil(),
            error: "test".to_string(),
        }
        .is_error());

        assert!(WorkerLifecycleEvent::Disconnected {
            error: "test".to_string(),
        }
        .is_error());

        assert!(WorkerLifecycleEvent::HeartbeatFailed {
            error: "test".to_string(),
            consecutive_failures: 1,
        }
        .is_error());

        assert!(!WorkerLifecycleEvent::HeartbeatSent.is_error());
        assert!(!WorkerLifecycleEvent::Reconnected.is_error());
    }

    #[test]
    fn test_event_name() {
        assert_eq!(
            WorkerLifecycleEvent::Starting {
                worker_id: "test".to_string(),
                worker_name: None,
            }
            .event_name(),
            "starting"
        );

        assert_eq!(
            WorkerLifecycleEvent::HeartbeatSent.event_name(),
            "heartbeat_sent"
        );
        assert_eq!(
            WorkerLifecycleEvent::Reconnected.event_name(),
            "reconnected"
        );
    }

    #[test]
    fn test_event_clone() {
        let event = WorkerLifecycleEvent::WorkCompleted {
            work_type: WorkType::Task,
            execution_id: Uuid::new_v4(),
            duration_ms: 100,
        };
        let cloned = event.clone();
        assert_eq!(cloned.event_name(), "work_completed");
    }

    #[test]
    fn test_event_serde() {
        let event = WorkerLifecycleEvent::WorkReceived {
            work_type: WorkType::Task,
            execution_id: Uuid::nil(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: WorkerLifecycleEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_name(), "work_received");
    }
}
