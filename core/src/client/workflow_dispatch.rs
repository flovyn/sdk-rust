//! WorkflowDispatch client wrapper

use crate::client::auth::AuthInterceptor;
use crate::error::CoreResult;
use crate::generated::flovyn_v1;
use crate::generated::flovyn_v1::workflow_dispatch_client::WorkflowDispatchClient;
use serde_json::Value;
use std::time::Duration;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use uuid::Uuid;

/// Type alias for authenticated client
type AuthClient = WorkflowDispatchClient<InterceptedService<Channel, AuthInterceptor>>;

/// Client for workflow dispatch operations
#[derive(Clone)]
pub struct WorkflowDispatch {
    inner: AuthClient,
}

impl WorkflowDispatch {
    /// Create from a channel with authentication
    pub fn new(channel: Channel, token: &str) -> Self {
        let interceptor = AuthInterceptor::worker_token(token);
        Self {
            inner: WorkflowDispatchClient::with_interceptor(channel, interceptor),
        }
    }

    /// Poll for workflows to execute
    pub async fn poll_workflow(
        &mut self,
        worker_id: &str,
        org_id: &str,
        queue: &str,
        timeout: Duration,
    ) -> CoreResult<Option<WorkflowExecutionInfo>> {
        let request = flovyn_v1::PollRequest {
            worker_id: worker_id.to_string(),
            org_id: org_id.to_string(),
            queue: queue.to_string(),
            timeout_seconds: timeout.as_secs() as i64,
            worker_pool_id: None,
            workflow_capabilities: vec![],
        };

        let response = self.inner.poll_workflow(request).await?;

        let poll_response = response.into_inner();
        Ok(poll_response
            .workflow_execution
            .map(|we| WorkflowExecutionInfo {
                id: we.id.parse().unwrap_or_default(),
                kind: we.kind,
                org_id: we.org_id.parse().unwrap_or_default(),
                input: serde_json::from_slice(&we.input).unwrap_or(Value::Null),
                queue: we.queue,
                current_sequence: we.current_sequence,
                workflow_task_time_millis: we.workflow_task_time_millis,
                workflow_version: we.workflow_version,
                workflow_definition_snapshot: we.workflow_definition_snapshot,
            }))
    }

    /// Subscribe to work-available notifications
    pub async fn subscribe_to_notifications(
        &mut self,
        worker_id: &str,
        org_id: &str,
        queue: &str,
    ) -> CoreResult<tonic::codec::Streaming<flovyn_v1::WorkAvailableEvent>> {
        let request = flovyn_v1::SubscriptionRequest {
            worker_id: worker_id.to_string(),
            org_id: org_id.to_string(),
            queue: queue.to_string(),
        };

        let response = self.inner.subscribe_to_notifications(request).await?;

        Ok(response.into_inner())
    }

    /// Start a workflow programmatically
    pub async fn start_workflow(
        &mut self,
        org_id: &str,
        workflow_kind: &str,
        input: Value,
        queue: Option<&str>,
        workflow_version: Option<&str>,
        idempotency_key: Option<&str>,
        metadata: Option<std::collections::HashMap<String, String>>,
    ) -> CoreResult<StartWorkflowResult> {
        let input_bytes = serde_json::to_vec(&input)?;

        let request = flovyn_v1::StartWorkflowRequest {
            org_id: org_id.to_string(),
            workflow_kind: workflow_kind.to_string(),
            input: input_bytes,
            metadata: metadata.unwrap_or_default(),
            queue: queue.unwrap_or("default").to_string(),
            priority_seconds: 0,
            workflow_definition_id: None,
            parent_workflow_execution_id: None,
            workflow_version: workflow_version.map(|s| s.to_string()),
            idempotency_key: idempotency_key.map(|s| s.to_string()),
            idempotency_key_ttl_seconds: None,
        };

        let response = self.inner.start_workflow(request).await?;

        let resp = response.into_inner();
        Ok(StartWorkflowResult {
            workflow_execution_id: resp.workflow_execution_id.parse().unwrap_or_default(),
            idempotency_key_used: resp.idempotency_key_used,
            idempotency_key_new: resp.idempotency_key_new,
        })
    }

    /// Get events for a workflow execution
    pub async fn get_events(
        &mut self,
        workflow_execution_id: Uuid,
        _from_sequence: Option<i32>,
    ) -> CoreResult<Vec<WorkflowEvent>> {
        let request = flovyn_v1::GetEventsRequest {
            workflow_execution_id: workflow_execution_id.to_string(),
        };

        let response = self.inner.get_events(request).await?;

        let events = response
            .into_inner()
            .events
            .into_iter()
            .map(|e| WorkflowEvent {
                sequence: e.sequence_number,
                event_type: e.event_type,
                payload: serde_json::from_slice(&e.event_data).unwrap_or(Value::Null),
            })
            .collect();

        Ok(events)
    }

    /// Submit workflow commands with status
    pub async fn submit_workflow_commands(
        &mut self,
        workflow_execution_id: Uuid,
        commands: Vec<flovyn_v1::WorkflowCommand>,
        status: flovyn_v1::WorkflowStatus,
    ) -> CoreResult<()> {
        let request = flovyn_v1::SubmitWorkflowCommandsRequest {
            workflow_execution_id: workflow_execution_id.to_string(),
            commands,
            status: status as i32,
        };

        self.inner.submit_workflow_commands(request).await?;

        Ok(())
    }

    /// Complete a workflow execution
    pub async fn complete_workflow(
        &mut self,
        workflow_execution_id: Uuid,
        output: Value,
        commands: Vec<flovyn_v1::WorkflowCommand>,
    ) -> CoreResult<()> {
        let output_bytes = serde_json::to_vec(&output)?;

        let mut all_commands = commands;
        let next_seq = all_commands
            .iter()
            .map(|c| c.sequence_number)
            .max()
            .unwrap_or(0)
            + 1;

        all_commands.push(flovyn_v1::WorkflowCommand {
            command_type: flovyn_v1::CommandType::CompleteWorkflow as i32,
            sequence_number: next_seq,
            command_data: Some(flovyn_v1::workflow_command::CommandData::CompleteWorkflow(
                flovyn_v1::CompleteWorkflowCommand {
                    output: output_bytes,
                },
            )),
        });

        self.submit_workflow_commands(
            workflow_execution_id,
            all_commands,
            flovyn_v1::WorkflowStatus::Completed,
        )
        .await
    }

    /// Fail a workflow execution
    pub async fn fail_workflow(
        &mut self,
        workflow_execution_id: Uuid,
        error_message: &str,
        failure_type: &str,
        commands: Vec<flovyn_v1::WorkflowCommand>,
    ) -> CoreResult<()> {
        let mut all_commands = commands;
        let next_seq = all_commands
            .iter()
            .map(|c| c.sequence_number)
            .max()
            .unwrap_or(0)
            + 1;

        all_commands.push(flovyn_v1::WorkflowCommand {
            command_type: flovyn_v1::CommandType::FailWorkflow as i32,
            sequence_number: next_seq,
            command_data: Some(flovyn_v1::workflow_command::CommandData::FailWorkflow(
                flovyn_v1::FailWorkflowCommand {
                    error: error_message.to_string(),
                    stack_trace: String::new(),
                    failure_type: failure_type.to_string(),
                },
            )),
        });

        self.submit_workflow_commands(
            workflow_execution_id,
            all_commands,
            flovyn_v1::WorkflowStatus::Failed,
        )
        .await
    }

    /// Suspend a workflow execution (waiting for external event)
    pub async fn suspend_workflow(
        &mut self,
        workflow_execution_id: Uuid,
        commands: Vec<flovyn_v1::WorkflowCommand>,
    ) -> CoreResult<()> {
        self.submit_workflow_commands(
            workflow_execution_id,
            commands,
            flovyn_v1::WorkflowStatus::Suspended,
        )
        .await
    }

    /// Report execution spans to the server for distributed tracing
    pub async fn report_execution_spans(
        &mut self,
        sdk_info: flovyn_v1::SdkInfo,
        spans: Vec<flovyn_v1::ExecutionSpan>,
    ) -> CoreResult<ReportExecutionSpansResult> {
        let request = flovyn_v1::ReportExecutionSpansRequest {
            sdk_info: Some(sdk_info),
            spans,
        };

        let response = self.inner.report_execution_spans(request).await?;

        let resp = response.into_inner();
        Ok(ReportExecutionSpansResult {
            accepted_count: resp.accepted_count,
            rejected_count: resp.rejected_count,
        })
    }

    /// Resolve a durable promise with a value.
    ///
    /// This allows external systems to resolve promises that were created
    /// by workflows using `ctx.promise()`.
    ///
    /// # Arguments
    /// * `promise_id` - The promise ID (format: workflow_execution_id/promise-name)
    /// * `value` - The value to resolve the promise with (serialized as JSON bytes)
    pub async fn resolve_promise(&mut self, promise_id: &str, value: Vec<u8>) -> CoreResult<()> {
        let request = flovyn_v1::ResolvePromiseRequest {
            promise_id: promise_id.to_string(),
            value,
        };

        self.inner.resolve_promise(request).await?;

        Ok(())
    }

    /// Reject a durable promise with an error.
    ///
    /// This allows external systems to reject promises that were created
    /// by workflows using `ctx.promise()`.
    ///
    /// # Arguments
    /// * `promise_id` - The promise ID (format: workflow_execution_id/promise-name)
    /// * `error` - The error message
    pub async fn reject_promise(&mut self, promise_id: &str, error: &str) -> CoreResult<()> {
        let request = flovyn_v1::RejectPromiseRequest {
            promise_id: promise_id.to_string(),
            error: error.to_string(),
        };

        self.inner.reject_promise(request).await?;

        Ok(())
    }
}

/// Information about a workflow execution received from polling
#[derive(Debug, Clone)]
pub struct WorkflowExecutionInfo {
    /// Unique execution ID
    pub id: Uuid,
    /// Workflow kind/type
    pub kind: String,
    /// Org ID
    pub org_id: Uuid,
    /// Input data
    pub input: Value,
    /// Task queue
    pub queue: String,
    /// Current sequence number
    pub current_sequence: i32,
    /// Workflow task time (for deterministic time)
    pub workflow_task_time_millis: i64,
    /// Locked workflow version
    pub workflow_version: Option<String>,
    /// Workflow definition snapshot (for visual workflows)
    pub workflow_definition_snapshot: Option<String>,
}

/// A workflow event
#[derive(Debug, Clone)]
pub struct WorkflowEvent {
    /// Sequence number
    pub sequence: i32,
    /// Event type
    pub event_type: String,
    /// Event payload
    pub payload: Value,
}

/// Result of starting a workflow
#[derive(Debug, Clone)]
pub struct StartWorkflowResult {
    /// Workflow execution ID
    pub workflow_execution_id: Uuid,
    /// Whether an idempotency key was used
    pub idempotency_key_used: bool,
    /// Whether a new execution was created (false means existing returned)
    pub idempotency_key_new: bool,
}

/// Result of reporting execution spans
#[derive(Debug, Clone)]
pub struct ReportExecutionSpansResult {
    /// Number of spans accepted by the server
    pub accepted_count: i32,
    /// Number of spans rejected by the server
    pub rejected_count: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_execution_info_debug() {
        let info = WorkflowExecutionInfo {
            id: Uuid::new_v4(),
            kind: "test-workflow".to_string(),
            org_id: Uuid::new_v4(),
            input: serde_json::json!({"key": "value"}),
            queue: "default".to_string(),
            current_sequence: 0,
            workflow_task_time_millis: 1234567890,
            workflow_version: Some("1.0.0".to_string()),
            workflow_definition_snapshot: None,
        };

        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("test-workflow"));
    }

    #[test]
    fn test_workflow_event_debug() {
        let event = WorkflowEvent {
            sequence: 1,
            event_type: "OperationCompleted".to_string(),
            payload: serde_json::json!({"result": 42}),
        };

        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("OperationCompleted"));
    }

    #[test]
    fn test_start_workflow_result() {
        let result = StartWorkflowResult {
            workflow_execution_id: Uuid::new_v4(),
            idempotency_key_used: true,
            idempotency_key_new: false,
        };

        assert!(result.idempotency_key_used);
        assert!(!result.idempotency_key_new);
    }
}
