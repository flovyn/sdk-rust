//! Activation types for the FFI protocol.
//!
//! These types define the activation-based protocol where:
//! - Core polls for work and returns activations with FfiWorkflowContext
//! - Language SDK processes activations using context methods
//! - Context handles replay, determinism validation, and command generation
//! - Core extracts commands from context at completion time

use std::sync::Arc;

use crate::context::FfiWorkflowContext;

/// A workflow activation containing work for the language SDK to process.
///
/// The activation includes a replay-aware context that handles:
/// - Determinism validation during replay
/// - Cached result return for replayed operations
/// - Command generation for new operations
///
/// Activations are returned by `CoreWorker::poll_workflow_activation()`.
#[derive(uniffi::Record)]
pub struct WorkflowActivation {
    /// The replay-aware workflow context.
    ///
    /// Use this context to call workflow operations like `schedule_task()`,
    /// `create_promise()`, `start_timer()`, etc. The context handles replay
    /// automatically and returns cached results during replay.
    pub context: Arc<FfiWorkflowContext>,

    /// The workflow kind/type.
    pub workflow_kind: String,

    /// Serialized workflow input as JSON bytes.
    pub input: Vec<u8>,

    /// Jobs to process in this activation.
    ///
    /// Jobs include signals, queries, and cancellation requests that
    /// don't go through the context replay mechanism.
    pub jobs: Vec<WorkflowActivationJob>,
}

/// A key-value pair for workflow state.
#[derive(Debug, Clone, uniffi::Record)]
pub struct StateEntry {
    /// State key.
    pub key: String,
    /// Serialized value as JSON bytes.
    pub value: Vec<u8>,
}

/// Jobs that can be included in a workflow activation.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum WorkflowActivationJob {
    /// Initialize/start the workflow.
    Initialize {
        /// Serialized workflow input as JSON bytes.
        input: Vec<u8>,
    },

    /// A timer has fired.
    FireTimer {
        /// The timer ID that fired.
        timer_id: String,
    },

    /// A scheduled task has completed.
    ResolveTask {
        /// The task execution ID.
        task_execution_id: String,
        /// Serialized task result as JSON bytes.
        result: Vec<u8>,
    },

    /// A scheduled task has failed.
    FailTask {
        /// The task execution ID.
        task_execution_id: String,
        /// Error message.
        error: String,
        /// Whether this is retryable.
        retryable: bool,
    },

    /// A durable promise has been resolved.
    ResolvePromise {
        /// The promise name/ID.
        promise_name: String,
        /// Serialized promise value as JSON bytes.
        value: Vec<u8>,
    },

    /// A durable promise has been rejected.
    RejectPromise {
        /// The promise name/ID.
        promise_name: String,
        /// Error message.
        error: String,
    },

    /// A durable promise has timed out.
    TimeoutPromise {
        /// The promise name/ID.
        promise_name: String,
    },

    /// A child workflow has completed.
    ResolveChildWorkflow {
        /// The child workflow execution ID.
        child_execution_id: String,
        /// Serialized result as JSON bytes.
        result: Vec<u8>,
    },

    /// A child workflow has failed.
    FailChildWorkflow {
        /// The child workflow execution ID.
        child_execution_id: String,
        /// Error message.
        error: String,
    },

    /// A cancellation has been requested.
    CancelWorkflow,

    /// A signal was received (external event).
    Signal {
        /// Signal name.
        signal_name: String,
        /// Serialized signal payload as JSON bytes.
        payload: Vec<u8>,
    },

    /// Query the workflow state (read-only).
    Query {
        /// Query name.
        query_name: String,
        /// Serialized query args as JSON bytes.
        args: Vec<u8>,
    },
}

/// Status to report when completing a workflow activation.
///
/// Used with `CoreWorker::complete_workflow_activation()`.
/// Commands are automatically extracted from the context.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum WorkflowCompletionStatus {
    /// Workflow completed successfully with output.
    Completed {
        /// Serialized output as JSON bytes.
        output: Vec<u8>,
    },

    /// Workflow suspended waiting for external events.
    ///
    /// The workflow will be resumed when a task completes, timer fires,
    /// promise resolves, or child workflow finishes.
    Suspended,

    /// Workflow was cancelled.
    Cancelled {
        /// Reason for cancellation.
        reason: String,
    },

    /// Workflow failed with an error.
    Failed {
        /// Error message.
        error: String,
    },
}

/// A task activation containing work for the language SDK to process.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TaskActivation {
    /// The task execution ID.
    pub task_execution_id: String,

    /// The task kind/type.
    pub task_kind: String,

    /// Serialized task input as JSON bytes.
    pub input: Vec<u8>,

    /// Workflow execution ID that scheduled this task (if any).
    pub workflow_execution_id: Option<String>,

    /// Current attempt number (1-based).
    pub attempt: u32,

    /// Maximum number of retries.
    pub max_retries: u32,

    /// Timeout in milliseconds (if set).
    pub timeout_ms: Option<i64>,
}

/// Completion sent back after processing a task activation.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum TaskCompletion {
    /// Task completed successfully.
    Completed {
        /// The task execution ID.
        task_execution_id: String,
        /// Serialized output as JSON bytes.
        output: Vec<u8>,
    },

    /// Task failed.
    Failed {
        /// The task execution ID.
        task_execution_id: String,
        /// Error message.
        error: String,
        /// Whether this is retryable.
        retryable: bool,
    },

    /// Task was cancelled.
    Cancelled {
        /// The task execution ID.
        task_execution_id: String,
    },
}

impl TaskCompletion {
    /// Get the task execution ID from this completion.
    pub fn task_execution_id(&self) -> &str {
        match self {
            TaskCompletion::Completed {
                task_execution_id, ..
            } => task_execution_id,
            TaskCompletion::Failed {
                task_execution_id, ..
            } => task_execution_id,
            TaskCompletion::Cancelled { task_execution_id } => task_execution_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_workflow_activation_creation() {
        let context = FfiWorkflowContext::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1234567890,
            12345,
            vec![],
            vec![],
            false,
        );

        let activation = WorkflowActivation {
            context,
            workflow_kind: "my-workflow".to_string(),
            input: b"{}".to_vec(),
            jobs: vec![WorkflowActivationJob::Initialize {
                input: b"{}".to_vec(),
            }],
        };
        assert_eq!(activation.workflow_kind, "my-workflow");
    }

    #[test]
    fn test_workflow_completion_status() {
        let status = WorkflowCompletionStatus::Completed {
            output: b"{}".to_vec(),
        };
        assert!(matches!(status, WorkflowCompletionStatus::Completed { .. }));

        let status = WorkflowCompletionStatus::Suspended;
        assert!(matches!(status, WorkflowCompletionStatus::Suspended));

        let status = WorkflowCompletionStatus::Failed {
            error: "test error".to_string(),
        };
        assert!(matches!(status, WorkflowCompletionStatus::Failed { .. }));
    }

    #[test]
    fn test_task_completion_task_id() {
        let completion = TaskCompletion::Completed {
            task_execution_id: "task-123".to_string(),
            output: b"{}".to_vec(),
        };
        assert_eq!(completion.task_execution_id(), "task-123");

        let completion = TaskCompletion::Failed {
            task_execution_id: "task-456".to_string(),
            error: "test error".to_string(),
            retryable: true,
        };
        assert_eq!(completion.task_execution_id(), "task-456");
    }
}
