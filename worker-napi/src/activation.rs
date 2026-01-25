//! Activation types for the NAPI protocol.
//!
//! These types define the activation-based protocol where:
//! - Core polls for work and returns activations
//! - Node.js SDK processes activations using context methods
//! - Context handles replay, determinism validation, and command generation
//! - Core extracts commands from context at completion time

use napi_derive::napi;

/// A workflow activation job containing work for the Node.js SDK to process.
#[napi(object)]
#[derive(Clone)]
pub struct WorkflowActivationJob {
    /// Job type: "initialize", "fire_timer", "resolve_task", "fail_task",
    /// "resolve_promise", "reject_promise", "timeout_promise",
    /// "resolve_child_workflow", "fail_child_workflow", "cancel_workflow",
    /// "signal", "query"
    pub job_type: String,

    /// Serialized job data as JSON string.
    pub data: String,
}

impl WorkflowActivationJob {
    /// Create an initialize job.
    pub fn initialize(input: Vec<u8>) -> Self {
        let base64_input = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &input,
        );
        Self {
            job_type: "initialize".to_string(),
            data: serde_json::json!({ "input": base64_input }).to_string(),
        }
    }

    /// Create a fire timer job.
    pub fn fire_timer(timer_id: &str) -> Self {
        Self {
            job_type: "fire_timer".to_string(),
            data: serde_json::json!({ "timerId": timer_id }).to_string(),
        }
    }

    /// Create a cancel workflow job.
    pub fn cancel_workflow() -> Self {
        Self {
            job_type: "cancel_workflow".to_string(),
            data: "{}".to_string(),
        }
    }

    /// Create a signal job.
    pub fn signal(signal_name: &str, payload: Vec<u8>) -> Self {
        let base64_payload = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &payload,
        );
        Self {
            job_type: "signal".to_string(),
            data: serde_json::json!({
                "signalName": signal_name,
                "payload": base64_payload
            })
            .to_string(),
        }
    }
}

/// A key-value pair for workflow state.
#[napi(object)]
#[derive(Clone)]
pub struct StateEntry {
    /// State key.
    pub key: String,
    /// Serialized value as base64 string.
    pub value: String,
}

/// Workflow activation data (passed as plain object, not class instance).
#[napi(object)]
#[derive(Clone)]
pub struct WorkflowActivationData {
    /// Workflow execution ID.
    pub workflow_execution_id: String,
    /// Org ID.
    pub org_id: String,
    /// The workflow kind/type.
    pub workflow_kind: String,
    /// Serialized workflow input as JSON string.
    pub input: String,
    /// Jobs to process in this activation.
    pub jobs: Vec<WorkflowActivationJob>,
    /// Replay events as JSON strings.
    pub replay_events: Vec<String>,
    /// State entries.
    pub state_entries: Vec<StateEntry>,
    /// Current timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// Random seed for deterministic randomness.
    pub random_seed: String,
    /// Whether cancellation has been requested.
    pub cancellation_requested: bool,
}

/// Workflow completion status type.
#[napi(string_enum)]
#[derive(Debug, PartialEq, Eq)]
pub enum WorkflowCompletionStatusType {
    Completed,
    Suspended,
    Cancelled,
    Failed,
}

/// Workflow completion status with data.
#[napi(object)]
#[derive(Clone)]
pub struct WorkflowCompletionStatus {
    /// Status type.
    pub status: WorkflowCompletionStatusType,
    /// Serialized output as JSON string (if completed).
    pub output: Option<String>,
    /// Reason (if cancelled).
    pub reason: Option<String>,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Commands to submit (as JSON array string).
    pub commands: Option<String>,
}

impl WorkflowCompletionStatus {
    /// Create a completed status.
    pub fn completed(output: String, commands: String) -> Self {
        Self {
            status: WorkflowCompletionStatusType::Completed,
            output: Some(output),
            reason: None,
            error: None,
            commands: Some(commands),
        }
    }

    /// Create a suspended status.
    pub fn suspended(commands: String) -> Self {
        Self {
            status: WorkflowCompletionStatusType::Suspended,
            output: None,
            reason: None,
            error: None,
            commands: Some(commands),
        }
    }

    /// Create a cancelled status.
    pub fn cancelled(reason: String, commands: String) -> Self {
        Self {
            status: WorkflowCompletionStatusType::Cancelled,
            output: None,
            reason: Some(reason),
            error: None,
            commands: Some(commands),
        }
    }

    /// Create a failed status.
    pub fn failed(error: String, commands: String) -> Self {
        Self {
            status: WorkflowCompletionStatusType::Failed,
            output: None,
            reason: None,
            error: Some(error),
            commands: Some(commands),
        }
    }
}

/// Task activation data (passed as plain object, not class instance).
#[napi(object)]
#[derive(Clone)]
pub struct TaskActivationData {
    /// The task execution ID.
    pub task_execution_id: String,
    /// The task kind/type.
    pub task_kind: String,
    /// Serialized task input as JSON string.
    pub input: String,
    /// Workflow execution ID that scheduled this task (if any).
    pub workflow_execution_id: Option<String>,
    /// Current attempt number (1-based).
    pub attempt: u32,
    /// Maximum number of retries.
    pub max_retries: u32,
    /// Timeout in milliseconds (if set).
    pub timeout_ms: Option<i64>,
}

/// Task completion status type.
#[napi(string_enum)]
#[derive(Debug, PartialEq, Eq)]
pub enum TaskCompletionStatusType {
    Completed,
    Failed,
    Cancelled,
}

/// Completion sent back after processing a task activation.
#[napi(object)]
#[derive(Clone)]
pub struct TaskCompletion {
    /// The task execution ID.
    pub task_execution_id: String,
    /// Status type.
    pub status: TaskCompletionStatusType,
    /// Serialized output as JSON string (if completed).
    pub output: Option<String>,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Whether this is retryable (if failed).
    pub retryable: Option<bool>,
}

impl TaskCompletion {
    /// Create a completed task completion.
    pub fn completed(task_execution_id: String, output: String) -> Self {
        Self {
            task_execution_id,
            status: TaskCompletionStatusType::Completed,
            output: Some(output),
            error: None,
            retryable: None,
        }
    }

    /// Create a failed task completion.
    pub fn failed(task_execution_id: String, error: String, retryable: bool) -> Self {
        Self {
            task_execution_id,
            status: TaskCompletionStatusType::Failed,
            output: None,
            error: Some(error),
            retryable: Some(retryable),
        }
    }

    /// Create a cancelled task completion.
    pub fn cancelled(task_execution_id: String) -> Self {
        Self {
            task_execution_id,
            status: TaskCompletionStatusType::Cancelled,
            output: None,
            error: None,
            retryable: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_completion_status_completed() {
        let status = WorkflowCompletionStatus::completed("{}".to_string(), "[]".to_string());
        assert_eq!(status.status, WorkflowCompletionStatusType::Completed);
        assert!(status.output.is_some());
    }

    #[test]
    fn test_workflow_completion_status_suspended() {
        let status = WorkflowCompletionStatus::suspended("[]".to_string());
        assert_eq!(status.status, WorkflowCompletionStatusType::Suspended);
    }

    #[test]
    fn test_workflow_completion_status_failed() {
        let status = WorkflowCompletionStatus::failed("test error".to_string(), "[]".to_string());
        assert_eq!(status.status, WorkflowCompletionStatusType::Failed);
        assert_eq!(status.error, Some("test error".to_string()));
    }

    #[test]
    fn test_task_completion_completed() {
        let completion = TaskCompletion::completed("task-123".to_string(), "{}".to_string());
        assert_eq!(completion.task_execution_id, "task-123");
        assert_eq!(completion.status, TaskCompletionStatusType::Completed);
    }

    #[test]
    fn test_task_completion_failed() {
        let completion =
            TaskCompletion::failed("task-456".to_string(), "test error".to_string(), true);
        assert_eq!(completion.task_execution_id, "task-456");
        assert_eq!(completion.status, TaskCompletionStatusType::Failed);
        assert_eq!(completion.retryable, Some(true));
    }
}
