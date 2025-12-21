//! TaskExecution client wrapper

use crate::client::auth::WorkerTokenInterceptor;
use crate::error::{FlovynError, Result};
use crate::generated::flovyn_v1;
use crate::generated::flovyn_v1::task_execution_client::TaskExecutionClient as GrpcTaskExecutionClient;
use crate::task::streaming::{StreamError, StreamEvent, StreamEventType as SdkStreamEventType};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use uuid::Uuid;

/// Type alias for authenticated client
type AuthClient = GrpcTaskExecutionClient<InterceptedService<Channel, WorkerTokenInterceptor>>;

/// Client for task execution operations
pub struct TaskExecutionClient {
    inner: AuthClient,
    /// Sequence counter for stream events
    stream_sequence: AtomicU32,
}

impl TaskExecutionClient {
    /// Create from a channel with worker token authentication
    pub fn new(channel: Channel, token: &str) -> Self {
        let interceptor = WorkerTokenInterceptor::new(token);
        Self {
            inner: GrpcTaskExecutionClient::with_interceptor(channel, interceptor),
            stream_sequence: AtomicU32::new(0),
        }
    }

    /// Reset the stream sequence counter
    ///
    /// Call this at the start of each task execution to reset sequence numbering.
    pub fn reset_stream_sequence(&self) {
        self.stream_sequence.store(0, Ordering::SeqCst);
    }

    /// Submit a task for execution
    #[allow(clippy::too_many_arguments)]
    pub async fn submit_task(
        &mut self,
        tenant_id: &str,
        task_type: &str,
        input: Value,
        workflow_execution_id: Option<Uuid>,
        timeout: Duration,
        max_retries: u32,
        queue: Option<&str>,
    ) -> Result<SubmitTaskResult> {
        let input_bytes = serde_json::to_vec(&input)?;

        let request = flovyn_v1::SubmitTaskRequest {
            workflow_execution_id: workflow_execution_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            tenant_id: tenant_id.to_string(),
            task_type: task_type.to_string(),
            input: input_bytes,
            labels: std::collections::HashMap::new(),
            max_retries: max_retries as i32,
            timeout_ms: timeout.as_millis() as i64,
            idempotency_key: None,
            idempotency_key_ttl_seconds: None,
            queue: queue.unwrap_or("default").to_string(),
            worker_pool_id: None,
        };

        let response = self
            .inner
            .submit_task(request)
            .await
            .map_err(FlovynError::Grpc)?;

        let resp = response.into_inner();
        Ok(SubmitTaskResult {
            task_execution_id: resp.task_execution_id.parse().unwrap_or_default(),
            idempotency_key_used: resp.idempotency_key_used,
            idempotency_key_new: resp.idempotency_key_new,
        })
    }

    /// Poll for tasks to execute
    pub async fn poll_task(
        &mut self,
        worker_id: &str,
        tenant_id: &str,
        queue: &str,
        timeout: Duration,
    ) -> Result<Option<TaskExecutionInfo>> {
        let request = flovyn_v1::PollTaskRequest {
            worker_id: worker_id.to_string(),
            tenant_id: tenant_id.to_string(),
            worker_labels: std::collections::HashMap::new(),
            timeout_seconds: timeout.as_secs() as i64,
            queue: queue.to_string(),
            worker_pool_id: None,
        };

        let response = self
            .inner
            .poll_task(request)
            .await
            .map_err(FlovynError::Grpc)?;

        let poll_response = response.into_inner();
        Ok(poll_response.task.map(|te| TaskExecutionInfo {
            id: te.task_execution_id.parse().unwrap_or_default(),
            workflow_execution_id: if te.workflow_execution_id.is_empty() {
                None
            } else {
                te.workflow_execution_id.parse().ok()
            },
            task_type: te.task_type,
            input: serde_json::from_slice(&te.input).unwrap_or(Value::Null),
            execution_count: te.execution_count as u32,
            queue: te.queue,
        }))
    }

    /// Complete a task execution
    pub async fn complete_task(&mut self, task_execution_id: Uuid, output: Value) -> Result<()> {
        let output_bytes = serde_json::to_vec(&output)?;

        let request = flovyn_v1::CompleteTaskRequest {
            task_execution_id: task_execution_id.to_string(),
            output: output_bytes,
        };

        self.inner
            .complete_task(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Fail a task execution
    pub async fn fail_task(&mut self, task_execution_id: Uuid, error_message: &str) -> Result<()> {
        let request = flovyn_v1::FailTaskRequest {
            task_execution_id: task_execution_id.to_string(),
            error: error_message.to_string(),
        };

        self.inner
            .fail_task(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Report progress for a task
    pub async fn report_progress(
        &mut self,
        task_execution_id: Uuid,
        progress: f64,
        details: Option<&str>,
    ) -> Result<()> {
        let request = flovyn_v1::ReportProgressRequest {
            task_execution_id: task_execution_id.to_string(),
            progress,
            details: details.map(|s| s.to_string()).unwrap_or_default(),
        };

        self.inner
            .report_progress(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Send a heartbeat for a task
    /// Returns Ok(()) if successful, or error if task was cancelled
    pub async fn heartbeat(&mut self, task_execution_id: Uuid) -> Result<()> {
        let request = flovyn_v1::HeartbeatRequest {
            task_execution_id: task_execution_id.to_string(),
        };

        self.inner
            .heartbeat(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Cancel a task
    pub async fn cancel_task(&mut self, task_execution_id: Uuid) -> Result<()> {
        let request = flovyn_v1::CancelTaskRequest {
            task_execution_id: task_execution_id.to_string(),
        };

        self.inner
            .cancel_task(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Log a message for a task
    pub async fn log_message(
        &mut self,
        task_execution_id: Uuid,
        level: &str,
        message: &str,
    ) -> Result<()> {
        let request = flovyn_v1::LogMessageRequest {
            task_execution_id: task_execution_id.to_string(),
            level: level.to_string(),
            message: message.to_string(),
        };

        self.inner
            .log_message(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Get a state value for a task
    pub async fn get_state(&mut self, task_execution_id: Uuid, key: &str) -> Result<Option<Value>> {
        let request = flovyn_v1::GetStateRequest {
            task_execution_id: task_execution_id.to_string(),
            key: key.to_string(),
        };

        let response = self
            .inner
            .get_state(request)
            .await
            .map_err(FlovynError::Grpc)?;

        let resp = response.into_inner();

        if !resp.found {
            return Ok(None);
        }

        let value: Value = if resp.value.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&resp.value)?
        };

        Ok(Some(value))
    }

    /// Set a state value for a task
    pub async fn set_state(
        &mut self,
        task_execution_id: Uuid,
        key: &str,
        value: Value,
    ) -> Result<()> {
        let value_bytes = serde_json::to_vec(&value)?;

        let request = flovyn_v1::SetStateRequest {
            task_execution_id: task_execution_id.to_string(),
            key: key.to_string(),
            value: value_bytes,
        };

        self.inner
            .set_state(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Clear a state value for a task
    pub async fn clear_state(&mut self, task_execution_id: Uuid, key: &str) -> Result<()> {
        let request = flovyn_v1::ClearStateRequest {
            task_execution_id: task_execution_id.to_string(),
            key: key.to_string(),
        };

        self.inner
            .clear_state(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Clear all state for a task
    pub async fn clear_all_state(&mut self, task_execution_id: Uuid) -> Result<()> {
        let request = flovyn_v1::ClearAllStateRequest {
            task_execution_id: task_execution_id.to_string(),
        };

        self.inner
            .clear_all_state(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(())
    }

    /// Get all state keys for a task
    pub async fn get_state_keys(&mut self, task_execution_id: Uuid) -> Result<Vec<String>> {
        let request = flovyn_v1::GetStateKeysRequest {
            task_execution_id: task_execution_id.to_string(),
        };

        let response = self
            .inner
            .get_state_keys(request)
            .await
            .map_err(FlovynError::Grpc)?;

        Ok(response.into_inner().keys)
    }

    /// Stream a task event to connected clients.
    ///
    /// This sends an ephemeral event that is delivered to clients via SSE.
    /// Events are not persisted and delivery is best-effort.
    ///
    /// # Arguments
    ///
    /// * `task_execution_id` - The task execution ID
    /// * `workflow_execution_id` - The workflow execution ID (empty for standalone tasks)
    /// * `event` - The stream event to send
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if acknowledged, `Ok(false)` if not acknowledged,
    /// or an error if the request failed.
    pub async fn stream_task_data(
        &mut self,
        task_execution_id: Uuid,
        workflow_execution_id: Option<Uuid>,
        event: &StreamEvent,
    ) -> std::result::Result<bool, StreamError> {
        // Map SDK event type to protobuf event type
        let event_type = match event.event_type() {
            SdkStreamEventType::Token => flovyn_v1::StreamEventType::Token,
            SdkStreamEventType::Progress => flovyn_v1::StreamEventType::Progress,
            SdkStreamEventType::Data => flovyn_v1::StreamEventType::Data,
            SdkStreamEventType::Error => flovyn_v1::StreamEventType::Error,
        };

        // Serialize event to JSON payload
        let payload = serde_json::to_string(event)?;

        // Get current timestamp
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        // Get and increment sequence number
        let sequence = self.stream_sequence.fetch_add(1, Ordering::SeqCst) as i32;

        let request = flovyn_v1::StreamTaskDataRequest {
            task_execution_id: task_execution_id.to_string(),
            workflow_execution_id: workflow_execution_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            sequence,
            r#type: event_type as i32,
            payload,
            timestamp_ms,
        };

        let response = self
            .inner
            .stream_task_data(request)
            .await
            .map_err(StreamError::GrpcError)?;

        Ok(response.into_inner().acknowledged)
    }
}

/// Result of submitting a task
#[derive(Debug, Clone)]
pub struct SubmitTaskResult {
    /// Task execution ID
    pub task_execution_id: Uuid,
    /// Whether an idempotency key was used
    pub idempotency_key_used: bool,
    /// Whether a new task was created
    pub idempotency_key_new: bool,
}

/// Information about a task execution received from polling
#[derive(Debug, Clone)]
pub struct TaskExecutionInfo {
    /// Unique task execution ID
    pub id: Uuid,
    /// Parent workflow execution ID (if any)
    pub workflow_execution_id: Option<Uuid>,
    /// Task type
    pub task_type: String,
    /// Input data
    pub input: Value,
    /// Current execution count (1-indexed)
    pub execution_count: u32,
    /// Queue this task belongs to
    pub queue: String,
}

#[cfg(test)]
#[allow(clippy::useless_vec)]
mod tests {
    use super::*;

    #[test]
    fn test_task_execution_info_debug() {
        let info = TaskExecutionInfo {
            id: Uuid::new_v4(),
            workflow_execution_id: Some(Uuid::new_v4()),
            task_type: "process-image".to_string(),
            input: serde_json::json!({"url": "https://example.com/image.jpg"}),
            execution_count: 1,
            queue: "default".to_string(),
        };

        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("process-image"));
    }

    #[test]
    fn test_task_execution_info_standalone() {
        let info = TaskExecutionInfo {
            id: Uuid::new_v4(),
            workflow_execution_id: None,
            task_type: "standalone-task".to_string(),
            input: serde_json::json!({}),
            execution_count: 2,
            queue: "gpu".to_string(),
        };

        assert!(info.workflow_execution_id.is_none());
        assert_eq!(info.execution_count, 2);
        assert_eq!(info.queue, "gpu");
    }

    #[test]
    fn test_submit_task_result() {
        let result = SubmitTaskResult {
            task_execution_id: Uuid::new_v4(),
            idempotency_key_used: false,
            idempotency_key_new: true,
        };

        assert!(!result.idempotency_key_used);
        assert!(result.idempotency_key_new);
    }

    #[test]
    fn test_state_value_serialization() {
        let value = serde_json::json!({
            "counter": 42,
            "status": "processing"
        });

        let bytes = serde_json::to_vec(&value).unwrap();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["counter"], 42);
        assert_eq!(parsed["status"], "processing");
    }

    #[test]
    fn test_empty_state_handling() {
        let empty_bytes: &[u8] = &[];

        // Empty bytes should return None
        let result = if empty_bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(empty_bytes).unwrap_or(Value::Null)
        };

        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_state_keys_result() {
        let keys = vec![
            "counter".to_string(),
            "status".to_string(),
            "lastUpdated".to_string(),
        ];

        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&"counter".to_string()));
    }
}
