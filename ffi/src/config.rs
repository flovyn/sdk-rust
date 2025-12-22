//! Configuration types for FFI.
//!
//! These records define the configuration needed to create workers and clients.

/// Configuration for creating a CoreWorker.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WorkerConfig {
    /// Server URL (e.g., "http://localhost:9090").
    pub server_url: String,

    /// Worker token for authentication (should start with "fwt_").
    /// If not provided or not starting with "fwt_", a placeholder will be used.
    pub worker_token: Option<String>,

    /// Tenant ID (UUID format) for worker registration.
    pub tenant_id: String,

    /// Task queue to poll for work.
    pub task_queue: String,

    /// Optional worker identity for debugging/monitoring.
    pub worker_identity: Option<String>,

    /// Maximum concurrent workflow tasks (default: 100).
    pub max_concurrent_workflow_tasks: Option<u32>,

    /// Maximum concurrent activity tasks (default: 100).
    pub max_concurrent_tasks: Option<u32>,

    /// Workflow kinds this worker handles.
    pub workflow_kinds: Vec<String>,

    /// Task kinds this worker handles.
    pub task_kinds: Vec<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:9090".to_string(),
            worker_token: None,
            tenant_id: "00000000-0000-0000-0000-000000000000".to_string(),
            task_queue: "default".to_string(),
            worker_identity: None,
            max_concurrent_workflow_tasks: Some(100),
            max_concurrent_tasks: Some(100),
            workflow_kinds: vec![],
            task_kinds: vec![],
        }
    }
}

/// Configuration for creating a CoreClient.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ClientConfig {
    /// Server URL (e.g., "http://localhost:9090").
    pub server_url: String,

    /// Client token for authentication (should start with "fct_").
    /// If not provided, a placeholder will be used.
    pub client_token: Option<String>,

    /// Tenant ID (UUID format) for operations.
    pub tenant_id: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:9090".to_string(),
            client_token: None,
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
        assert_eq!(config.task_queue, "default");
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
