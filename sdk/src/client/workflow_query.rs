//! WorkflowQuery client wrapper for querying workflow state

use crate::client::auth::WorkerTokenInterceptor;
use crate::error::{FlovynError, Result};
use crate::generated::flovyn_v1;
use crate::generated::flovyn_v1::workflow_query_client::WorkflowQueryClient as GrpcWorkflowQueryClient;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use uuid::Uuid;

/// Type alias for authenticated client
type AuthClient = GrpcWorkflowQueryClient<InterceptedService<Channel, WorkerTokenInterceptor>>;

/// Client for workflow query operations
pub struct WorkflowQueryClient {
    inner: AuthClient,
}

impl WorkflowQueryClient {
    /// Create from a channel with worker token authentication
    pub fn new(channel: Channel, token: &str) -> Self {
        let interceptor = WorkerTokenInterceptor::new(token);
        Self {
            inner: GrpcWorkflowQueryClient::with_interceptor(channel, interceptor),
        }
    }

    /// Query workflow state and return raw Value
    pub async fn query(
        &mut self,
        workflow_execution_id: Uuid,
        query_name: &str,
        params: Value,
    ) -> Result<Value> {
        let params_bytes = serde_json::to_vec(&params)?;

        let request = flovyn_v1::QueryWorkflowRequest {
            workflow_execution_id: workflow_execution_id.to_string(),
            query_name: query_name.to_string(),
            params: params_bytes,
        };

        let response = self
            .inner
            .query_workflow(request)
            .await
            .map_err(FlovynError::Grpc)?;

        let resp = response.into_inner();

        // Check for error
        if let Some(error) = resp.error {
            return Err(FlovynError::Other(format!("Query failed: {}", error)));
        }

        // Parse result
        let result: Value = if resp.result.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&resp.result)?
        };

        Ok(result)
    }

    /// Query workflow state and deserialize to type T
    pub async fn query_typed<T: DeserializeOwned>(
        &mut self,
        workflow_execution_id: Uuid,
        query_name: &str,
        params: Value,
    ) -> Result<T> {
        let value = self
            .query(workflow_execution_id, query_name, params)
            .await?;
        serde_json::from_value(value).map_err(FlovynError::Serialization)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_query_client_new() {
        // Just verify the type compiles correctly
        // Actual connection tests require a running server
    }

    #[test]
    fn test_query_result_parsing() {
        // Test that JSON parsing works correctly
        let json_bytes = br#"{"status": "active", "count": 42}"#;
        let result: Value = serde_json::from_slice(json_bytes).unwrap();

        assert_eq!(result["status"], "active");
        assert_eq!(result["count"], 42);
    }

    #[test]
    fn test_query_result_typed_parsing() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct QueryResult {
            status: String,
            count: i32,
        }

        let json_bytes = br#"{"status": "active", "count": 42}"#;
        let value: Value = serde_json::from_slice(json_bytes).unwrap();
        let result: QueryResult = serde_json::from_value(value).unwrap();

        assert_eq!(result.status, "active");
        assert_eq!(result.count, 42);
    }

    #[test]
    fn test_empty_result_handling() {
        let empty_bytes: &[u8] = &[];
        // Empty bytes should not parse as JSON
        assert!(serde_json::from_slice::<Value>(empty_bytes).is_err());

        // So we handle it specially in the client
        let result = if empty_bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(empty_bytes).unwrap_or(Value::Null)
        };

        assert_eq!(result, Value::Null);
    }
}
