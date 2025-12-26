//! FFI error types for cross-language error handling.
//!
//! This module provides uniffi-compatible error types that wrap internal
//! errors from `flovyn-core` for consumption by foreign languages.

use flovyn_sdk_core::{CoreError, DeterminismViolationError};

/// FFI-compatible error type for all operations.
///
/// This enum wraps internal errors into a format suitable for FFI,
/// with simple field types that can cross the language boundary.
///
/// Note: We use `msg` instead of `message` to avoid conflicts with
/// Kotlin's Exception.message property in generated bindings.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    /// gRPC communication error.
    #[error("gRPC error: {msg} (code: {code})")]
    Grpc {
        /// Error message from the gRPC layer.
        msg: String,
        /// gRPC status code as integer.
        code: i32,
    },

    /// Operation timed out.
    #[error("Timeout: {msg}")]
    Timeout {
        /// Description of what timed out.
        msg: String,
    },

    /// Serialization or deserialization error.
    #[error("Serialization error: {msg}")]
    Serialization {
        /// Description of the serialization error.
        msg: String,
    },

    /// Determinism violation detected during replay.
    #[error("Determinism violation: {msg}")]
    DeterminismViolation {
        /// Description of what violated determinism.
        msg: String,
    },

    /// Invalid configuration provided.
    #[error("Invalid configuration: {msg}")]
    InvalidConfiguration {
        /// Description of the configuration error.
        msg: String,
    },

    /// Operation was cancelled.
    #[error("Cancelled")]
    Cancelled,

    /// Worker is shutting down.
    #[error("Worker is shutting down")]
    ShuttingDown,

    /// No work available (poll returned empty).
    #[error("No work available")]
    NoWorkAvailable,

    /// Generic error for other cases.
    #[error("{msg}")]
    Other {
        /// Error message.
        msg: String,
    },
}

impl From<CoreError> for FfiError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::Grpc(status) => FfiError::Grpc {
                msg: status.message().to_string(),
                code: status.code() as i32,
            },
            CoreError::Serialization(err) => FfiError::Serialization {
                msg: err.to_string(),
            },
            CoreError::Io(err) => FfiError::Other {
                msg: format!("IO error: {}", err),
            },
            CoreError::InvalidConfiguration(m) => FfiError::InvalidConfiguration { msg: m },
            CoreError::Timeout(m) => FfiError::Timeout { msg: m },
            CoreError::Other(m) => FfiError::Other { msg: m },
        }
    }
}

impl From<DeterminismViolationError> for FfiError {
    fn from(err: DeterminismViolationError) -> Self {
        FfiError::DeterminismViolation {
            msg: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for FfiError {
    fn from(err: serde_json::Error) -> Self {
        FfiError::Serialization {
            msg: err.to_string(),
        }
    }
}

/// Result type alias for FFI operations.
pub type FfiResult<T> = Result<T, FfiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ffi_error_display() {
        let err = FfiError::Grpc {
            msg: "connection refused".to_string(),
            code: 14,
        };
        assert_eq!(err.to_string(), "gRPC error: connection refused (code: 14)");

        let err = FfiError::Timeout {
            msg: "poll timeout".to_string(),
        };
        assert_eq!(err.to_string(), "Timeout: poll timeout");

        let err = FfiError::Cancelled;
        assert_eq!(err.to_string(), "Cancelled");
    }

    #[test]
    fn test_from_core_error() {
        let core_err = CoreError::Timeout("test timeout".to_string());
        let ffi_err: FfiError = core_err.into();
        assert!(matches!(ffi_err, FfiError::Timeout { msg } if msg == "test timeout"));
    }

    #[test]
    fn test_from_determinism_violation() {
        let violation = DeterminismViolationError::TypeMismatch {
            sequence: 1,
            expected: flovyn_sdk_core::EventType::OperationCompleted,
            actual: flovyn_sdk_core::EventType::TaskScheduled,
        };
        let ffi_err: FfiError = violation.into();
        assert!(matches!(ffi_err, FfiError::DeterminismViolation { .. }));
    }
}
