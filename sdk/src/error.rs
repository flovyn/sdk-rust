//! Error types for the Flovyn SDK

use crate::workflow::event::EventType;

/// Main error type for the Flovyn SDK
#[derive(Debug, thiserror::Error)]
pub enum FlovynError {
    /// Workflow is suspended waiting for external event (task completion, promise, timer)
    #[error("Workflow suspended: {reason}")]
    Suspended { reason: String },

    /// Task was cancelled
    #[error("Task cancelled")]
    TaskCancelled,

    /// Workflow was cancelled
    #[error("Workflow cancelled: {0}")]
    WorkflowCancelled(String),

    /// Workflow execution failed
    #[error("Workflow failed: {0}")]
    WorkflowFailed(String),

    /// Determinism violation detected during replay
    #[error("Determinism violation: {0}")]
    DeterminismViolation(DeterminismViolationError),

    /// Child workflow failed
    #[error("Child workflow failed: {name} ({execution_id}): {error}")]
    ChildWorkflowFailed {
        execution_id: String,
        name: String,
        error: String,
    },

    /// Non-retryable error (permanent failure)
    #[error("Non-retryable error: {0}")]
    NonRetryable(String),

    /// gRPC communication error
    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// I/O error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Workflow definition not found
    #[error("Workflow not found: {0}")]
    WorkflowNotFound(String),

    /// Task definition not found
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// Promise timeout
    #[error("Promise timeout: {name}")]
    PromiseTimeout { name: String },

    /// Timer error
    #[error("Timer error: {0}")]
    TimerError(String),

    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Network error
    #[error("Network error: {0}")]
    NetworkError(String),

    /// Timeout error
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

/// Determinism violation types detected during replay validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterminismViolationError {
    /// Command type doesn't match historical event type
    TypeMismatch {
        sequence: i32,
        expected: EventType,
        actual: EventType,
    },

    /// Operation name doesn't match historical operation
    OperationNameMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// Task type doesn't match historical task
    TaskTypeMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// Child workflow mismatch
    ChildWorkflowMismatch {
        sequence: i32,
        field: String,
        expected: String,
        actual: String,
    },

    /// Promise name mismatch
    PromiseNameMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// Timer ID mismatch
    TimerIdMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },

    /// State key mismatch
    StateKeyMismatch {
        sequence: i32,
        expected: String,
        actual: String,
    },
}

impl std::fmt::Display for DeterminismViolationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TypeMismatch {
                sequence,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Type mismatch at sequence {}: expected {:?}, got {:?}",
                    sequence, expected, actual
                )
            }
            Self::OperationNameMismatch {
                sequence,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Operation name mismatch at sequence {}: expected '{}', got '{}'",
                    sequence, expected, actual
                )
            }
            Self::TaskTypeMismatch {
                sequence,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Task type mismatch at sequence {}: expected '{}', got '{}'",
                    sequence, expected, actual
                )
            }
            Self::ChildWorkflowMismatch {
                sequence,
                field,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Child workflow {} mismatch at sequence {}: expected '{}', got '{}'",
                    field, sequence, expected, actual
                )
            }
            Self::PromiseNameMismatch {
                sequence,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Promise name mismatch at sequence {}: expected '{}', got '{}'",
                    sequence, expected, actual
                )
            }
            Self::TimerIdMismatch {
                sequence,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "Timer ID mismatch at sequence {}: expected '{}', got '{}'",
                    sequence, expected, actual
                )
            }
            Self::StateKeyMismatch {
                sequence,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "State key mismatch at sequence {}: expected '{}', got '{}'",
                    sequence, expected, actual
                )
            }
        }
    }
}

impl std::error::Error for DeterminismViolationError {}

/// Result type alias for Flovyn SDK operations
pub type Result<T> = std::result::Result<T, FlovynError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flovyn_error_display() {
        let err = FlovynError::Suspended {
            reason: "Waiting for task".to_string(),
        };
        assert_eq!(err.to_string(), "Workflow suspended: Waiting for task");

        let err = FlovynError::TaskCancelled;
        assert_eq!(err.to_string(), "Task cancelled");

        let err = FlovynError::DeterminismViolation(DeterminismViolationError::TypeMismatch {
            sequence: 5,
            expected: EventType::OperationCompleted,
            actual: EventType::TaskScheduled,
        });
        assert!(err.to_string().contains("Type mismatch at sequence 5"));
    }

    #[test]
    fn test_determinism_violation_error_display() {
        let err = DeterminismViolationError::TypeMismatch {
            sequence: 1,
            expected: EventType::OperationCompleted,
            actual: EventType::TaskScheduled,
        };
        assert!(err.to_string().contains("Type mismatch at sequence 1"));

        let err = DeterminismViolationError::OperationNameMismatch {
            sequence: 2,
            expected: "fetch-user".to_string(),
            actual: "fetch-order".to_string(),
        };
        assert!(err.to_string().contains("Operation name mismatch"));
        assert!(err.to_string().contains("fetch-user"));
        assert!(err.to_string().contains("fetch-order"));
    }

    #[test]
    fn test_result_type() {
        fn returns_ok() -> Result<i32> {
            Ok(42)
        }

        fn returns_err() -> Result<i32> {
            Err(FlovynError::Other("test error".to_string()))
        }

        assert_eq!(returns_ok().unwrap(), 42);
        assert!(returns_err().is_err());
    }

    #[test]
    fn test_error_from_serde_json() {
        let result: std::result::Result<serde_json::Value, serde_json::Error> =
            serde_json::from_str("invalid json");
        let err: FlovynError = result.unwrap_err().into();
        assert!(matches!(err, FlovynError::Serialization(_)));
    }
}
