//! Configuration types for FFI.
//!
//! These records define the configuration needed to create workers and clients.

/// Configuration for creating a CoreWorker.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WorkerConfig {
    /// Server URL (e.g., "http://localhost:9090").
    pub server_url: String,

    /// Namespace for the worker.
    pub namespace: String,

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
            namespace: "default".to_string(),
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

    /// Namespace for operations.
    pub namespace: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:9090".to_string(),
            namespace: "default".to_string(),
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
        assert_eq!(config.namespace, "default");
        assert_eq!(config.task_queue, "default");
        assert!(config.worker_identity.is_none());
    }

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert_eq!(config.server_url, "http://localhost:9090");
        assert_eq!(config.namespace, "default");
    }
}
