//! gRPC-based TaskSubmitter implementation

use crate::client::task_execution::TaskExecutionClient;
use crate::error::Result;
use crate::workflow::task_submitter::{TaskSubmitOptions, TaskSubmitter};
use async_trait::async_trait;
use serde_json::Value;
use tonic::transport::Channel;
use uuid::Uuid;

/// gRPC-based implementation of TaskSubmitter.
///
/// Submits tasks to the Flovyn server via the TaskExecution gRPC service.
pub struct GrpcTaskSubmitter {
    /// gRPC channel (cloneable)
    channel: Channel,
    /// Worker token for authentication
    worker_token: String,
}

impl GrpcTaskSubmitter {
    /// Create a new GrpcTaskSubmitter with the given channel and worker token.
    pub fn new(channel: Channel, worker_token: &str) -> Self {
        Self {
            channel,
            worker_token: worker_token.to_string(),
        }
    }
}

#[async_trait]
impl TaskSubmitter for GrpcTaskSubmitter {
    async fn submit_task(
        &self,
        workflow_execution_id: Uuid,
        tenant_id: Uuid,
        task_type: &str,
        input: Value,
        options: TaskSubmitOptions,
    ) -> Result<Uuid> {
        // Create a new client for this call (channel is cheap to clone)
        let mut client = TaskExecutionClient::new(self.channel.clone(), &self.worker_token);
        let result = client
            .submit_task(
                &tenant_id.to_string(),
                task_type,
                input,
                Some(workflow_execution_id),
                options.timeout,
                options.max_retries,
                options.queue.as_deref(),
            )
            .await?;

        Ok(result.task_execution_id)
    }
}
