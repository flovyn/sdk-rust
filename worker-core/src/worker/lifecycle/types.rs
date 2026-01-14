//! Core types for worker lifecycle management.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use thiserror::Error;
use uuid::Uuid;

/// Current operational status of the worker.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WorkerStatus {
    /// Worker is initializing, not yet polling.
    #[default]
    Initializing,

    /// Worker is registering with the server.
    Registering,

    /// Worker is active and polling for work.
    Running {
        /// Server-assigned worker ID.
        server_worker_id: Option<Uuid>,
        /// When the worker started running (Unix timestamp in milliseconds).
        #[serde(with = "system_time_serde")]
        started_at: SystemTime,
    },

    /// Worker is connected but temporarily paused.
    Paused {
        /// Reason for pausing.
        reason: String,
    },

    /// Worker is attempting to reconnect after connection loss.
    Reconnecting {
        /// Number of reconnection attempts.
        attempts: u32,
        /// When connection was lost (Unix timestamp in milliseconds).
        #[serde(with = "system_time_serde")]
        disconnected_at: SystemTime,
        /// Last error message.
        last_error: Option<String>,
    },

    /// Worker is shutting down gracefully.
    ShuttingDown {
        /// When shutdown was requested (Unix timestamp in milliseconds).
        #[serde(with = "system_time_serde")]
        requested_at: SystemTime,
        /// Number of in-flight tasks/workflows.
        in_flight_count: usize,
    },

    /// Worker has stopped.
    Stopped {
        /// When the worker stopped (Unix timestamp in milliseconds).
        #[serde(with = "system_time_serde")]
        stopped_at: SystemTime,
        /// Reason for stopping.
        reason: StopReason,
    },
}

mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        serializer.serialize_u64(duration.as_millis() as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + Duration::from_millis(millis))
    }
}

/// Reason for worker stopping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// Normal graceful shutdown.
    Graceful,
    /// Immediate stop requested.
    Immediate,
    /// Aborted.
    Aborted,
    /// Unrecoverable error.
    Error(String),
}

/// Information about worker registration with the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationInfo {
    /// Server-assigned worker ID.
    pub worker_id: Uuid,

    /// Whether registration was successful.
    pub success: bool,

    /// When the worker was registered (Unix timestamp in milliseconds).
    pub registered_at_ms: u64,

    /// Registered workflow kinds.
    pub workflow_kinds: Vec<String>,

    /// Registered task kinds.
    pub task_kinds: Vec<String>,

    /// Any workflow registration conflicts.
    pub workflow_conflicts: Vec<WorkflowConflict>,

    /// Any task registration conflicts.
    pub task_conflicts: Vec<TaskConflict>,
}

impl RegistrationInfo {
    /// Returns true if there are any registration conflicts.
    pub fn has_conflicts(&self) -> bool {
        !self.workflow_conflicts.is_empty() || !self.task_conflicts.is_empty()
    }

    /// Get registered_at as SystemTime
    pub fn registered_at(&self) -> SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(self.registered_at_ms)
    }
}

/// Information about a workflow registration conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConflict {
    /// The workflow kind that has a conflict.
    pub kind: String,

    /// Reason for the conflict.
    pub reason: String,

    /// ID of the existing worker that has this workflow.
    pub existing_worker_id: String,
}

/// Information about a task registration conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskConflict {
    /// The task kind that has a conflict.
    pub kind: String,

    /// Reason for the conflict.
    pub reason: String,

    /// ID of the existing worker that has this task.
    pub existing_worker_id: String,
}

/// Information about the worker's connection to the server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Whether currently connected.
    pub connected: bool,

    /// Time of last successful heartbeat (Unix timestamp in milliseconds).
    pub last_heartbeat_ms: Option<u64>,

    /// Time of last successful poll (Unix timestamp in milliseconds).
    pub last_poll_ms: Option<u64>,

    /// Number of consecutive heartbeat failures.
    pub heartbeat_failures: u32,

    /// Number of consecutive poll failures.
    pub poll_failures: u32,

    /// Current reconnection attempt (if reconnecting).
    pub reconnect_attempt: Option<u32>,
}

impl ConnectionInfo {
    /// Creates a new ConnectionInfo with connected state.
    pub fn connected() -> Self {
        Self {
            connected: true,
            ..Default::default()
        }
    }

    /// Get last_heartbeat as SystemTime
    pub fn last_heartbeat(&self) -> Option<SystemTime> {
        self.last_heartbeat_ms
            .map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
    }

    /// Get last_poll as SystemTime
    pub fn last_poll(&self) -> Option<SystemTime> {
        self.last_poll_ms
            .map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
    }
}

/// Error returned by worker control operations.
#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerControlError {
    /// Worker is not running.
    #[error("Worker is not running")]
    NotRunning,

    /// Worker is shutting down.
    #[error("Worker is shutting down")]
    ShuttingDown,

    /// Operation not supported in current state.
    #[error("Operation not supported in current state: {0}")]
    InvalidState(String),

    /// Operation timed out.
    #[error("Operation timed out")]
    Timeout,

    /// Server error.
    #[error("Server error: {0}")]
    ServerError(String),

    /// Hook rejected the operation.
    #[error("Operation rejected by hook: {0}")]
    RejectedByHook(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_status_default() {
        let status = WorkerStatus::default();
        assert_eq!(status, WorkerStatus::Initializing);
    }

    #[test]
    fn test_worker_status_equality() {
        let status1 = WorkerStatus::Running {
            server_worker_id: Some(Uuid::nil()),
            started_at: SystemTime::UNIX_EPOCH,
        };
        let status2 = WorkerStatus::Running {
            server_worker_id: Some(Uuid::nil()),
            started_at: SystemTime::UNIX_EPOCH,
        };
        assert_eq!(status1, status2);
    }

    #[test]
    fn test_worker_status_clone() {
        let status = WorkerStatus::Paused {
            reason: "maintenance".to_string(),
        };
        let cloned = status.clone();
        assert_eq!(status, cloned);
    }

    #[test]
    fn test_stop_reason_equality() {
        assert_eq!(StopReason::Graceful, StopReason::Graceful);
        assert_eq!(
            StopReason::Error("test".to_string()),
            StopReason::Error("test".to_string())
        );
        assert_ne!(StopReason::Graceful, StopReason::Immediate);
    }

    #[test]
    fn test_connection_info_default() {
        let info = ConnectionInfo::default();
        assert!(!info.connected);
        assert!(info.last_heartbeat().is_none());
        assert!(info.last_poll().is_none());
        assert_eq!(info.heartbeat_failures, 0);
        assert_eq!(info.poll_failures, 0);
        assert!(info.reconnect_attempt.is_none());
    }

    #[test]
    fn test_connection_info_connected() {
        let info = ConnectionInfo::connected();
        assert!(info.connected);
    }

    #[test]
    fn test_registration_info_has_conflicts() {
        let info = RegistrationInfo {
            worker_id: Uuid::new_v4(),
            success: true,
            registered_at_ms: 0,
            workflow_kinds: vec!["test".to_string()],
            task_kinds: vec![],
            workflow_conflicts: vec![],
            task_conflicts: vec![],
        };
        assert!(!info.has_conflicts());

        let info_with_conflicts = RegistrationInfo {
            workflow_conflicts: vec![WorkflowConflict {
                kind: "test".to_string(),
                reason: "conflict".to_string(),
                existing_worker_id: "worker-1".to_string(),
            }],
            ..info.clone()
        };
        assert!(info_with_conflicts.has_conflicts());
    }

    #[test]
    fn test_worker_control_error_display() {
        let err = WorkerControlError::NotRunning;
        assert_eq!(err.to_string(), "Worker is not running");

        let err = WorkerControlError::InvalidState("Paused".to_string());
        assert_eq!(
            err.to_string(),
            "Operation not supported in current state: Paused"
        );
    }

    #[test]
    fn test_worker_status_serde() {
        let status = WorkerStatus::Running {
            server_worker_id: Some(Uuid::nil()),
            started_at: SystemTime::UNIX_EPOCH,
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: WorkerStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, parsed);
    }
}
