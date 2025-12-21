//! Error types for the Flovyn SDK

// Re-export core error types
pub use flovyn_core::{CoreError, DeterminismViolationError};

// Re-export EventType for use in error tests
#[cfg(test)]
use flovyn_core::EventType;

/// Main error type for the Flovyn SDK
#[derive(Debug, thiserror::Error)]
pub enum FlovynError {
    /// Workflow is suspended waiting for external event (task completion, promise, timer)
    #[error("Workflow suspended: {reason}")]
    Suspended { reason: String },

    /// Task was cancelled
    #[error("Task cancelled")]
    TaskCancelled,

    /// Task execution failed
    #[error("Task failed: {0}")]
    TaskFailed(String),

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

    /// Promise rejected
    #[error("Promise rejected: {name}: {error}")]
    PromiseRejected { name: String, error: String },

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

// DeterminismViolationError is now defined in flovyn-core and re-exported above

/// Result type alias for Flovyn SDK operations
pub type Result<T> = std::result::Result<T, FlovynError>;

impl From<CoreError> for FlovynError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Grpc(status) => FlovynError::Grpc(status),
            CoreError::Serialization(e) => FlovynError::Serialization(e),
            CoreError::Io(e) => FlovynError::Io(e),
            CoreError::InvalidConfiguration(msg) => FlovynError::InvalidConfiguration(msg),
            CoreError::Timeout(msg) => FlovynError::Timeout(msg),
            CoreError::Other(msg) => FlovynError::Other(msg),
        }
    }
}

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
