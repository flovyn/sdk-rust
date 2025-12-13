//! FlovynClient builder for fluent configuration

use crate::client::hook::{CompositeWorkflowHook, WorkflowHook};
use crate::config::FlovynClientConfig;
use crate::error::{FlovynError, Result};
use crate::task::definition::TaskDefinition;
use crate::task::registry::TaskRegistry;
use crate::worker::registry::WorkflowRegistry;
use crate::workflow::definition::WorkflowDefinition;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::Channel;
use uuid::Uuid;

/// Default task queue name
pub const DEFAULT_TASK_QUEUE: &str = "default";

/// Builder for creating FlovynClient instances
///
/// Example:
/// ```ignore
/// let client = FlovynClient::builder()
///     .server_address("localhost", 9090)
///     .tenant_id(tenant_id)
///     .task_queue("my-queue")
///     .register_workflow(MyWorkflow::new())
///     .register_task(MyTask::new())
///     .build()
///     .await?;
/// ```
pub struct FlovynClientBuilder {
    server_host: String,
    server_port: u16,
    tenant_id: Uuid,
    worker_id: String,
    task_queue: String,
    config: FlovynClientConfig,
    custom_channel: Option<Channel>,
    poll_timeout: Duration,
    space_id: Option<Uuid>,
    worker_name: Option<String>,
    worker_version: String,
    enable_auto_registration: bool,
    heartbeat_interval: Duration,
    workflow_registry: WorkflowRegistry,
    task_registry: TaskRegistry,
    hooks: Vec<Box<dyn WorkflowHook>>,
}

impl Default for FlovynClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FlovynClientBuilder {
    /// Create a new builder with default values
    pub fn new() -> Self {
        Self {
            server_host: "localhost".to_string(),
            server_port: 9090,
            tenant_id: Uuid::new_v4(),
            worker_id: format!("worker-{}", Uuid::new_v4()),
            task_queue: DEFAULT_TASK_QUEUE.to_string(),
            config: FlovynClientConfig::default(),
            custom_channel: None,
            poll_timeout: Duration::from_secs(60),
            space_id: None,
            worker_name: None,
            worker_version: "1.0.0".to_string(),
            enable_auto_registration: true,
            heartbeat_interval: Duration::from_secs(30),
            workflow_registry: WorkflowRegistry::new(),
            task_registry: TaskRegistry::new(),
            hooks: Vec::new(),
        }
    }

    /// Set the server address
    pub fn server_address(mut self, host: impl Into<String>, port: u16) -> Self {
        self.server_host = host.into();
        self.server_port = port;
        self
    }

    /// Set the tenant ID
    pub fn tenant_id(mut self, id: Uuid) -> Self {
        self.tenant_id = id;
        self
    }

    /// Set the worker ID
    pub fn worker_id(mut self, id: impl Into<String>) -> Self {
        self.worker_id = id.into();
        self
    }

    /// Set the task queue that this worker will poll from
    ///
    /// Workers only execute workflows from their assigned queue.
    ///
    /// Default: "default"
    pub fn task_queue(mut self, queue: impl Into<String>) -> Self {
        self.task_queue = queue.into();
        self
    }

    /// Set the long-polling timeout
    ///
    /// This controls how long workers wait for new work before retrying.
    ///
    /// Default: 60 seconds
    pub fn poll_timeout(mut self, timeout: Duration) -> Self {
        self.poll_timeout = timeout;
        self
    }

    /// Set a custom gRPC channel
    ///
    /// When set, server_address() is ignored.
    pub fn custom_channel(mut self, channel: Channel) -> Self {
        self.custom_channel = Some(channel);
        self
    }

    /// Set the space ID for space-scoped workflow/task registration
    pub fn space_id(mut self, id: Option<Uuid>) -> Self {
        self.space_id = id;
        self
    }

    /// Set the worker name for registration
    ///
    /// This is a human-readable name used for identification in the server.
    ///
    /// Default: Uses worker_id if not specified
    pub fn worker_name(mut self, name: impl Into<String>) -> Self {
        self.worker_name = Some(name.into());
        self
    }

    /// Set the worker version for registration
    ///
    /// Default: "1.0.0"
    pub fn worker_version(mut self, version: impl Into<String>) -> Self {
        self.worker_version = version.into();
        self
    }

    /// Enable or disable automatic worker registration
    ///
    /// Default: true
    pub fn enable_auto_registration(mut self, enable: bool) -> Self {
        self.enable_auto_registration = enable;
        self
    }

    /// Set the heartbeat interval
    ///
    /// Default: 30 seconds
    pub fn heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Set the complete client configuration
    pub fn config(mut self, config: FlovynClientConfig) -> Self {
        self.config = config;
        self
    }

    /// Set maximum concurrent workflows
    pub fn max_concurrent_workflows(mut self, max: usize) -> Self {
        self.config.workflow_config.max_concurrent = max;
        self
    }

    /// Set maximum concurrent tasks
    pub fn max_concurrent_tasks(mut self, max: usize) -> Self {
        self.config.task_config.max_concurrent = max;
        self
    }

    /// Set worker labels for task routing
    pub fn worker_labels(mut self, labels: std::collections::HashMap<String, String>) -> Self {
        self.config.worker_labels = labels;
        self
    }

    /// Register a workflow lifecycle hook for observability
    ///
    /// Hooks receive notifications about workflow execution events.
    pub fn register_hook(mut self, hook: impl WorkflowHook + 'static) -> Self {
        self.hooks.push(Box::new(hook));
        self
    }

    /// Register a workflow definition
    ///
    /// # Example
    ///
    /// ```ignore
    /// let client = FlovynClient::builder()
    ///     .server_address("localhost", 9090)
    ///     .tenant_id(tenant_id)
    ///     .register_workflow(MyWorkflow)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn register_workflow<W, I, O>(self, workflow: W) -> Self
    where
        W: WorkflowDefinition<Input = I, Output = O> + 'static,
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    {
        // Ignore registration errors during build - they'll be caught at build() time
        let _ = self.workflow_registry.register(workflow);
        self
    }

    /// Register a task definition
    ///
    /// # Example
    ///
    /// ```ignore
    /// let client = FlovynClient::builder()
    ///     .server_address("localhost", 9090)
    ///     .tenant_id(tenant_id)
    ///     .register_task(MyTask)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn register_task<T, I, O>(self, task: T) -> Self
    where
        T: TaskDefinition<Input = I, Output = O> + 'static,
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    {
        // Ignore registration errors during build - they'll be caught at build() time
        let _ = self.task_registry.register(task);
        self
    }

    /// Build the FlovynClient
    pub async fn build(self) -> Result<super::FlovynClient> {
        // Create or use provided channel
        let channel = match self.custom_channel {
            Some(ch) => ch,
            None => {
                let endpoint = format!("http://{}:{}", self.server_host, self.server_port);
                Channel::from_shared(endpoint)
                    .map_err(|e| FlovynError::InvalidConfiguration(e.to_string()))?
                    .connect()
                    .await
                    .map_err(|e| FlovynError::NetworkError(e.to_string()))?
            }
        };

        // Create composite hook if there are hooks
        let workflow_hook: Option<Arc<dyn WorkflowHook>> = if self.hooks.is_empty() {
            None
        } else if self.hooks.len() == 1 {
            // Unwrap the single hook
            let mut hooks = self.hooks;
            Some(Arc::from(hooks.remove(0)))
        } else {
            Some(Arc::new(CompositeWorkflowHook::new(self.hooks)))
        };

        Ok(super::FlovynClient {
            server_host: self.server_host,
            server_port: self.server_port,
            tenant_id: self.tenant_id,
            worker_id: self.worker_id,
            task_queue: self.task_queue,
            poll_timeout: self.poll_timeout,
            config: self.config,
            channel,
            space_id: self.space_id,
            worker_name: self.worker_name,
            worker_version: self.worker_version,
            enable_auto_registration: self.enable_auto_registration,
            heartbeat_interval: self.heartbeat_interval,
            workflow_registry: Arc::new(self.workflow_registry),
            task_registry: Arc::new(self.task_registry),
            workflow_hook,
            running: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_default() {
        let builder = FlovynClientBuilder::new();
        assert_eq!(builder.server_host, "localhost");
        assert_eq!(builder.server_port, 9090);
        assert_eq!(builder.task_queue, "default");
        assert_eq!(builder.poll_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_builder_server_address() {
        let builder = FlovynClientBuilder::new().server_address("example.com", 8080);
        assert_eq!(builder.server_host, "example.com");
        assert_eq!(builder.server_port, 8080);
    }

    #[test]
    fn test_builder_tenant_id() {
        let id = Uuid::new_v4();
        let builder = FlovynClientBuilder::new().tenant_id(id);
        assert_eq!(builder.tenant_id, id);
    }

    #[test]
    fn test_builder_task_queue() {
        let builder = FlovynClientBuilder::new().task_queue("gpu-workers");
        assert_eq!(builder.task_queue, "gpu-workers");
    }

    #[test]
    fn test_builder_poll_timeout() {
        let builder = FlovynClientBuilder::new().poll_timeout(Duration::from_secs(30));
        assert_eq!(builder.poll_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_builder_worker_id() {
        let builder = FlovynClientBuilder::new().worker_id("my-worker");
        assert_eq!(builder.worker_id, "my-worker");
    }

    #[test]
    fn test_builder_max_concurrent() {
        let builder = FlovynClientBuilder::new()
            .max_concurrent_workflows(50)
            .max_concurrent_tasks(100);
        assert_eq!(builder.config.workflow_config.max_concurrent, 50);
        assert_eq!(builder.config.task_config.max_concurrent, 100);
    }

    #[test]
    fn test_builder_worker_labels() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".to_string(), "production".to_string());

        let builder = FlovynClientBuilder::new().worker_labels(labels);
        assert_eq!(
            builder.config.worker_labels.get("env"),
            Some(&"production".to_string())
        );
    }

    #[test]
    fn test_builder_config_preset() {
        let builder = FlovynClientBuilder::new().config(FlovynClientConfig::high_throughput());
        assert_eq!(builder.config.workflow_config.max_concurrent, 50);
        assert_eq!(builder.config.task_config.max_concurrent, 100);
    }

    #[test]
    fn test_builder_auto_registration() {
        let builder = FlovynClientBuilder::new().enable_auto_registration(false);
        assert!(!builder.enable_auto_registration);
    }

    #[test]
    fn test_builder_heartbeat_interval() {
        let builder = FlovynClientBuilder::new().heartbeat_interval(Duration::from_secs(60));
        assert_eq!(builder.heartbeat_interval, Duration::from_secs(60));
    }
}
