//! Configuration types for FFI.
//!
//! These records define the configuration needed to create workers and clients.

/// OAuth2 client credentials for authentication.
#[derive(Debug, Clone, uniffi::Record)]
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

/// Workflow metadata passed from Kotlin/Swift to Rust FFI.
/// Contains full metadata including optional schemas.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WorkflowMetadataFfi {
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

/// Task metadata passed from Kotlin/Swift to Rust FFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TaskMetadataFfi {
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

/// Configuration for creating a CoreWorker.
#[derive(Debug, Clone, uniffi::Record)]
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

    /// Tenant ID (UUID format) for worker registration.
    pub tenant_id: String,

    /// Task queue to poll for work.
    pub queue: String,

    /// Optional worker identity for debugging/monitoring.
    pub worker_identity: Option<String>,

    /// Maximum concurrent workflow tasks (default: 100).
    pub max_concurrent_workflow_tasks: Option<u32>,

    /// Maximum concurrent activity tasks (default: 100).
    pub max_concurrent_tasks: Option<u32>,

    /// Workflow metadata for this worker (replaces workflow_kinds).
    pub workflow_metadata: Vec<WorkflowMetadataFfi>,

    /// Task metadata for this worker (replaces task_kinds).
    pub task_metadata: Vec<TaskMetadataFfi>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:9090".to_string(),
            worker_token: None,
            oauth2_credentials: None,
            tenant_id: "00000000-0000-0000-0000-000000000000".to_string(),
            queue: "default".to_string(),
            worker_identity: None,
            max_concurrent_workflow_tasks: Some(100),
            max_concurrent_tasks: Some(100),
            workflow_metadata: vec![],
            task_metadata: vec![],
        }
    }
}

/// Configuration for creating a CoreClient.
#[derive(Debug, Clone, uniffi::Record)]
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

    /// Tenant ID (UUID format) for operations.
    pub tenant_id: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:9090".to_string(),
            client_token: None,
            oauth2_credentials: None,
            tenant_id: "00000000-0000-0000-0000-000000000000".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_config_default() {
        let config = WorkerConfig::default();
        assert_eq!(config.server_url, "http://localhost:9090");
        assert!(config.worker_token.is_none());
        assert_eq!(config.tenant_id, "00000000-0000-0000-0000-000000000000");
        assert_eq!(config.queue, "default");
        assert!(config.worker_identity.is_none());
    }

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert_eq!(config.server_url, "http://localhost:9090");
        assert!(config.client_token.is_none());
        assert_eq!(config.tenant_id, "00000000-0000-0000-0000-000000000000");
    }
}
