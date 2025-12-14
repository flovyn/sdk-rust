//! TaskSubmitter trait for submitting tasks from workflow context

use crate::error::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

/// Options for scheduling a task
#[derive(Debug, Clone, Default)]
pub struct TaskSubmitOptions {
    /// Maximum retries for the task
    pub max_retries: u32,
    /// Task timeout
    pub timeout: Duration,
    /// Task queue
    pub queue: Option<String>,
    /// Priority in seconds
    pub priority_seconds: Option<i32>,
}

/// Trait for submitting tasks from within a workflow.
///
/// This trait abstracts the task submission mechanism, allowing the workflow context
/// to submit tasks to the server without directly depending on gRPC clients.
#[async_trait]
pub trait TaskSubmitter: Send + Sync {
    /// Submit a task for execution.
    ///
    /// Returns the task execution ID assigned by the server.
    async fn submit_task(
        &self,
        workflow_execution_id: Uuid,
        tenant_id: Uuid,
        task_type: &str,
        input: Value,
        options: TaskSubmitOptions,
    ) -> Result<Uuid>;
}

/// A no-op task submitter that always fails.
/// Used when task submission is not configured.
pub struct NoOpTaskSubmitter;

#[async_trait]
impl TaskSubmitter for NoOpTaskSubmitter {
    async fn submit_task(
        &self,
        _workflow_execution_id: Uuid,
        _tenant_id: Uuid,
        task_type: &str,
        _input: Value,
        _options: TaskSubmitOptions,
    ) -> Result<Uuid> {
        Err(crate::error::FlovynError::Other(format!(
            "TaskSubmitter is not configured. Cannot schedule task '{}'",
            task_type
        )))
    }
}
