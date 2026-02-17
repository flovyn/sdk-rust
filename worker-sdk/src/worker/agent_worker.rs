//! AgentExecutorWorker - Polling service for agent execution

use crate::agent::context_impl::AgentContextImpl;
use crate::agent::executor::TaskExecutor;
use crate::agent::registry::AgentRegistry;
use crate::agent::signals::SignalSource;
use crate::agent::storage::AgentStorage;
use crate::error::{FlovynError, Result};
use crate::worker::lifecycle::{
    HookChain, ReconnectionStrategy, StopReason, WorkType, WorkerInternals,
};
use flovyn_worker_core::client::{AgentDispatch, WorkerLifecycleClient};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, Notify, Semaphore};
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Configuration for AgentExecutorWorker
#[derive(Clone)]
pub struct AgentWorkerConfig {
    /// Unique worker identifier
    pub worker_id: String,
    /// Org ID
    pub org_id: Uuid,
    /// Queue to poll agents from
    pub queue: String,
    /// Long polling timeout
    pub poll_timeout: Duration,
    /// Interval to wait before polling again when no agent is available
    pub no_work_backoff: Duration,
    /// Maximum concurrent agent executions
    pub max_concurrent: usize,
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Human-readable worker name for registration
    pub worker_name: Option<String>,
    /// Worker version for registration
    pub worker_version: String,
    /// Team ID (None = org-level)
    pub team_id: Option<Uuid>,
    /// Enable automatic worker registration on startup
    pub enable_auto_registration: bool,
    /// Worker token for gRPC authentication
    pub worker_token: String,
    /// Worker lifecycle hooks
    pub lifecycle_hooks: HookChain,
    /// Reconnection strategy for connection recovery
    pub reconnection_strategy: ReconnectionStrategy,
    /// Pre-registered server worker ID (from unified registration)
    /// If set, skip auto-registration and use this ID
    pub server_worker_id: Option<Uuid>,
    /// Custom agent storage backend (None = use RemoteStorage)
    pub agent_storage: Option<Arc<dyn AgentStorage>>,
    /// Custom agent task executor (None = use RemoteTaskExecutor)
    pub agent_task_executor: Option<Arc<dyn TaskExecutor>>,
    /// Custom agent signal source (None = use RemoteSignalSource)
    pub agent_signal_source: Option<Arc<dyn SignalSource>>,
}

impl Default for AgentWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: Uuid::new_v4().to_string(),
            org_id: Uuid::nil(),
            queue: "default".to_string(),
            poll_timeout: Duration::from_secs(60),
            no_work_backoff: Duration::from_millis(100),
            max_concurrent: 5,
            heartbeat_interval: Duration::from_secs(30),
            worker_name: None,
            worker_version: "1.0.0".to_string(),
            team_id: None,
            enable_auto_registration: true,
            worker_token: String::new(),
            lifecycle_hooks: HookChain::new(),
            reconnection_strategy: ReconnectionStrategy::default(),
            server_worker_id: None,
            agent_storage: None,
            agent_task_executor: None,
            agent_signal_source: None,
        }
    }
}

impl std::fmt::Debug for AgentWorkerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentWorkerConfig")
            .field("worker_id", &self.worker_id)
            .field("org_id", &self.org_id)
            .field("queue", &self.queue)
            .field("poll_timeout", &self.poll_timeout)
            .field("max_concurrent", &self.max_concurrent)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("worker_name", &self.worker_name)
            .field("worker_version", &self.worker_version)
            .field("team_id", &self.team_id)
            .field("enable_auto_registration", &self.enable_auto_registration)
            .field("worker_token", &"<redacted>")
            .field("lifecycle_hooks_count", &self.lifecycle_hooks.len())
            .field("reconnection_strategy", &self.reconnection_strategy)
            .finish()
    }
}

/// Worker service that polls for agents and executes them
pub struct AgentExecutorWorker {
    config: AgentWorkerConfig,
    registry: Arc<AgentRegistry>,
    channel: Channel,
    running: Arc<AtomicBool>,
    semaphore: Arc<Semaphore>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    /// Server-assigned worker ID from registration
    server_worker_id: Option<Uuid>,
    /// Notify when worker is ready
    ready_notify: Arc<Notify>,
    /// External shutdown signal receiver
    external_shutdown_rx: Option<watch::Receiver<bool>>,
    /// Shared internal state for lifecycle tracking
    internals: Arc<WorkerInternals>,
}

impl AgentExecutorWorker {
    /// Create a new agent worker with the given configuration
    pub fn new(config: AgentWorkerConfig, registry: Arc<AgentRegistry>, channel: Channel) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));

        // Create worker internals with lifecycle hooks from config
        let internals = Arc::new(WorkerInternals::new(
            config.worker_id.clone(),
            config.worker_name.clone(),
            config.lifecycle_hooks.clone(),
        ));

        Self {
            config,
            registry,
            channel,
            running: Arc::new(AtomicBool::new(false)),
            semaphore,
            shutdown_tx: None,
            server_worker_id: None,
            ready_notify: Arc::new(Notify::new()),
            external_shutdown_rx: None,
            internals,
        }
    }

    /// Get the worker internals for status/metrics access.
    pub fn internals(&self) -> Arc<WorkerInternals> {
        self.internals.clone()
    }

    /// Set shared worker internals for lifecycle tracking.
    ///
    /// This replaces the internally created WorkerInternals with a shared one,
    /// allowing multiple workers to share the same status, metrics, and events.
    pub fn with_internals(mut self, internals: Arc<WorkerInternals>) -> Self {
        self.internals = internals;
        self
    }

    /// Set the external shutdown signal receiver
    pub fn with_shutdown_signal(mut self, rx: watch::Receiver<bool>) -> Self {
        self.external_shutdown_rx = Some(rx);
        self
    }

    /// Get the ready notification handle
    pub fn ready_notify(&self) -> Arc<Notify> {
        self.ready_notify.clone()
    }

    /// Get the running flag for external shutdown control
    pub fn running_flag(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    /// Check if the worker is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Run heartbeat loop
    async fn heartbeat_loop(
        channel: Channel,
        worker_token: String,
        server_worker_id: Option<Uuid>,
        heartbeat_interval: Duration,
        running: Arc<AtomicBool>,
        internals: Arc<WorkerInternals>,
    ) {
        if server_worker_id.is_none() {
            debug!("No server worker ID, skipping heartbeat loop");
            return;
        }

        let worker_id = server_worker_id.unwrap();
        let mut client = WorkerLifecycleClient::new(channel, &worker_token);

        while running.load(Ordering::SeqCst) {
            tokio::time::sleep(heartbeat_interval).await;

            if !running.load(Ordering::SeqCst) {
                break;
            }

            match client.send_heartbeat(worker_id).await {
                Ok(()) => {
                    debug!("Heartbeat sent successfully");
                    internals.record_heartbeat_success().await;
                }
                Err(e) => {
                    warn!("Failed to send heartbeat: {}", e);
                    internals.record_heartbeat_failure(e.to_string()).await;
                }
            }
        }
    }

    /// Start the worker
    pub async fn start(&mut self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(FlovynError::Other("Worker already running".to_string()));
        }

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Clone external shutdown receiver if present
        let mut external_shutdown_rx = self.external_shutdown_rx.clone();

        info!(
            worker_id = %self.config.worker_id,
            org_id = %self.config.org_id,
            queue = %self.config.queue,
            "Starting agent worker"
        );

        // Only emit Starting event if we're doing auto-registration
        // (unified registration already set the status to Running)
        if self.config.enable_auto_registration {
            self.internals.record_starting().await;
        }

        // Use the server_worker_id from config (set by unified registration in FlovynClient)
        self.server_worker_id = self.config.server_worker_id;

        // Start heartbeat loop in background
        let heartbeat_running = self.running.clone();
        let heartbeat_channel = self.channel.clone();
        let heartbeat_worker_token = self.config.worker_token.clone();
        let heartbeat_worker_id = self.server_worker_id;
        let heartbeat_interval = self.config.heartbeat_interval;
        let heartbeat_internals = self.internals.clone();

        tokio::spawn(async move {
            Self::heartbeat_loop(
                heartbeat_channel,
                heartbeat_worker_token,
                heartbeat_worker_id,
                heartbeat_interval,
                heartbeat_running,
                heartbeat_internals,
            )
            .await;
        });

        // Signal that worker is ready
        self.ready_notify.notify_one();

        // Emit Ready event
        self.internals.record_ready(self.server_worker_id).await;

        // Clone components needed for the polling loop
        let running = self.running.clone();
        let semaphore = self.semaphore.clone();
        let registry = self.registry.clone();
        let config = self.config.clone();
        let channel = self.channel.clone();

        // Polling loop with reconnection and pause support
        let mut reconnect_attempt: u32 = 0;
        let reconnection_strategy = self.config.reconnection_strategy.clone();
        let mut pause_rx = self.internals.pause_receiver();

        while running.load(Ordering::SeqCst) {
            // Check if paused - if so, wait for resume
            if *pause_rx.borrow() {
                debug!("Agent worker is paused, waiting for resume signal");
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!("Received shutdown signal while paused");
                        break;
                    }
                    _ = async {
                        if let Some(ref mut rx) = external_shutdown_rx {
                            let _ = rx.changed().await;
                        } else {
                            std::future::pending::<()>().await
                        }
                    } => {
                        debug!("Received external shutdown signal while paused");
                        break;
                    }
                    _ = pause_rx.changed() => {
                        // Pause state changed - loop back to check
                        continue;
                    }
                }
            }

            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!("Received shutdown signal");
                    break;
                }
                _ = async {
                    if let Some(ref mut rx) = external_shutdown_rx {
                        let _ = rx.changed().await;
                    } else {
                        std::future::pending::<()>().await
                    }
                } => {
                    debug!("Received external shutdown signal");
                    break;
                }
                _ = pause_rx.changed() => {
                    // Pause state changed - loop back to check
                    continue;
                }
                result = Self::poll_and_execute(
                    semaphore.clone(),
                    registry.clone(),
                    &config,
                    channel.clone(),
                    running.clone(),
                    self.internals.clone(),
                ) => {
                    if let Err(e) = result {
                        // Check if it's a connection error
                        let err_str = e.to_string();
                        let is_h2_reset = err_str.contains("h2 protocol error")
                            || err_str.contains("connection error received: not a result of an error")
                            || (err_str.contains("transport error") && err_str.contains("Unknown"));
                        let is_connection_error = !is_h2_reset
                            && (err_str.contains("UNAVAILABLE")
                                || err_str.contains("Connection refused")
                                || err_str.contains("connection error"));

                        if is_h2_reset {
                            // HTTP/2 connection recycled by proxy — transient, retry immediately
                            debug!("HTTP/2 connection reset, retrying: {}", e);
                            continue;
                        } else if is_connection_error {
                            warn!("Server unavailable (attempt {}), will retry: {}", reconnect_attempt + 1, e);

                            // Record disconnected state on first failure
                            if reconnect_attempt == 0 {
                                self.internals.record_disconnected(e.to_string()).await;
                            }

                            // Calculate delay using reconnection strategy
                            match reconnection_strategy.calculate_delay(reconnect_attempt) {
                                Some(delay) => {
                                    reconnect_attempt += 1;
                                    self.internals.record_reconnecting(reconnect_attempt, Some(e.to_string())).await;
                                    debug!("Waiting {:?} before reconnection attempt {}", delay, reconnect_attempt);
                                    tokio::time::sleep(delay).await;
                                }
                                None => {
                                    // Max attempts reached
                                    error!("Max reconnection attempts ({}) reached, stopping worker", reconnect_attempt);
                                    break;
                                }
                            }
                        } else {
                            error!("Polling loop error: {}", e);
                            // Non-connection error, use short delay
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    } else if reconnect_attempt > 0 {
                        // Successfully reconnected
                        info!("Successfully reconnected to server after {} attempts", reconnect_attempt);
                        self.internals.record_reconnected(self.server_worker_id).await;
                        reconnect_attempt = 0;
                    }
                }
            }
        }

        running.store(false, Ordering::SeqCst);

        // Emit Stopped event
        self.internals.record_stopped(StopReason::Graceful).await;

        info!("Agent worker stopped");
        Ok(())
    }

    /// Stop the worker gracefully
    pub fn stop(&mut self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return; // Already stopped
        }

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }
    }

    /// Poll for an agent and execute it (static version for concurrent execution)
    #[allow(clippy::too_many_arguments)]
    async fn poll_and_execute(
        semaphore: Arc<Semaphore>,
        registry: Arc<AgentRegistry>,
        config: &AgentWorkerConfig,
        channel: Channel,
        _running: Arc<AtomicBool>,
        internals: Arc<WorkerInternals>,
    ) -> Result<()> {
        // Acquire semaphore permit
        let permit = semaphore
            .acquire_owned()
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to acquire semaphore: {}", e)))?;

        // Get registered agent capabilities
        let agent_capabilities = registry.get_registered_kinds();

        // Use server_worker_id (UUID) for polling, not the client-side worker_id
        let worker_id = config
            .server_worker_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| config.worker_id.clone());

        // Create client for polling
        let mut client = AgentDispatch::new(channel.clone(), &config.worker_token);

        // Poll for agent
        let agent_info = client
            .poll_agent(
                &worker_id,
                &config.org_id.to_string(),
                &config.queue,
                config.poll_timeout,
                agent_capabilities,
            )
            .await
            .map_err(|e| FlovynError::Other(format!("gRPC error: {}", e)))?;

        match agent_info {
            Some(info) => {
                let agent_execution_id = info.id;
                let agent_kind = info.kind.clone();

                // Record work received
                internals
                    .record_work_received(WorkType::Agent, agent_execution_id)
                    .await;

                debug!(
                    agent_execution_id = %agent_execution_id,
                    agent_kind = %agent_kind,
                    "Executing agent"
                );

                // Clone what we need for spawned task
                let config = config.clone();
                let internals = internals.clone();

                // Spawn execution in background
                tokio::spawn(async move {
                    let _permit = permit; // Keep permit alive during execution

                    let start_time = std::time::Instant::now();

                    // Look up agent in registry
                    let agent = match registry.get(&agent_kind) {
                        Some(a) => a,
                        None => {
                            error!(
                                agent_kind = %agent_kind,
                                "Agent kind not registered"
                            );
                            // Fail the agent since we can't execute it
                            let mut client =
                                AgentDispatch::new(channel.clone(), &config.worker_token);
                            if let Err(e) = client
                                .fail_agent(
                                    agent_execution_id,
                                    &format!(
                                        "Agent kind '{}' not registered on this worker",
                                        agent_kind
                                    ),
                                    None,
                                    Some("AGENT_NOT_FOUND"),
                                )
                                .await
                            {
                                error!("Failed to report agent failure: {}", e);
                            }
                            return;
                        }
                    };

                    // Clone input before moving it to context
                    let agent_input = info.input.clone();

                    // Create context from loaded data
                    let ctx = match Self::create_agent_context_static(
                        channel.clone(),
                        &config.worker_token,
                        agent_execution_id,
                        info.org_id,
                        info.input,
                        info.current_checkpoint_seq,
                        config.agent_storage.clone(),
                        config.agent_task_executor.clone(),
                        config.agent_signal_source.clone(),
                    )
                    .await
                    {
                        Ok(mut ctx) => {
                            // Propagate parent_execution_id from PollAgent response
                            ctx.set_parent_execution_id(info.parent_execution_id);
                            // Set queue context for child agent queue resolution
                            ctx.set_queue_context(
                                crate::agent::queue::QueueContext::from_worker_queue(
                                    config.queue.clone(),
                                ),
                            );
                            ctx
                        }
                        Err(e) => {
                            error!(
                                agent_execution_id = %agent_execution_id,
                                error = %e,
                                "Failed to create agent context"
                            );
                            let mut client =
                                AgentDispatch::new(channel.clone(), &config.worker_token);
                            if let Err(e) = client
                                .fail_agent(
                                    agent_execution_id,
                                    &format!("Failed to create agent context: {}", e),
                                    None,
                                    Some("CONTEXT_CREATION_FAILED"),
                                )
                                .await
                            {
                                error!("Failed to report agent failure: {}", e);
                            }
                            return;
                        }
                    };

                    // Execute the agent
                    let ctx_arc: Arc<dyn crate::agent::context::AgentContext + Send + Sync> =
                        Arc::new(ctx);
                    let result = agent.execute(ctx_arc.clone(), agent_input).await;

                    // Record work completion metrics
                    let duration = start_time.elapsed();

                    // Report result to server
                    let mut client = AgentDispatch::new(channel.clone(), &config.worker_token);
                    match result {
                        Ok(output) => {
                            debug!(
                                agent_execution_id = %agent_execution_id,
                                "Agent completed successfully"
                            );
                            // Flush any pending entries before completing.
                            if let Err(e) = ctx_arc.flush_pending().await {
                                warn!(
                                    agent_execution_id = %agent_execution_id,
                                    error = %e,
                                    "Failed to flush pending entries before completion"
                                );
                            }
                            if let Err(e) = client.complete_agent(agent_execution_id, &output).await
                            {
                                error!("Failed to report agent completion: {}", e);
                            }

                            internals
                                .record_work_completed(
                                    WorkType::Agent,
                                    agent_execution_id,
                                    duration,
                                )
                                .await;
                        }
                        Err(e) => {
                            // Check if it's a suspension
                            if matches!(e, FlovynError::AgentSuspended(_)) {
                                debug!(
                                    agent_execution_id = %agent_execution_id,
                                    reason = %e,
                                    "Agent suspended"
                                );
                                internals
                                    .record_work_completed(
                                        WorkType::Agent,
                                        agent_execution_id,
                                        duration,
                                    )
                                    .await;
                            } else {
                                debug!(
                                    agent_execution_id = %agent_execution_id,
                                    error = %e,
                                    "Agent failed"
                                );
                                if let Err(err) = client
                                    .fail_agent(agent_execution_id, &e.to_string(), None, None)
                                    .await
                                {
                                    error!("Failed to report agent failure: {}", err);
                                }

                                internals
                                    .record_work_failed(
                                        WorkType::Agent,
                                        agent_execution_id,
                                        e.to_string(),
                                        duration,
                                    )
                                    .await;
                            }
                        }
                    }
                });

                Ok(())
            }
            None => {
                // No agent available - release permit and wait before next poll
                drop(permit);
                tokio::time::sleep(config.no_work_backoff).await;
                Ok(())
            }
        }
    }

    /// Create an AgentContextImpl from loaded agent data (static version for spawned tasks)
    #[allow(clippy::too_many_arguments)]
    async fn create_agent_context_static(
        channel: Channel,
        worker_token: &str,
        agent_execution_id: Uuid,
        org_id: Uuid,
        input: serde_json::Value,
        current_checkpoint_seq: i32,
        storage: Option<Arc<dyn AgentStorage>>,
        task_executor: Option<Arc<dyn TaskExecutor>>,
        signal_source: Option<Arc<dyn SignalSource>>,
    ) -> Result<AgentContextImpl> {
        // Create a fresh client for the context
        let ctx_client = AgentDispatch::new(channel, worker_token);

        if storage.is_some() || task_executor.is_some() || signal_source.is_some() {
            // Use custom backends, falling back to defaults for any not specified
            use crate::agent::executor::RemoteTaskExecutor;
            use crate::agent::signals::RemoteSignalSource;
            use crate::agent::storage::RemoteStorage;

            let storage =
                storage.unwrap_or_else(|| Arc::new(RemoteStorage::new(ctx_client.clone(), org_id)));
            let task_executor =
                task_executor.unwrap_or_else(|| Arc::new(RemoteTaskExecutor::new()));
            let signal_source =
                signal_source.unwrap_or_else(|| Arc::new(RemoteSignalSource::new()));

            AgentContextImpl::with_backends(
                ctx_client,
                agent_execution_id,
                org_id,
                input,
                current_checkpoint_seq,
                storage,
                task_executor,
                signal_source,
            )
            .await
        } else {
            // Use defaults (existing behavior)
            AgentContextImpl::new(
                ctx_client,
                agent_execution_id,
                org_id,
                input,
                current_checkpoint_seq,
            )
            .await
        }
    }

    /// Get the agent registry
    pub fn registry(&self) -> &AgentRegistry {
        &self.registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_worker_config_default() {
        let config = AgentWorkerConfig::default();
        assert_eq!(config.queue, "default");
        assert_eq!(config.poll_timeout, Duration::from_secs(60));
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.worker_version, "1.0.0");
        assert!(config.enable_auto_registration);
        assert!(config.worker_name.is_none());
        assert!(config.team_id.is_none());
    }

    #[test]
    fn test_agent_worker_config_custom() {
        let config = AgentWorkerConfig {
            worker_id: "agent-worker-1".to_string(),
            org_id: Uuid::new_v4(),
            queue: "agents".to_string(),
            poll_timeout: Duration::from_secs(30),
            no_work_backoff: Duration::from_millis(100),
            max_concurrent: 10,
            heartbeat_interval: Duration::from_secs(15),
            worker_name: Some("Agent Worker".to_string()),
            worker_version: "2.0.0".to_string(),
            team_id: Some(Uuid::new_v4()),
            enable_auto_registration: false,
            worker_token: "test-token".to_string(),
            lifecycle_hooks: HookChain::new(),
            reconnection_strategy: ReconnectionStrategy::fixed(Duration::from_secs(5)),
            server_worker_id: None,
            agent_storage: None,
            agent_task_executor: None,
            agent_signal_source: None,
        };

        assert_eq!(config.worker_id, "agent-worker-1");
        assert_eq!(config.queue, "agents");
        assert_eq!(config.max_concurrent, 10);
        assert_eq!(config.worker_name, Some("Agent Worker".to_string()));
        assert_eq!(config.worker_version, "2.0.0");
        assert!(!config.enable_auto_registration);
    }

    #[test]
    fn test_agent_worker_config_debug() {
        let config = AgentWorkerConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("AgentWorkerConfig"));
        assert!(debug_str.contains("queue"));
        assert!(debug_str.contains("<redacted>")); // Token should be redacted
    }
}
