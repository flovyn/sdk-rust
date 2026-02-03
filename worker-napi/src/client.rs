//! NapiClient - Client for workflow operations.
//!
//! The NapiClient provides methods for starting workflows, sending signals,
//! querying workflows, and resolving promises.

use flovyn_worker_core::client::{oauth2, WorkflowDispatch, WorkflowQueryClient};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::config::ClientConfig;
use crate::error::{from_core_error, napi_error, NapiErrorCode};

/// Client for workflow operations (not worker polling).
///
/// NapiClient provides methods for:
/// - Starting workflows
/// - Querying workflow state
/// - Resolving/rejecting external promises
#[napi]
pub struct NapiClient {
    /// gRPC channel
    channel: Channel,
    /// Client token
    client_token: String,
    /// Client configuration
    config: ClientConfig,
    /// Tokio runtime for async operations
    #[allow(dead_code)]
    runtime: Arc<Runtime>,
}

/// Result of starting a workflow.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct StartWorkflowResult {
    /// The workflow execution ID.
    pub workflow_execution_id: String,
    /// Whether the idempotency key was used (for idempotent requests).
    pub idempotency_key_used: bool,
    /// Whether this created a new workflow (vs finding existing).
    pub idempotency_key_new: bool,
}

/// Options for starting a workflow.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct StartWorkflowOptions {
    /// Queue to run the workflow on (optional, uses default if not provided).
    pub queue: Option<String>,
    /// Workflow version (optional).
    pub workflow_version: Option<String>,
    /// Idempotency key for deduplication (optional).
    pub idempotency_key: Option<String>,
}

/// Result of signaling a workflow.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct SignalWorkflowResult {
    /// Sequence number of the signal event.
    pub signal_event_sequence: i64,
}

/// Result of signal-with-start operation.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct SignalWithStartResult {
    /// The workflow execution ID.
    pub workflow_execution_id: String,
    /// Whether the workflow was created (vs already existed).
    pub workflow_created: bool,
    /// Sequence number of the signal event.
    pub signal_event_sequence: i64,
}

#[napi]
impl NapiClient {
    /// Create a new NapiClient with the given configuration.
    #[napi(constructor)]
    pub fn new(config: ClientConfig) -> Result<Self> {
        // Create a Tokio runtime
        let runtime = Arc::new(Runtime::new().map_err(|e| {
            napi_error(
                NapiErrorCode::Other,
                format!("Failed to create runtime: {}", e),
            )
        })?);

        // Connect to the server
        let channel: Channel = runtime.block_on(async {
            Channel::from_shared(config.server_url.clone())
                .map_err(|e| {
                    napi_error(
                        NapiErrorCode::InvalidConfiguration,
                        format!("Invalid server URL: {}", e),
                    )
                })?
                .connect()
                .await
                .map_err(|e| napi_error(NapiErrorCode::Grpc, format!("Failed to connect: {}", e)))
        })?;

        // Determine authentication token
        let client_token = if let Some(oauth2_creds) = &config.oauth2_credentials {
            // Fetch token using OAuth2 client credentials flow
            let core_creds = oauth2::OAuth2Credentials::new(
                &oauth2_creds.client_id,
                &oauth2_creds.client_secret,
                &oauth2_creds.token_endpoint,
            );
            let core_creds = if let Some(scopes) = &oauth2_creds.scopes {
                core_creds.with_scopes(scopes.split_whitespace().map(String::from).collect())
            } else {
                core_creds
            };

            runtime.block_on(async {
                oauth2::fetch_access_token(&core_creds)
                    .await
                    .map(|r| r.access_token)
                    .map_err(|e| {
                        napi_error(
                            NapiErrorCode::Other,
                            format!("OAuth2 token fetch failed: {}", e),
                        )
                    })
            })?
        } else if let Some(token) = &config.client_token {
            token.clone()
        } else {
            format!("fct_placeholder-{}", Uuid::new_v4())
        };

        Ok(Self {
            channel,
            client_token,
            config,
            runtime,
        })
    }

    /// Start a new workflow.
    #[napi]
    pub async fn start_workflow(
        &self,
        workflow_kind: String,
        input: String,
        options: Option<StartWorkflowOptions>,
    ) -> Result<StartWorkflowResult> {
        let options = options.unwrap_or(StartWorkflowOptions {
            queue: None,
            workflow_version: None,
            idempotency_key: None,
        });

        let input_value: serde_json::Value =
            serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        let result = dispatch_client
            .start_workflow(
                &self.config.org_id,
                &workflow_kind,
                input_value,
                options.queue.as_deref(),
                options.workflow_version.as_deref(),
                options.idempotency_key.as_deref(),
                None, // metadata
            )
            .await
            .map_err(from_core_error)?;

        Ok(StartWorkflowResult {
            workflow_execution_id: result.workflow_execution_id.to_string(),
            idempotency_key_used: result.idempotency_key_used,
            idempotency_key_new: result.idempotency_key_new,
        })
    }

    /// Query a workflow.
    #[napi]
    pub async fn query_workflow(
        &self,
        workflow_id: String,
        query_name: String,
        args: String,
    ) -> Result<String> {
        let workflow_execution_id = Uuid::parse_str(&workflow_id)
            .map_err(|_| napi_error(NapiErrorCode::InvalidConfiguration, "Invalid workflow ID"))?;

        let args_value: serde_json::Value =
            serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);

        let mut query_client = WorkflowQueryClient::new(self.channel.clone(), &self.client_token);

        let result = query_client
            .query(workflow_execution_id, &query_name, args_value)
            .await
            .map_err(from_core_error)?;

        Ok(serde_json::to_string(&result).unwrap_or_default())
    }

    /// Resolve an external promise.
    #[napi]
    pub async fn resolve_promise(&self, promise_id: String, value: String) -> Result<()> {
        let value_bytes = value.into_bytes();

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        dispatch_client
            .resolve_promise(&promise_id, value_bytes)
            .await
            .map_err(from_core_error)?;

        Ok(())
    }

    /// Reject an external promise.
    #[napi]
    pub async fn reject_promise(&self, promise_id: String, error: String) -> Result<()> {
        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        dispatch_client
            .reject_promise(&promise_id, &error)
            .await
            .map_err(from_core_error)?;

        Ok(())
    }

    /// Send a signal to an existing workflow.
    #[napi]
    pub async fn signal_workflow(
        &self,
        workflow_execution_id: String,
        signal_name: String,
        signal_value: String,
    ) -> Result<SignalWorkflowResult> {
        let value_bytes = signal_value.into_bytes();

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        let result = dispatch_client
            .signal_workflow(
                &self.config.org_id,
                &workflow_execution_id,
                &signal_name,
                value_bytes,
            )
            .await
            .map_err(from_core_error)?;

        Ok(SignalWorkflowResult {
            signal_event_sequence: result.signal_event_sequence,
        })
    }

    /// Send a signal to an existing workflow, or create a new workflow and send the signal.
    #[napi]
    pub async fn signal_with_start_workflow(
        &self,
        workflow_id: String,
        workflow_kind: String,
        workflow_input: String,
        queue: Option<String>,
        signal_name: String,
        signal_value: String,
    ) -> Result<SignalWithStartResult> {
        let input_bytes = workflow_input.into_bytes();
        let signal_bytes = signal_value.into_bytes();

        let mut dispatch_client = WorkflowDispatch::new(self.channel.clone(), &self.client_token);

        let result = dispatch_client
            .signal_with_start_workflow(
                &self.config.org_id,
                &workflow_id,
                &workflow_kind,
                input_bytes,
                queue.as_deref().unwrap_or("default"),
                &signal_name,
                signal_bytes,
                None, // priority_seconds
                None, // workflow_version
                None, // metadata
                None, // idempotency_key_ttl_seconds
            )
            .await
            .map_err(from_core_error)?;

        Ok(SignalWithStartResult {
            workflow_execution_id: result.workflow_execution_id.to_string(),
            workflow_created: result.workflow_created,
            signal_event_sequence: result.signal_event_sequence,
        })
    }

    /// Get the server URL.
    #[napi(getter)]
    pub fn server_url(&self) -> String {
        self.config.server_url.clone()
    }

    /// Get the org ID.
    #[napi(getter)]
    pub fn org_id(&self) -> String {
        self.config.org_id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_workflow_options_default() {
        let options = StartWorkflowOptions {
            queue: None,
            workflow_version: None,
            idempotency_key: None,
        };
        assert!(options.queue.is_none());
    }
}
