//! Internal workflow outcome types for control flow separation.
//!
//! This module provides the `WorkflowOutcome<T>` type that separates internal
//! control flow (suspension, determinism violations) from application errors.
//! This prevents users from accidentally catching system errors.
//!
//! # Design
//!
//! - `WorkflowOutcome::Ready(Result<T, FlovynError>)` - Operation completed (success or app error)
//! - `WorkflowOutcome::Suspended` - Workflow needs to suspend (control flow)
//! - `WorkflowOutcome::DeterminismViolation` - Replay corruption detected (system error)
//!
//! User-facing APIs return `Result<T, FlovynError>` which only contains
//! application/infrastructure errors. System errors are handled internally.

// Allow dead code - these methods will be used as we migrate more code to WorkflowOutcome
#![allow(dead_code)]

use crate::error::{DeterminismViolationError, FlovynError};

/// Internal outcome type for workflow operations.
///
/// This type separates control flow and system errors from application errors,
/// preventing users from catching errors they shouldn't handle.
///
/// # Variants
///
/// - `Ready` - Operation completed with either a success value or an application error
/// - `Suspended` - Workflow needs to suspend waiting for an external event
/// - `DeterminismViolation` - Replay detected non-deterministic behavior (fatal)
#[derive(Debug)]
pub(crate) enum WorkflowOutcome<T> {
    /// Operation completed with a value or application error.
    ///
    /// The inner `Result` only contains errors that users should handle:
    /// - `TaskFailed`, `TaskCancelled`
    /// - `ChildWorkflowFailed`, `WorkflowCancelled`
    /// - `PromiseTimeout`, `PromiseRejected`
    /// - Infrastructure errors (`Grpc`, `Io`, etc.)
    Ready(Result<T, FlovynError>),

    /// Workflow needs to suspend waiting for an external event.
    ///
    /// This is a control flow mechanism, not an error. The workflow will
    /// resume when the awaited event (task completion, timer, promise, etc.)
    /// becomes available.
    Suspended {
        /// Human-readable reason for suspension (for debugging).
        reason: String,
    },

    /// Determinism violation detected during replay.
    ///
    /// This indicates the workflow's behavior during replay differs from
    /// its original execution. This is a fatal error - the workflow will
    /// be failed automatically.
    DeterminismViolation(DeterminismViolationError),
}

impl<T> WorkflowOutcome<T> {
    /// Create a successful outcome.
    #[inline]
    pub fn ok(value: T) -> Self {
        WorkflowOutcome::Ready(Ok(value))
    }

    /// Create an error outcome (application/infrastructure error).
    #[inline]
    pub fn err(error: FlovynError) -> Self {
        WorkflowOutcome::Ready(Err(error))
    }

    /// Create a suspended outcome.
    #[inline]
    pub fn suspended(reason: impl Into<String>) -> Self {
        WorkflowOutcome::Suspended {
            reason: reason.into(),
        }
    }

    /// Create a determinism violation outcome.
    #[inline]
    pub fn determinism_violation(error: DeterminismViolationError) -> Self {
        WorkflowOutcome::DeterminismViolation(error)
    }

    /// Check if this outcome is a suspension.
    #[inline]
    pub fn is_suspended(&self) -> bool {
        matches!(self, WorkflowOutcome::Suspended { .. })
    }

    /// Check if this outcome is a determinism violation.
    #[inline]
    pub fn is_determinism_violation(&self) -> bool {
        matches!(self, WorkflowOutcome::DeterminismViolation(_))
    }

    /// Check if this outcome is ready (completed with value or error).
    #[inline]
    pub fn is_ready(&self) -> bool {
        matches!(self, WorkflowOutcome::Ready(_))
    }

    /// Check if this outcome is a successful completion.
    #[inline]
    pub fn is_ok(&self) -> bool {
        matches!(self, WorkflowOutcome::Ready(Ok(_)))
    }

    /// Check if this outcome is an application error.
    #[inline]
    pub fn is_err(&self) -> bool {
        matches!(self, WorkflowOutcome::Ready(Err(_)))
    }

    /// Convert to Result, mapping system errors to FlovynError.
    ///
    /// This is used at API boundaries where we need to return Result<T, FlovynError>
    /// for backward compatibility during migration.
    ///
    /// # Note
    ///
    /// After migration is complete, this method should only be used internally.
    /// User-facing code should never see Suspended or DeterminismViolation.
    pub fn into_result_legacy(self) -> Result<T, FlovynError> {
        match self {
            WorkflowOutcome::Ready(result) => result,
            WorkflowOutcome::Suspended { reason } => Err(FlovynError::Suspended { reason }),
            WorkflowOutcome::DeterminismViolation(err) => {
                Err(FlovynError::DeterminismViolation(err))
            }
        }
    }

    /// Map the success value.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> WorkflowOutcome<U> {
        match self {
            WorkflowOutcome::Ready(Ok(value)) => WorkflowOutcome::Ready(Ok(f(value))),
            WorkflowOutcome::Ready(Err(e)) => WorkflowOutcome::Ready(Err(e)),
            WorkflowOutcome::Suspended { reason } => WorkflowOutcome::Suspended { reason },
            WorkflowOutcome::DeterminismViolation(e) => WorkflowOutcome::DeterminismViolation(e),
        }
    }

    /// Map the error value.
    pub fn map_err<F: FnOnce(FlovynError) -> FlovynError>(self, f: F) -> WorkflowOutcome<T> {
        match self {
            WorkflowOutcome::Ready(Ok(value)) => WorkflowOutcome::Ready(Ok(value)),
            WorkflowOutcome::Ready(Err(e)) => WorkflowOutcome::Ready(Err(f(e))),
            WorkflowOutcome::Suspended { reason } => WorkflowOutcome::Suspended { reason },
            WorkflowOutcome::DeterminismViolation(e) => WorkflowOutcome::DeterminismViolation(e),
        }
    }

    /// Unwrap the success value, panicking on error or system state.
    ///
    /// # Panics
    ///
    /// Panics if the outcome is not `Ready(Ok(_))`.
    #[track_caller]
    pub fn unwrap(self) -> T
    where
        T: std::fmt::Debug,
    {
        match self {
            WorkflowOutcome::Ready(Ok(value)) => value,
            WorkflowOutcome::Ready(Err(e)) => {
                panic!(
                    "called `WorkflowOutcome::unwrap()` on an `Err` value: {:?}",
                    e
                )
            }
            WorkflowOutcome::Suspended { reason } => {
                panic!(
                    "called `WorkflowOutcome::unwrap()` on a `Suspended` value: {}",
                    reason
                )
            }
            WorkflowOutcome::DeterminismViolation(e) => {
                panic!(
                    "called `WorkflowOutcome::unwrap()` on a `DeterminismViolation` value: {:?}",
                    e
                )
            }
        }
    }

    /// Get the suspension reason if this is a Suspended outcome.
    pub fn suspension_reason(&self) -> Option<&str> {
        match self {
            WorkflowOutcome::Suspended { reason } => Some(reason),
            _ => None,
        }
    }

    /// Get the determinism violation error if this is a DeterminismViolation outcome.
    pub fn determinism_error(&self) -> Option<&DeterminismViolationError> {
        match self {
            WorkflowOutcome::DeterminismViolation(e) => Some(e),
            _ => None,
        }
    }
}

impl<T> From<Result<T, FlovynError>> for WorkflowOutcome<T> {
    fn from(result: Result<T, FlovynError>) -> Self {
        WorkflowOutcome::Ready(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::event::EventType;

    #[test]
    fn test_workflow_outcome_ok() {
        let outcome: WorkflowOutcome<i32> = WorkflowOutcome::ok(42);
        assert!(outcome.is_ready());
        assert!(outcome.is_ok());
        assert!(!outcome.is_err());
        assert!(!outcome.is_suspended());
        assert!(!outcome.is_determinism_violation());
        assert_eq!(outcome.unwrap(), 42);
    }

    #[test]
    fn test_workflow_outcome_err() {
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::err(FlovynError::TaskFailed("task failed".to_string()));
        assert!(outcome.is_ready());
        assert!(!outcome.is_ok());
        assert!(outcome.is_err());
        assert!(!outcome.is_suspended());
        assert!(!outcome.is_determinism_violation());
    }

    #[test]
    fn test_workflow_outcome_suspended() {
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::suspended("waiting for task completion");
        assert!(!outcome.is_ready());
        assert!(!outcome.is_ok());
        assert!(!outcome.is_err());
        assert!(outcome.is_suspended());
        assert!(!outcome.is_determinism_violation());
        assert_eq!(
            outcome.suspension_reason(),
            Some("waiting for task completion")
        );
    }

    #[test]
    fn test_workflow_outcome_determinism_violation() {
        let violation = DeterminismViolationError::TypeMismatch {
            sequence: 1,
            expected: EventType::TaskScheduled,
            actual: EventType::OperationCompleted,
        };
        let outcome: WorkflowOutcome<i32> = WorkflowOutcome::determinism_violation(violation);
        assert!(!outcome.is_ready());
        assert!(!outcome.is_ok());
        assert!(!outcome.is_err());
        assert!(!outcome.is_suspended());
        assert!(outcome.is_determinism_violation());
        assert!(outcome.determinism_error().is_some());
    }

    #[test]
    fn test_workflow_outcome_into_result_legacy() {
        // Ok case
        let outcome: WorkflowOutcome<i32> = WorkflowOutcome::ok(42);
        assert_eq!(outcome.into_result_legacy().unwrap(), 42);

        // Err case
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::err(FlovynError::TaskFailed("failed".to_string()));
        assert!(outcome.into_result_legacy().is_err());

        // Suspended case - converts to FlovynError::Suspended
        let outcome: WorkflowOutcome<i32> = WorkflowOutcome::suspended("waiting");
        let result = outcome.into_result_legacy();
        assert!(matches!(result, Err(FlovynError::Suspended { .. })));

        // DeterminismViolation case - converts to FlovynError::DeterminismViolation
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::determinism_violation(DeterminismViolationError::TypeMismatch {
                sequence: 1,
                expected: EventType::TaskScheduled,
                actual: EventType::OperationCompleted,
            });
        let result = outcome.into_result_legacy();
        assert!(matches!(result, Err(FlovynError::DeterminismViolation(_))));
    }

    #[test]
    fn test_workflow_outcome_map() {
        let outcome: WorkflowOutcome<i32> = WorkflowOutcome::ok(21);
        let mapped = outcome.map(|x| x * 2);
        assert_eq!(mapped.unwrap(), 42);

        // Map on error preserves error
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::err(FlovynError::TaskFailed("error".to_string()));
        let mapped = outcome.map(|x| x * 2);
        assert!(mapped.is_err());

        // Map on suspended preserves suspended
        let outcome: WorkflowOutcome<i32> = WorkflowOutcome::suspended("waiting");
        let mapped = outcome.map(|x| x * 2);
        assert!(mapped.is_suspended());

        // Map on determinism violation preserves violation
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::determinism_violation(DeterminismViolationError::TypeMismatch {
                sequence: 1,
                expected: EventType::TaskScheduled,
                actual: EventType::OperationCompleted,
            });
        let mapped = outcome.map(|x| x * 2);
        assert!(mapped.is_determinism_violation());
    }

    #[test]
    fn test_workflow_outcome_from_result() {
        let result: Result<i32, FlovynError> = Ok(42);
        let outcome: WorkflowOutcome<i32> = result.into();
        assert!(outcome.is_ok());

        let result: Result<i32, FlovynError> = Err(FlovynError::TaskFailed("error".to_string()));
        let outcome: WorkflowOutcome<i32> = result.into();
        assert!(outcome.is_err());
    }

    #[test]
    #[should_panic(expected = "called `WorkflowOutcome::unwrap()` on an `Err` value")]
    fn test_workflow_outcome_unwrap_err_panics() {
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::err(FlovynError::TaskFailed("error".to_string()));
        outcome.unwrap();
    }

    #[test]
    #[should_panic(expected = "called `WorkflowOutcome::unwrap()` on a `Suspended` value")]
    fn test_workflow_outcome_unwrap_suspended_panics() {
        let outcome: WorkflowOutcome<i32> = WorkflowOutcome::suspended("waiting");
        outcome.unwrap();
    }

    #[test]
    #[should_panic(
        expected = "called `WorkflowOutcome::unwrap()` on a `DeterminismViolation` value"
    )]
    fn test_workflow_outcome_unwrap_determinism_violation_panics() {
        let outcome: WorkflowOutcome<i32> =
            WorkflowOutcome::determinism_violation(DeterminismViolationError::TypeMismatch {
                sequence: 1,
                expected: EventType::TaskScheduled,
                actual: EventType::OperationCompleted,
            });
        outcome.unwrap();
    }
}
