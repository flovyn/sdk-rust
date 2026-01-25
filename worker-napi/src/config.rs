//! Configuration types for NAPI bindings.
//!
//! These objects define the configuration needed to create workers and clients.

use napi_derive::napi;

/// OAuth2 client credentials for authentication.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct OAuth2Credentials {
    /// OAuth2 client ID.
    pub client_id: String,

    /// OAuth2 client secret.
    pub client_secret: String,

    /// Token endpoint URL (e.g., `https://keycloak.example.com/realms/myrealm/protocol/openid-connect/token`).
    pub token_endpoint: String,

    /// Optional scopes (space-separated if multiple).
    pub scopes: Option<String>,
}

/// Workflow metadata for registration.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct WorkflowMetadata {
    /// Unique workflow kind identifier (required).
    pub kind: String,

    /// Human-readable name (defaults to kind if empty).
    pub name: String,

    /// Optional description.
    pub description: Option<String>,

    /// Version string (e.g., "1.0.0").
    pub version: Option<String>,

    /// Tags for categorization.
    pub tags: Vec<String>,

    /// Whether the workflow can be cancelled.
    pub cancellable: bool,

    /// Timeout in seconds.
    pub timeout_seconds: Option<u32>,

    /// JSON Schema for input validation (JSON string).
    pub input_schema: Option<String>,

    /// JSON Schema for output validation (JSON string).
    pub output_schema: Option<String>,
}

/// Task metadata for registration.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct TaskMetadata {
    /// Unique task kind identifier (required).
    pub kind: String,

    /// Human-readable name (defaults to kind if empty).
    pub name: String,

    /// Optional description.
    pub description: Option<String>,

    /// Version string (e.g., "1.0.0").
    pub version: Option<String>,

    /// Tags for categorization.
    pub tags: Vec<String>,

    /// Whether the task can be cancelled.
    pub cancellable: bool,

    /// Timeout in seconds.
    pub timeout_seconds: Option<u32>,

    /// JSON Schema for input validation (JSON string).
    pub input_schema: Option<String>,

    /// JSON Schema for output validation (JSON string).
    pub output_schema: Option<String>,
}

/// Configuration for creating a NapiWorker.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Server URL (e.g., `http://localhost:9090`).
    pub server_url: String,

    /// Worker token for authentication (should start with "fwt_").
    /// If not provided or not starting with "fwt_", a placeholder will be used.
    /// Ignored if oauth2_credentials is provided.
    pub worker_token: Option<String>,

    /// OAuth2 client credentials for authentication.
    /// If provided, the SDK will fetch a JWT using client credentials flow.
    pub oauth2_credentials: Option<OAuth2Credentials>,

    /// Org ID (UUID format) for worker registration.
    pub org_id: String,

    /// Task queue to poll for work.
    pub queue: String,

    /// Optional worker identity for debugging/monitoring.
    pub worker_identity: Option<String>,

    /// Maximum concurrent workflow tasks (default: 100).
    pub max_concurrent_workflow_tasks: Option<u32>,

    /// Maximum concurrent activity tasks (default: 100).
    pub max_concurrent_tasks: Option<u32>,

    /// Workflow metadata for this worker.
    pub workflow_metadata: Vec<WorkflowMetadata>,

    /// Task metadata for this worker.
    pub task_metadata: Vec<TaskMetadata>,
}

/// Configuration for creating a NapiClient.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Server URL (e.g., `http://localhost:9090`).
    pub server_url: String,

    /// Client token for authentication (should start with "fct_").
    /// If not provided, a placeholder will be used.
    /// Ignored if oauth2_credentials is provided.
    pub client_token: Option<String>,

    /// OAuth2 client credentials for authentication.
    /// If provided, the SDK will fetch a JWT using client credentials flow.
    pub oauth2_credentials: Option<OAuth2Credentials>,

    /// Org ID (UUID format) for operations.
    pub org_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_config_creation() {
        let config = WorkerConfig {
            server_url: "http://localhost:9090".to_string(),
            worker_token: Some("fwt_test".to_string()),
            oauth2_credentials: None,
            org_id: "00000000-0000-0000-0000-000000000000".to_string(),
            queue: "default".to_string(),
            worker_identity: None,
            max_concurrent_workflow_tasks: Some(100),
            max_concurrent_tasks: Some(100),
            workflow_metadata: vec![],
            task_metadata: vec![],
        };
        assert_eq!(config.server_url, "http://localhost:9090");
        assert_eq!(config.queue, "default");
    }

    #[test]
    fn test_client_config_creation() {
        let config = ClientConfig {
            server_url: "http://localhost:9090".to_string(),
            client_token: Some("fct_test".to_string()),
            oauth2_credentials: None,
            org_id: "00000000-0000-0000-0000-000000000000".to_string(),
        };
        assert_eq!(config.server_url, "http://localhost:9090");
    }
}
