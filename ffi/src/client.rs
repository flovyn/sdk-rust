//! CoreClient - Client object for FFI.
//!
//! The CoreClient provides operations like starting workflows,
//! sending signals, and querying workflow state.

use std::sync::Arc;

use flovyn_sdk_core::client::{WorkflowDispatch, WorkflowQueryClient};
use tokio::runtime::Runtime;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::config::ClientConfig;
use crate::error::FfiError;

/// Result of starting a workflow.
#[derive(Debug, Clone, uniffi::Record)]
pub struct StartWorkflowResponse {
    /// The workflow execution ID.
    pub workflow_execution_id: String,
    /// Whether an idempotency key was used.
    pub idempotency_key_used: bool,
    /// Whether this is a new execution (false means existing was returned).
    pub idempotency_key_new: bool,
}

/// The client object for FFI, used for workflow operations.
///
/// CoreClient provides:
/// - Starting workflows
/// - Querying workflow state
/// - Sending signals to workflows
/// - Cancelling workflows
#[derive(uniffi::Object)]
pub struct CoreClient {
    /// gRPC channel
    channel: Channel,
    /// Client token (for authentication)
    client_token: String,
    /// Client configuration
    config: ClientConfig,
    /// Tokio runtime for async operations
    runtime: Runtime,
}

#[uniffi::export]
impl CoreClient {
    /// Create a new CoreClient with the given configuration.
    #[uniffi::constructor]
    pub fn new(config: ClientConfig) -> Result<Arc<Self>, FfiError> {
        let runtime = Runtime::new().map_err(|e| FfiError::Other {
            msg: format!("Failed to create runtime: {}", e),
        })?;

        let channel: Channel = runtime.block_on(async {
            Channel::from_shared(config.server_url.clone())
                .map_err(|e| FfiError::InvalidConfiguration {
                    msg: format!("Invalid server URL: {}", e),
                })?
                .connect()
                .await
                .map_err(|e| FfiError::Grpc {
                    msg: format!("Failed to connect: {}", e),
                    code: 14, // UNAVAILABLE
                })
        })?;

        // Use the client token if provided, otherwise generate a placeholder
        let client_token = config
            .client_token
            .clone()
            .unwrap_or_else(|| format!("fct_placeholder-{}", Uuid::new_v4()));

        Ok(Arc::new(Self {
            channel,
            client_token,
            config,
            runtime,
        }))
    }

    /// Start a new workflow execution.
    ///
    /// # Arguments
    /// * `workflow_kind` - The type/kind of workflow to start
    /// * `input` - Serialized input as JSON bytes
    /// * `task_queue` - Optional task queue (defaults to "default")
    /// * `workflow_version` - Optional workflow version
    /// * `idempotency_key` - Optional idempotency key for deduplication
    ///
    /// # Returns
    /// The workflow execution ID and idempotency information.
    pub fn start_workflow(
        &self,
        workflow_kind: String,
        input: Vec<u8>,
        task_queue: Option<String>,
        workflow_version: Option<String>,
        idempotency_key: Option<String>,
    ) -> Result<StartWorkflowResponse, FfiError> {
        let input_value: serde_json::Value =
            serde_json::from_slice(&input).unwrap_or(serde_json::Value::Null);

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        let result = self.runtime.block_on(async {
            dispatch_client
                .start_workflow(
                    &self.config.tenant_id,
                    &workflow_kind,
                    input_value,
                    task_queue.as_deref(),
                    workflow_version.as_deref(),
                    idempotency_key.as_deref(),
                )
                .await
        })?;

        Ok(StartWorkflowResponse {
            workflow_execution_id: result.workflow_execution_id.to_string(),
            idempotency_key_used: result.idempotency_key_used,
            idempotency_key_new: result.idempotency_key_new,
        })
    }

    /// Get the events for a workflow execution.
    ///
    /// # Arguments
    /// * `workflow_execution_id` - The workflow execution ID
    ///
    /// # Returns
    /// A list of workflow events as JSON bytes.
    pub fn get_workflow_events(
        &self,
        workflow_execution_id: String,
    ) -> Result<Vec<WorkflowEventRecord>, FfiError> {
        let execution_id = Uuid::parse_str(&workflow_execution_id).map_err(|_| {
            FfiError::InvalidConfiguration {
                msg: "Invalid workflow execution ID".to_string(),
            }
        })?;

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        let events = self
            .runtime
            .block_on(async { dispatch_client.get_events(execution_id, None).await })?;

        Ok(events
            .into_iter()
            .map(|e| WorkflowEventRecord {
                sequence: e.sequence,
                event_type: e.event_type,
                payload: serde_json::to_vec(&e.payload).unwrap_or_default(),
            })
            .collect())
    }

    /// Query workflow state.
    ///
    /// # Arguments
    /// * `workflow_execution_id` - The workflow execution ID
    /// * `query_name` - The name of the query to execute
    /// * `params` - Query parameters as JSON bytes
    ///
    /// # Returns
    /// The query result as JSON bytes.
    pub fn query_workflow(
        &self,
        workflow_execution_id: String,
        query_name: String,
        params: Vec<u8>,
    ) -> Result<Vec<u8>, FfiError> {
        let execution_id = Uuid::parse_str(&workflow_execution_id).map_err(|_| {
            FfiError::InvalidConfiguration {
                msg: "Invalid workflow execution ID".to_string(),
            }
        })?;

        let params_value: serde_json::Value =
            serde_json::from_slice(&params).unwrap_or(serde_json::Value::Null);

        let mut query_client = WorkflowQueryClient::new(self.channel.clone(), &self.client_token);

        let result = self.runtime.block_on(async {
            query_client
                .query(execution_id, &query_name, params_value)
                .await
        })?;

        serde_json::to_vec(&result).map_err(|e| FfiError::Other {
            msg: format!("Failed to serialize query result: {}", e),
        })
    }

    /// Resolve a durable promise with a value.
    ///
    /// This allows external systems to resolve promises that were created
    /// by workflows using `ctx.promise()`.
    ///
    /// # Arguments
    /// * `promise_id` - The promise ID (format: workflow_execution_id/promise-name)
    /// * `value` - The value to resolve the promise with (JSON bytes)
    pub fn resolve_promise(&self, promise_id: String, value: Vec<u8>) -> Result<(), FfiError> {
        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        self.runtime
            .block_on(async { dispatch_client.resolve_promise(&promise_id, value).await })?;

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
    pub fn reject_promise(&self, promise_id: String, error: String) -> Result<(), FfiError> {
        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        self.runtime
            .block_on(async { dispatch_client.reject_promise(&promise_id, &error).await })?;

        Ok(())
    }
}

/// A workflow event record for FFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WorkflowEventRecord {
    /// Event sequence number.
    pub sequence: i32,
    /// Event type as string.
    pub event_type: String,
    /// Event payload as JSON bytes.
    pub payload: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_workflow_response() {
        let response = StartWorkflowResponse {
            workflow_execution_id: "test-id".to_string(),
            idempotency_key_used: true,
            idempotency_key_new: false,
        };
        assert_eq!(response.workflow_execution_id, "test-id");
        assert!(response.idempotency_key_used);
        assert!(!response.idempotency_key_new);
    }

    #[test]
    fn test_workflow_event_record() {
        let record = WorkflowEventRecord {
            sequence: 1,
            event_type: "OPERATION_COMPLETED".to_string(),
            payload: b"{}".to_vec(),
        };
        assert_eq!(record.sequence, 1);
        assert_eq!(record.event_type, "OPERATION_COMPLETED");
    }
}
