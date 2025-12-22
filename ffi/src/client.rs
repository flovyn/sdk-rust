//! CoreClient - Client object for FFI.
//!
//! The CoreClient provides operations like starting workflows,
//! sending signals, and querying workflow state.

use std::sync::Arc;

use flovyn_core::client::WorkflowDispatch;
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
            message: format!("Failed to create runtime: {}", e),
        })?;

        let channel: Channel = runtime.block_on(async {
            Channel::from_shared(config.server_url.clone())
                .map_err(|e| FfiError::InvalidConfiguration {
                    message: format!("Invalid server URL: {}", e),
                })?
                .connect()
                .await
                .map_err(|e| FfiError::Grpc {
                    message: format!("Failed to connect: {}", e),
                    code: 14, // UNAVAILABLE
                })
        })?;

        // Generate a client token (in production this would come from auth)
        let client_token = format!("client-token-{}", Uuid::new_v4());

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
                    &self.config.namespace,
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
                message: "Invalid workflow execution ID".to_string(),
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
