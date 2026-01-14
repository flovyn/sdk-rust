//! Core error types for the Flovyn workflow orchestration platform
//!
//! This module contains error types that are shared across all language SDKs.
//! Language-specific error types should wrap or extend these core errors.

use crate::workflow::EventType;

/// Core error type for gRPC client operations
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// gRPC communication error
    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// I/O error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// Timeout error
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

/// Result type alias for core operations
pub type CoreResult<T> = std::result::Result<T, CoreError>;

/// Determinism violation types detected during replay validation
///
/// These errors indicate that a workflow's behavior during replay differs
/// from its behavior during the original execution, which violates the
/// determinism requirement for workflow execution.
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

#[cfg(test)]
mod tests {
    use super::*;

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

        let err = DeterminismViolationError::TaskTypeMismatch {
            sequence: 3,
            expected: "payment-task".to_string(),
            actual: "email-task".to_string(),
        };
        assert!(err.to_string().contains("Task type mismatch"));

        let err = DeterminismViolationError::ChildWorkflowMismatch {
            sequence: 4,
            field: "name".to_string(),
            expected: "child-a".to_string(),
            actual: "child-b".to_string(),
        };
        assert!(err.to_string().contains("Child workflow name mismatch"));

        let err = DeterminismViolationError::PromiseNameMismatch {
            sequence: 5,
            expected: "approval".to_string(),
            actual: "confirmation".to_string(),
        };
        assert!(err.to_string().contains("Promise name mismatch"));

        let err = DeterminismViolationError::TimerIdMismatch {
            sequence: 6,
            expected: "timer-1".to_string(),
            actual: "timer-2".to_string(),
        };
        assert!(err.to_string().contains("Timer ID mismatch"));

        let err = DeterminismViolationError::StateKeyMismatch {
            sequence: 7,
            expected: "count".to_string(),
            actual: "total".to_string(),
        };
        assert!(err.to_string().contains("State key mismatch"));
    }
}
