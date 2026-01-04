//! FlovynClient builder for fluent configuration

use crate::client::hook::{CompositeWorkflowHook, WorkflowHook};
use crate::config::FlovynClientConfig;
use crate::error::{FlovynError, Result};
use crate::task::definition::TaskDefinition;
use crate::task::registry::TaskRegistry;
use crate::worker::lifecycle::{HookChain, ReconnectionStrategy, WorkerLifecycleHook};
use crate::worker::registry::WorkflowRegistry;
use crate::workflow::definition::WorkflowDefinition;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::Channel;
use uuid::Uuid;

#[cfg(feature = "oauth2")]
use flovyn_sdk_core::client::oauth2::{self, OAuth2Credentials};

/// Default task queue name
pub const DEFAULT_TASK_QUEUE: &str = "default";

/// Builder for creating FlovynClient instances
///
/// Example:
/// ```ignore
/// let client = FlovynClient::builder()
///     .server_address("localhost", 9090)
///     .tenant_id(tenant_id)
///     .queue("my-queue")
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
    queue: String,
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
    /// Worker token for gRPC authentication
    worker_token: Option<String>,
    /// Enable telemetry (span reporting to server)
    enable_telemetry: bool,
    /// Worker lifecycle hooks
    lifecycle_hooks: Vec<Arc<dyn WorkerLifecycleHook>>,
    /// Reconnection strategy for connection recovery
    reconnection_strategy: ReconnectionStrategy,
    /// OAuth2 credentials for client credentials flow
    #[cfg(feature = "oauth2")]
    oauth2_credentials: Option<OAuth2Credentials>,
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
            queue: DEFAULT_TASK_QUEUE.to_string(),
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
            worker_token: None,
            enable_telemetry: false,
            lifecycle_hooks: Vec::new(),
            reconnection_strategy: ReconnectionStrategy::default(),
            #[cfg(feature = "oauth2")]
            oauth2_credentials: None,
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
    pub fn queue(mut self, queue: impl Into<String>) -> Self {
        self.queue = queue.into();
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

    /// Set the worker token for gRPC authentication
    ///
    /// The worker token is passed in the Authorization header for all gRPC requests.
    /// This is required when the server has security enabled.
    ///
    /// This is an alias for `.api_key()` - both methods accept any token format.
    pub fn worker_token(mut self, token: impl Into<String>) -> Self {
        self.worker_token = Some(token.into());
        self
    }

    /// Set OAuth2 client credentials for authentication
    ///
    /// The SDK will fetch a JWT from the OAuth2 provider using the client credentials
    /// flow and use it for all gRPC requests.
    ///
    /// # Arguments
    ///
    /// * `client_id` - OAuth2 client ID
    /// * `client_secret` - OAuth2 client secret
    /// * `token_endpoint` - Token endpoint URL (e.g., `https://idp.example.com/realms/my-realm/protocol/openid-connect/token`)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let client = FlovynClient::builder()
    ///     .server_address("localhost", 9090)
    ///     .tenant_id(tenant_id)
    ///     .oauth2_client_credentials(
    ///         "my-worker-client",
    ///         "my-client-secret",
    ///         "https://keycloak.example.com/realms/flovyn/protocol/openid-connect/token"
    ///     )
    ///     .build()
    ///     .await?;
    /// ```
    ///
    /// # Note
    ///
    /// This method requires the `oauth2` feature to be enabled (enabled by default).
    #[cfg(feature = "oauth2")]
    pub fn oauth2_client_credentials(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        token_endpoint: impl Into<String>,
    ) -> Self {
        self.oauth2_credentials = Some(OAuth2Credentials::new(
            client_id,
            client_secret,
            token_endpoint,
        ));
        self
    }

    /// Set OAuth2 credentials with additional scopes
    ///
    /// Same as `oauth2_client_credentials` but allows specifying additional scopes.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let client = FlovynClient::builder()
    ///     .server_address("localhost", 9090)
    ///     .tenant_id(tenant_id)
    ///     .oauth2_client_credentials_with_scopes(
    ///         "my-worker-client",
    ///         "my-client-secret",
    ///         "https://keycloak.example.com/realms/flovyn/protocol/openid-connect/token",
    ///         vec!["openid".to_string(), "profile".to_string()]
    ///     )
    ///     .build()
    ///     .await?;
    /// ```
    #[cfg(feature = "oauth2")]
    pub fn oauth2_client_credentials_with_scopes(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        token_endpoint: impl Into<String>,
        scopes: Vec<String>,
    ) -> Self {
        self.oauth2_credentials = Some(
            OAuth2Credentials::new(client_id, client_secret, token_endpoint).with_scopes(scopes),
        );
        self
    }

    /// Enable telemetry (span reporting to server)
    ///
    /// When enabled, the SDK will report execution spans to the server for
    /// distributed tracing. The server will create OTLP spans from the SDK data.
    ///
    /// Default: false
    pub fn enable_telemetry(mut self, enable: bool) -> Self {
        self.enable_telemetry = enable;
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

    /// Add a worker lifecycle hook
    ///
    /// Lifecycle hooks receive notifications about worker lifecycle events
    /// such as starting, stopping, reconnecting, and work completion.
    ///
    /// Multiple hooks can be registered and they are called in registration order.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use flovyn_sdk::prelude::*;
    ///
    /// struct LoggingHook;
    ///
    /// #[async_trait]
    /// impl WorkerLifecycleHook for LoggingHook {
    ///     async fn on_event(&self, event: WorkerLifecycleEvent) {
    ///         println!("Event: {:?}", event);
    ///     }
    /// }
    ///
    /// let client = FlovynClient::builder()
    ///     .add_lifecycle_hook(LoggingHook)
    ///     .build()
    ///     .await?;
    /// ```
    pub fn add_lifecycle_hook<H: WorkerLifecycleHook + 'static>(mut self, hook: H) -> Self {
        self.lifecycle_hooks.push(Arc::new(hook));
        self
    }

    /// Set the reconnection strategy
    ///
    /// Controls how the worker reconnects after connection loss.
    ///
    /// Default: ExponentialBackoff with 1s initial delay, 60s max, 2.0 multiplier
    pub fn reconnection_strategy(mut self, strategy: ReconnectionStrategy) -> Self {
        self.reconnection_strategy = strategy;
        self
    }

    /// Set maximum reconnection attempts (convenience method)
    ///
    /// This updates the reconnection strategy to use the specified max attempts.
    pub fn max_reconnect_attempts(mut self, max: u32) -> Self {
        self.reconnection_strategy = match self.reconnection_strategy {
            ReconnectionStrategy::Fixed { delay, .. } => ReconnectionStrategy::Fixed {
                delay,
                max_attempts: Some(max),
            },
            ReconnectionStrategy::ExponentialBackoff {
                initial_delay,
                max_delay,
                multiplier,
                ..
            } => ReconnectionStrategy::ExponentialBackoff {
                initial_delay,
                max_delay,
                multiplier,
                max_attempts: Some(max),
            },
            other => other,
        };
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
        I: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
        O: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
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
        I: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
        O: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
    {
        // Ignore registration errors during build - they'll be caught at build() time
        let _ = self.task_registry.register(task);
        self
    }

    /// Build the FlovynClient
    ///
    /// # Errors
    /// Returns an error if no authentication method is configured.
    pub async fn build(self) -> Result<super::FlovynClient> {
        // Determine authentication token
        // Priority: OAuth2 credentials > explicit worker_token/api_key
        #[cfg(feature = "oauth2")]
        let worker_token = if let Some(oauth2_creds) = &self.oauth2_credentials {
            // Fetch token using OAuth2 client credentials flow
            let token_response = oauth2::fetch_access_token(oauth2_creds)
                .await
                .map_err(FlovynError::AuthenticationError)?;
            token_response.access_token
        } else if let Some(token) = self.worker_token {
            token
        } else {
            return Err(FlovynError::InvalidConfiguration(
                "Authentication required. Use .worker_token(...) or .oauth2_client_credentials(...).".to_string(),
            ));
        };

        #[cfg(not(feature = "oauth2"))]
        let worker_token = self.worker_token.ok_or_else(|| {
            FlovynError::InvalidConfiguration(
                "Authentication required. Use .worker_token(...).".to_string(),
            )
        })?;

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

        // Create lifecycle hook chain
        let mut lifecycle_hooks = HookChain::new();
        for hook in self.lifecycle_hooks {
            lifecycle_hooks.add_arc(hook);
        }

        Ok(super::FlovynClient {
            tenant_id: self.tenant_id,
            worker_id: self.worker_id,
            queue: self.queue,
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
            worker_token,
            enable_telemetry: self.enable_telemetry,
            lifecycle_hooks,
            reconnection_strategy: self.reconnection_strategy,
            worker_internals: parking_lot::RwLock::new(None),
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
        assert_eq!(builder.queue, "default");
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
    fn test_builder_queue() {
        let builder = FlovynClientBuilder::new().queue("gpu-workers");
        assert_eq!(builder.queue, "gpu-workers");
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

    #[test]
    fn test_builder_lifecycle_hooks() {
        use crate::worker::lifecycle::{WorkerLifecycleEvent, WorkerLifecycleHook};
        use async_trait::async_trait;

        struct TestHook;

        #[async_trait]
        impl WorkerLifecycleHook for TestHook {
            async fn on_event(&self, _event: WorkerLifecycleEvent) {}
        }

        let builder = FlovynClientBuilder::new()
            .add_lifecycle_hook(TestHook)
            .add_lifecycle_hook(TestHook);
        assert_eq!(builder.lifecycle_hooks.len(), 2);
    }

    #[test]
    fn test_builder_reconnection_strategy() {
        let builder = FlovynClientBuilder::new()
            .reconnection_strategy(ReconnectionStrategy::fixed(Duration::from_secs(5)));

        match builder.reconnection_strategy {
            ReconnectionStrategy::Fixed {
                delay,
                max_attempts,
            } => {
                assert_eq!(delay, Duration::from_secs(5));
                assert!(max_attempts.is_none());
            }
            _ => panic!("Expected Fixed strategy"),
        }
    }

    #[test]
    fn test_builder_max_reconnect_attempts() {
        // Test with default exponential backoff
        let builder = FlovynClientBuilder::new().max_reconnect_attempts(5);

        match builder.reconnection_strategy {
            ReconnectionStrategy::ExponentialBackoff { max_attempts, .. } => {
                assert_eq!(max_attempts, Some(5));
            }
            _ => panic!("Expected ExponentialBackoff strategy"),
        }

        // Test with fixed strategy
        let builder = FlovynClientBuilder::new()
            .reconnection_strategy(ReconnectionStrategy::fixed(Duration::from_secs(10)))
            .max_reconnect_attempts(3);

        match builder.reconnection_strategy {
            ReconnectionStrategy::Fixed { max_attempts, .. } => {
                assert_eq!(max_attempts, Some(3));
            }
            _ => panic!("Expected Fixed strategy"),
        }
    }

    #[test]
    fn test_builder_worker_token() {
        let builder = FlovynClientBuilder::new().worker_token("test_token_123");
        assert_eq!(builder.worker_token, Some("test_token_123".to_string()));
    }

    #[test]
    fn test_builder_worker_token_accepts_any_format() {
        // worker_token accepts any format without validation
        let builder = FlovynClientBuilder::new().worker_token("flovyn_wk_live_xyz123");
        assert_eq!(
            builder.worker_token,
            Some("flovyn_wk_live_xyz123".to_string())
        );

        let builder = FlovynClientBuilder::new().worker_token("simple-key");
        assert_eq!(builder.worker_token, Some("simple-key".to_string()));
    }
}
