//! Error types for NAPI-RS bindings.
//!
//! This module provides error conversions from internal errors to napi::Error.

use flovyn_worker_core::{CoreError, DeterminismViolationError};
use napi::bindgen_prelude::*;

/// Error codes for NAPI errors.
#[derive(Debug, Clone, Copy)]
pub enum NapiErrorCode {
    /// gRPC communication error.
    Grpc,
    /// Operation timed out.
    Timeout,
    /// Serialization or deserialization error.
    Serialization,
    /// Determinism violation detected during replay.
    DeterminismViolation,
    /// Invalid configuration provided.
    InvalidConfiguration,
    /// Operation was cancelled.
    Cancelled,
    /// Worker is shutting down.
    ShuttingDown,
    /// Invalid state for the requested operation.
    InvalidState,
    /// No work available.
    NoWorkAvailable,
    /// Generic error.
    Other,
}

impl NapiErrorCode {
    /// Get the error code as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            NapiErrorCode::Grpc => "GRPC_ERROR",
            NapiErrorCode::Timeout => "TIMEOUT",
            NapiErrorCode::Serialization => "SERIALIZATION_ERROR",
            NapiErrorCode::DeterminismViolation => "DETERMINISM_VIOLATION",
            NapiErrorCode::InvalidConfiguration => "INVALID_CONFIGURATION",
            NapiErrorCode::Cancelled => "CANCELLED",
            NapiErrorCode::ShuttingDown => "SHUTTING_DOWN",
            NapiErrorCode::InvalidState => "INVALID_STATE",
            NapiErrorCode::NoWorkAvailable => "NO_WORK_AVAILABLE",
            NapiErrorCode::Other => "UNKNOWN_ERROR",
        }
    }
}

/// Create a NAPI error with a code and message.
pub fn napi_error(code: NapiErrorCode, msg: impl AsRef<str>) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("[{}] {}", code.as_str(), msg.as_ref()),
    )
}

/// Convert a CoreError to a NAPI error.
pub fn from_core_error(err: CoreError) -> Error {
    match err {
        CoreError::Grpc(status) => napi_error(
            NapiErrorCode::Grpc,
            format!("gRPC error: {} (code: {})", status.message(), status.code()),
        ),
        CoreError::Serialization(err) => {
            napi_error(NapiErrorCode::Serialization, format!("Serialization error: {}", err))
        }
        CoreError::Io(err) => napi_error(NapiErrorCode::Other, format!("IO error: {}", err)),
        CoreError::InvalidConfiguration(msg) => {
            napi_error(NapiErrorCode::InvalidConfiguration, msg)
        }
        CoreError::Timeout(msg) => napi_error(NapiErrorCode::Timeout, msg),
        CoreError::Other(msg) => napi_error(NapiErrorCode::Other, msg),
    }
}

/// Convert a DeterminismViolationError to a NAPI error.
pub fn from_determinism_error(err: DeterminismViolationError) -> Error {
    napi_error(NapiErrorCode::DeterminismViolation, err.to_string())
}

/// Convert a serde_json::Error to a NAPI error.
pub fn from_json_error(err: serde_json::Error) -> Error {
    napi_error(NapiErrorCode::Serialization, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_string() {
        assert_eq!(NapiErrorCode::Grpc.as_str(), "GRPC_ERROR");
        assert_eq!(NapiErrorCode::DeterminismViolation.as_str(), "DETERMINISM_VIOLATION");
    }

    #[test]
    fn test_napi_error_creation() {
        let err = napi_error(NapiErrorCode::Timeout, "Operation timed out");
        assert!(err.reason.contains("TIMEOUT"));
        assert!(err.reason.contains("Operation timed out"));
    }
}
