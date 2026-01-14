//! WorkflowExecutorWorker - Polling service for workflow execution

use crate::client::hook::WorkflowHook;
use crate::client::{WorkerLifecycleClient, WorkflowDispatch, WorkflowExecutionInfo};
use crate::error::{FlovynError, Result};
use crate::telemetry::{workflow_execute_span, workflow_replay_span, SpanCollector};
use crate::worker::executor::WorkflowStatus;
use crate::worker::lifecycle::{HookChain, ReconnectionStrategy, StopReason, WorkerInternals};
use crate::worker::registry::WorkflowRegistry;
use crate::workflow::command::WorkflowCommand;
use crate::workflow::context_impl::WorkflowContextImpl;
use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::recorder::CommandCollector;
use chrono::Utc;
use flovyn_worker_core::generated::flovyn_v1;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, Notify, Semaphore};
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Configuration for WorkflowExecutorWorker
#[derive(Clone)]
pub struct WorkflowWorkerConfig {
    /// Unique worker identifier
    pub worker_id: String,
    /// Org ID
    pub org_id: Uuid,
    /// Task queue to poll from
    pub queue: String,
    /// Long polling timeout
    pub poll_timeout: Duration,
    /// Interval to wait before polling again when no workflow is available
    pub no_work_backoff: Duration,
    /// Maximum concurrent workflow executions
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
    /// Enable telemetry (span reporting to server)
    pub enable_telemetry: bool,
    /// Worker lifecycle hooks
    pub lifecycle_hooks: HookChain,
    /// Reconnection strategy for connection recovery
    pub reconnection_strategy: ReconnectionStrategy,
    /// Pre-registered server worker ID (from unified registration)
    /// If set, skip auto-registration and use this ID
    pub server_worker_id: Option<Uuid>,
}

impl Default for WorkflowWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: Uuid::new_v4().to_string(),
            org_id: Uuid::nil(),
            queue: "default".to_string(),
            poll_timeout: Duration::from_secs(60),
            no_work_backoff: Duration::from_millis(100),
            max_concurrent: 1,
            heartbeat_interval: Duration::from_secs(30),
            worker_name: None,
            worker_version: "1.0.0".to_string(),
            team_id: None,
            enable_auto_registration: true,
            worker_token: String::new(),
            enable_telemetry: false,
            lifecycle_hooks: HookChain::new(),
            reconnection_strategy: ReconnectionStrategy::default(),
            server_worker_id: None,
        }
    }
}

impl std::fmt::Debug for WorkflowWorkerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowWorkerConfig")
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
            .field("enable_telemetry", &self.enable_telemetry)
            .field("lifecycle_hooks_count", &self.lifecycle_hooks.len())
            .field("reconnection_strategy", &self.reconnection_strategy)
            .finish()
    }
}

/// Worker service that polls for workflows and executes them
pub struct WorkflowExecutorWorker {
    config: WorkflowWorkerConfig,
    registry: Arc<WorkflowRegistry>,
    client: WorkflowDispatch,
    channel: Channel,
    running: Arc<AtomicBool>,
    semaphore: Arc<Semaphore>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    /// Server-assigned worker ID from registration
    server_worker_id: Option<Uuid>,
    /// Workflow hook for lifecycle events
    workflow_hook: Option<Arc<dyn WorkflowHook>>,
    /// Notify when worker is ready
    ready_notify: Arc<Notify>,
    /// Span collector for telemetry
    span_collector: SpanCollector,
    /// External shutdown signal receiver
    external_shutdown_rx: Option<watch::Receiver<bool>>,
    /// Shared internal state for lifecycle tracking
    internals: Arc<WorkerInternals>,
}

impl WorkflowExecutorWorker {
    /// Create a new worker with the given configuration
    pub fn new(
        config: WorkflowWorkerConfig,
        registry: Arc<WorkflowRegistry>,
        channel: Channel,
    ) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
        let client = WorkflowDispatch::new(channel.clone(), &config.worker_token);
        let span_collector = SpanCollector::new(config.enable_telemetry);

        // Create worker internals with lifecycle hooks from config
        let internals = Arc::new(WorkerInternals::new(
            config.worker_id.clone(),
            config.worker_name.clone(),
            config.lifecycle_hooks.clone(),
        ));

        Self {
            config,
            registry,
            client,
            channel,
            running: Arc::new(AtomicBool::new(false)),
            semaphore,
            shutdown_tx: None,
            server_worker_id: None,
            workflow_hook: None,
            ready_notify: Arc::new(Notify::new()),
            span_collector,
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

    /// Set the workflow hook for lifecycle events
    pub fn with_hook(mut self, hook: Arc<dyn WorkflowHook>) -> Self {
        self.workflow_hook = Some(hook);
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
            "Starting workflow worker"
        );

        // Only emit Starting event if we're doing auto-registration
        // (unified registration already set the status to Running)
        if self.config.enable_auto_registration {
            self.internals.record_starting().await;
        }

        // Use the server_worker_id from config (set by unified registration in FlovynClient)
        self.server_worker_id = self.config.server_worker_id;

        // Clone what we need for the polling loop
        let running = self.running.clone();
        let semaphore = self.semaphore.clone();
        let registry = self.registry.clone();
        let mut client = self.client.clone();
        let config = self.config.clone();
        let workflow_hook = self.workflow_hook.clone();
        let span_collector = self.span_collector.clone();

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
        // Use notify_one() instead of notify_waiters() to store a permit
        // that can be consumed even if await_ready() hasn't been called yet
        self.ready_notify.notify_one();

        // Emit Ready event
        self.internals.record_ready(self.server_worker_id).await;

        // Polling loop with reconnection and pause support
        let mut reconnect_attempt: u32 = 0;
        let internals = self.internals.clone();
        let reconnection_strategy = config.reconnection_strategy.clone();
        let mut pause_rx = self.internals.pause_receiver();

        while running.load(Ordering::SeqCst) {
            // Check if paused - if so, wait for resume
            if *pause_rx.borrow() {
                debug!("Worker is paused, waiting for resume signal");
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
                    &mut client,
                    registry.clone(),
                    &config,
                    semaphore.clone(),
                    running.clone(),
                    workflow_hook.clone(),
                    span_collector.clone(),
                    internals.clone(),
                ) => {
                    if let Err(e) = result {
                        // Check if it's a connection error
                        let is_connection_error = e.to_string().contains("UNAVAILABLE")
                            || e.to_string().contains("Connection refused");

                        if is_connection_error {
                            warn!("Server unavailable (attempt {}), will retry: {}", reconnect_attempt + 1, e);

                            // Record disconnected state on first failure
                            if reconnect_attempt == 0 {
                                internals.record_disconnected(e.to_string()).await;
                            }

                            // Calculate delay using reconnection strategy
                            match reconnection_strategy.calculate_delay(reconnect_attempt) {
                                Some(delay) => {
                                    reconnect_attempt += 1;
                                    internals.record_reconnecting(reconnect_attempt, Some(e.to_string())).await;
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
                        internals.record_reconnected(self.server_worker_id).await;
                        reconnect_attempt = 0;
                    }
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);

        // Emit Stopped event
        self.internals.record_stopped(StopReason::Graceful).await;

        info!("Workflow worker stopped");
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

    /// Poll for a workflow and execute it
    #[allow(clippy::too_many_arguments)]
    async fn poll_and_execute(
        client: &mut WorkflowDispatch,
        registry: Arc<WorkflowRegistry>,
        config: &WorkflowWorkerConfig,
        semaphore: Arc<Semaphore>,
        running: Arc<AtomicBool>,
        workflow_hook: Option<Arc<dyn WorkflowHook>>,
        span_collector: SpanCollector,
        internals: Arc<WorkerInternals>,
    ) -> Result<()> {
        // Acquire semaphore permit
        let permit = semaphore
            .acquire_owned()
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to acquire semaphore: {}", e)))?;

        // Poll for workflow
        let workflow_info = client
            .poll_workflow(
                &config.worker_id,
                &config.org_id.to_string(),
                &config.queue,
                config.poll_timeout,
            )
            .await?;

        match workflow_info {
            Some(info) => {
                // Record work received
                let execution_id = info.id;
                internals
                    .record_work_received(
                        crate::worker::lifecycle::WorkType::Workflow,
                        execution_id,
                    )
                    .await;

                // Spawn execution in background
                let mut client = client.clone();
                let hook = workflow_hook.clone();
                let span_collector = span_collector.clone();
                let internals = internals.clone();

                tokio::spawn(async move {
                    let _permit = permit; // Keep permit alive during execution
                    let _running = running; // Keep reference

                    let start_time = std::time::Instant::now();
                    let result =
                        Self::execute_workflow(&mut client, &registry, info, hook, span_collector)
                            .await;
                    let duration = start_time.elapsed();

                    match result {
                        Ok(()) => {
                            internals
                                .record_work_completed(
                                    crate::worker::lifecycle::WorkType::Workflow,
                                    execution_id,
                                    duration,
                                )
                                .await;
                        }
                        Err(e) => {
                            error!("Workflow execution error: {}", e);
                            internals
                                .record_work_failed(
                                    crate::worker::lifecycle::WorkType::Workflow,
                                    execution_id,
                                    e.to_string(),
                                    duration,
                                )
                                .await;
                        }
                    }
                });

                Ok(())
            }
            None => {
                // No workflow available - release permit and wait before next poll
                drop(permit);
                tokio::time::sleep(config.no_work_backoff).await;
                Ok(())
            }
        }
    }

    /// Parse event type from string
    fn parse_event_type(s: &str) -> EventType {
        match s {
            "WORKFLOW_STARTED" => EventType::WorkflowStarted,
            "WORKFLOW_COMPLETED" => EventType::WorkflowCompleted,
            "WORKFLOW_EXECUTION_FAILED" => EventType::WorkflowExecutionFailed,
            "WORKFLOW_SUSPENDED" => EventType::WorkflowSuspended,
            "CANCELLATION_REQUESTED" => EventType::CancellationRequested,
            "OPERATION_COMPLETED" => EventType::OperationCompleted,
            "STATE_SET" => EventType::StateSet,
            "STATE_CLEARED" => EventType::StateCleared,
            "TASK_SCHEDULED" => EventType::TaskScheduled,
            "TASK_COMPLETED" => EventType::TaskCompleted,
            "TASK_FAILED" => EventType::TaskFailed,
            "PROMISE_CREATED" => EventType::PromiseCreated,
            "PROMISE_RESOLVED" => EventType::PromiseResolved,
            "PROMISE_REJECTED" => EventType::PromiseRejected,
            "PROMISE_TIMEOUT" => EventType::PromiseTimeout,
            "CHILD_WORKFLOW_INITIATED" => EventType::ChildWorkflowInitiated,
            "CHILD_WORKFLOW_STARTED" => EventType::ChildWorkflowStarted,
            "CHILD_WORKFLOW_COMPLETED" => EventType::ChildWorkflowCompleted,
            "CHILD_WORKFLOW_FAILED" => EventType::ChildWorkflowFailed,
            "TIMER_STARTED" => EventType::TimerStarted,
            "TIMER_FIRED" => EventType::TimerFired,
            "TIMER_CANCELLED" => EventType::TimerCancelled,
            _ => EventType::WorkflowStarted, // Default fallback
        }
    }

    /// Execute a single workflow
    async fn execute_workflow(
        client: &mut WorkflowDispatch,
        registry: &WorkflowRegistry,
        workflow_info: WorkflowExecutionInfo,
        workflow_hook: Option<Arc<dyn WorkflowHook>>,
        span_collector: SpanCollector,
    ) -> Result<()> {
        let workflow_id = workflow_info.id;
        let kind = &workflow_info.kind;
        let workflow_id_str = workflow_id.to_string();

        debug!(
            workflow_id = %workflow_id,
            kind = %kind,
            "Executing workflow"
        );

        // Get registered workflow
        let registered = registry
            .get(kind)
            .ok_or_else(|| FlovynError::Other(format!("Workflow kind not registered: {}", kind)))?;

        // Fetch existing events for replay
        let events = client.get_events(workflow_id, None).await?;
        let replay_events: Vec<ReplayEvent> = events
            .into_iter()
            .map(|e| {
                ReplayEvent::new(
                    e.sequence,
                    Self::parse_event_type(&e.event_type),
                    e.payload,
                    Utc::now(),
                )
            })
            .collect();

        // Record replay span if there are events to replay
        if !replay_events.is_empty() {
            let replay_span = workflow_replay_span(&workflow_id_str, replay_events.len());
            span_collector.record(replay_span.finish());
        }

        // Create workflow context
        let recorder = CommandCollector::new();
        let current_sequence = replay_events.len() as i32;
        let workflow_task_time = workflow_info.workflow_task_time_millis;

        let ctx = Arc::new(WorkflowContextImpl::new_with_telemetry(
            workflow_id,
            workflow_info.org_id,
            workflow_info.input.clone(),
            recorder,
            replay_events.clone(),
            workflow_task_time,
            span_collector.is_enabled(),
        ));

        // Call hook: workflow started
        if let Some(ref hook) = workflow_hook {
            hook.on_workflow_started(workflow_id, kind, &workflow_info.input)
                .await;
        }

        // Start workflow.execute span
        let execute_span = workflow_execute_span(&workflow_id_str, kind);

        // Get the workflow future and pin it for polling
        let mut workflow_future =
            std::pin::pin!(registered.execute(ctx.clone(), workflow_info.input.clone()));

        // Use poll_fn to intercept Poll::Pending and check for suspension signals
        // Suspension is signaled via the context's SuspensionCell (not thread-local)
        let ctx_for_poll = ctx.clone();
        let result: Result<serde_json::Value> = std::future::poll_fn(|cx| {
            // Clear any previous suspension before polling
            ctx_for_poll.clear_suspension();

            match workflow_future.as_mut().poll(cx) {
                std::task::Poll::Ready(result) => {
                    tracing::debug!(workflow_id = %workflow_id_str, "Workflow poll returned Ready");
                    std::task::Poll::Ready(result)
                }
                std::task::Poll::Pending => {
                    // Check for suspension signal in workflow context
                    if let Some(reason) = ctx_for_poll.take_suspension() {
                        tracing::debug!(
                            workflow_id = %workflow_id_str,
                            reason = %reason,
                            "Workflow poll returned Pending with suspension signal"
                        );
                        std::task::Poll::Ready(Err(FlovynError::Suspended { reason }))
                    } else {
                        // No suspension signal - this shouldn't happen in deterministic workflow
                        // Treat as generic suspension
                        tracing::warn!(
                            workflow_id = %workflow_id_str,
                            "Workflow poll returned Pending WITHOUT suspension signal"
                        );
                        std::task::Poll::Ready(Err(FlovynError::Suspended {
                            reason: "Workflow suspended without explicit reason".to_string(),
                        }))
                    }
                }
            }
        })
        .await;

        // Get commands and determine status
        let mut commands = ctx.get_commands();
        // Calculate final_sequence from commands already recorded (or replay events if no commands)
        let mut final_sequence = commands
            .iter()
            .map(|c| c.sequence_number())
            .max()
            .unwrap_or(current_sequence);

        let (status, output_commands, output_for_hook, error_info) = match result {
            Ok(output) => {
                // Add complete command if not already present
                final_sequence += 1;
                let output_clone = output.clone();
                commands.push(WorkflowCommand::CompleteWorkflow {
                    sequence_number: final_sequence,
                    output,
                });
                (
                    WorkflowStatus::Completed,
                    commands,
                    Some(output_clone),
                    None,
                )
            }
            Err(FlovynError::Suspended { reason }) => {
                final_sequence += 1;
                commands.push(WorkflowCommand::SuspendWorkflow {
                    sequence_number: final_sequence,
                    reason,
                });
                (WorkflowStatus::Suspended, commands, None, None)
            }
            Err(e) => {
                final_sequence += 1;
                let error_msg = e.to_string();
                let error_type = match &e {
                    FlovynError::DeterminismViolation(_) => "DeterminismViolation",
                    FlovynError::WorkflowFailed { .. } => "WorkflowFailed",
                    FlovynError::TaskFailed { .. } => "TaskFailed",
                    _ => "Error",
                };
                commands.push(WorkflowCommand::FailWorkflow {
                    sequence_number: final_sequence,
                    error: error_msg.clone(),
                    stack_trace: String::new(),
                    failure_type: Some("ERROR".to_string()),
                });
                // Call hook: workflow failed
                if let Some(ref hook) = workflow_hook {
                    hook.on_workflow_failed(workflow_id, kind, &error_msg).await;
                }
                (
                    WorkflowStatus::Failed,
                    commands,
                    None,
                    Some((error_type.to_string(), error_msg)),
                )
            }
        };

        // Finish and record execution span
        let finished_span = if let Some((error_type, error_msg)) = error_info {
            execute_span.finish_with_error(&error_type, &error_msg)
        } else {
            execute_span.finish()
        };
        span_collector.record(finished_span);

        // Call hook: workflow completed (only for actual completion, not suspension)
        if status == WorkflowStatus::Completed {
            if let Some(ref hook) = workflow_hook {
                if let Some(ref output) = output_for_hook {
                    hook.on_workflow_completed(workflow_id, kind, output).await;
                }
            }
        }

        // Convert commands to protobuf and submit
        let proto_commands: Vec<flovyn_v1::WorkflowCommand> = output_commands
            .iter()
            .map(Self::convert_command_to_proto)
            .collect();

        let proto_status = match status {
            WorkflowStatus::Running => flovyn_v1::WorkflowStatus::Running,
            WorkflowStatus::Suspended => flovyn_v1::WorkflowStatus::Suspended,
            WorkflowStatus::Completed => flovyn_v1::WorkflowStatus::Completed,
            WorkflowStatus::Failed => flovyn_v1::WorkflowStatus::Failed,
            WorkflowStatus::Cancelled => flovyn_v1::WorkflowStatus::Cancelled,
        };

        client
            .submit_workflow_commands(workflow_id, proto_commands, proto_status)
            .await?;

        // Collect spans recorded by the workflow context (e.g., run.execute spans)
        for span in ctx.take_recorded_spans() {
            span_collector.record(span);
        }

        // Flush spans to server (fire-and-forget)
        span_collector.flush(client).await;

        debug!(
            workflow_id = %workflow_id,
            status = ?status,
            "Workflow task completed"
        );

        Ok(())
    }

    /// Convert a workflow command to protobuf format
    fn convert_command_to_proto(command: &WorkflowCommand) -> flovyn_v1::WorkflowCommand {
        use flovyn_v1::workflow_command::CommandData;

        let sequence_number = command.sequence_number();
        let (command_type, command_data) = match command {
            WorkflowCommand::RecordOperation {
                operation_name,
                result,
                ..
            } => (
                flovyn_v1::CommandType::RecordOperation as i32,
                Some(CommandData::RecordOperation(
                    flovyn_v1::RecordOperationCommand {
                        operation_name: operation_name.clone(),
                        result: serde_json::to_vec(result).unwrap_or_default(),
                    },
                )),
            ),
            WorkflowCommand::SetState { key, value, .. } => (
                flovyn_v1::CommandType::SetState as i32,
                Some(CommandData::SetState(flovyn_v1::SetStateCommand {
                    key: key.clone(),
                    value: serde_json::to_vec(value).unwrap_or_default(),
                })),
            ),
            WorkflowCommand::ClearState { key, .. } => (
                flovyn_v1::CommandType::ClearState as i32,
                Some(CommandData::ClearState(flovyn_v1::ClearStateCommand {
                    key: key.clone(),
                })),
            ),
            WorkflowCommand::ScheduleTask {
                kind,
                input,
                task_execution_id,
                max_retries,
                timeout_ms,
                queue,
                priority_seconds,
                idempotency_key,
                idempotency_key_ttl_seconds,
                ..
            } => (
                flovyn_v1::CommandType::ScheduleTask as i32,
                Some(CommandData::ScheduleTask(flovyn_v1::ScheduleTaskCommand {
                    kind: kind.clone(),
                    input: serde_json::to_vec(input).unwrap_or_default(),
                    task_execution_id: task_execution_id.to_string(),
                    max_retries: max_retries.map(|v| v as i32),
                    timeout_ms: *timeout_ms,
                    queue: queue.clone(),
                    priority_seconds: *priority_seconds,
                    idempotency_key: idempotency_key.clone(),
                    idempotency_key_ttl_seconds: *idempotency_key_ttl_seconds,
                })),
            ),
            WorkflowCommand::CompleteWorkflow { output, .. } => (
                flovyn_v1::CommandType::CompleteWorkflow as i32,
                Some(CommandData::CompleteWorkflow(
                    flovyn_v1::CompleteWorkflowCommand {
                        output: serde_json::to_vec(output).unwrap_or_default(),
                    },
                )),
            ),
            WorkflowCommand::FailWorkflow {
                error,
                stack_trace,
                failure_type,
                ..
            } => (
                flovyn_v1::CommandType::FailWorkflow as i32,
                Some(CommandData::FailWorkflow(flovyn_v1::FailWorkflowCommand {
                    error: error.clone(),
                    stack_trace: stack_trace.clone(),
                    failure_type: failure_type.clone().unwrap_or_default(),
                })),
            ),
            WorkflowCommand::SuspendWorkflow { reason, .. } => (
                flovyn_v1::CommandType::SuspendWorkflow as i32,
                Some(CommandData::SuspendWorkflow(
                    flovyn_v1::SuspendWorkflowCommand {
                        reason: reason.clone(),
                    },
                )),
            ),
            WorkflowCommand::CancelWorkflow { reason, .. } => (
                flovyn_v1::CommandType::CancelWorkflow as i32,
                Some(CommandData::CancelWorkflow(
                    flovyn_v1::CancelWorkflowCommand {
                        reason: reason.clone(),
                    },
                )),
            ),
            WorkflowCommand::CreatePromise {
                promise_id,
                timeout_ms,
                idempotency_key,
                idempotency_key_ttl_seconds,
                ..
            } => (
                flovyn_v1::CommandType::CreatePromise as i32,
                Some(CommandData::CreatePromise(
                    flovyn_v1::CreatePromiseCommand {
                        promise_id: promise_id.clone(),
                        timeout_ms: *timeout_ms,
                        idempotency_key: idempotency_key.clone(),
                        idempotency_key_ttl_seconds: *idempotency_key_ttl_seconds,
                    },
                )),
            ),
            WorkflowCommand::ResolvePromise {
                promise_id, value, ..
            } => (
                flovyn_v1::CommandType::ResolvePromise as i32,
                Some(CommandData::ResolvePromise(
                    flovyn_v1::ResolvePromiseCommand {
                        promise_id: promise_id.clone(),
                        value: serde_json::to_vec(value).unwrap_or_default(),
                    },
                )),
            ),
            WorkflowCommand::ScheduleChildWorkflow {
                name,
                kind,
                definition_id,
                child_execution_id,
                input,
                queue,
                priority_seconds,
                ..
            } => (
                flovyn_v1::CommandType::ScheduleChildWorkflow as i32,
                Some(CommandData::ScheduleChildWorkflow(
                    flovyn_v1::ScheduleChildWorkflowCommand {
                        child_execution_name: name.clone(),
                        workflow_kind: kind.clone(),
                        workflow_definition_id: definition_id.map(|id| id.to_string()),
                        child_workflow_execution_id: child_execution_id.to_string(),
                        input: serde_json::to_vec(input).unwrap_or_default(),
                        queue: queue.clone(),
                        priority_seconds: *priority_seconds,
                    },
                )),
            ),
            WorkflowCommand::StartTimer {
                timer_id,
                duration_ms,
                ..
            } => (
                flovyn_v1::CommandType::StartTimer as i32,
                Some(CommandData::StartTimer(flovyn_v1::StartTimerCommand {
                    timer_id: timer_id.clone(),
                    duration_ms: *duration_ms,
                })),
            ),
            WorkflowCommand::CancelTimer { timer_id, .. } => (
                flovyn_v1::CommandType::CancelTimer as i32,
                Some(CommandData::CancelTimer(flovyn_v1::CancelTimerCommand {
                    timer_id: timer_id.clone(),
                })),
            ),
            // Cancellation request commands are handled internally but don't have
            // proto definitions yet. For now, we skip them in the command stream.
            // TODO: Add proto support for RequestCancelTask and RequestCancelChildWorkflow
            WorkflowCommand::RequestCancelTask { .. }
            | WorkflowCommand::RequestCancelChildWorkflow { .. } => {
                // Use Unspecified with empty data - server should ignore these
                (flovyn_v1::CommandType::Unspecified as i32, None)
            }
        };

        flovyn_v1::WorkflowCommand {
            sequence_number,
            command_type,
            command_data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_worker_config_default() {
        let config = WorkflowWorkerConfig::default();
        assert_eq!(config.queue, "default");
        assert_eq!(config.poll_timeout, Duration::from_secs(60));
        assert_eq!(config.no_work_backoff, Duration::from_millis(100));
        assert_eq!(config.max_concurrent, 1);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.worker_version, "1.0.0");
        assert!(config.enable_auto_registration);
        assert!(config.worker_name.is_none());
        assert!(config.team_id.is_none());
    }

    #[test]
    fn test_workflow_worker_config_custom() {
        let config = WorkflowWorkerConfig {
            worker_id: "test-worker".to_string(),
            org_id: Uuid::new_v4(),
            queue: "high-priority".to_string(),
            poll_timeout: Duration::from_secs(30),
            no_work_backoff: Duration::from_millis(100),
            max_concurrent: 10,
            heartbeat_interval: Duration::from_secs(15),
            worker_name: Some("My Worker".to_string()),
            worker_version: "2.0.0".to_string(),
            team_id: Some(Uuid::new_v4()),
            enable_auto_registration: false,
            worker_token: "test-token".to_string(),
            enable_telemetry: true,
            lifecycle_hooks: HookChain::new(),
            reconnection_strategy: ReconnectionStrategy::fixed(Duration::from_secs(5)),
            server_worker_id: None,
        };

        assert_eq!(config.worker_id, "test-worker");
        assert_eq!(config.queue, "high-priority");
        assert_eq!(config.max_concurrent, 10);
        assert_eq!(config.worker_name, Some("My Worker".to_string()));
        assert_eq!(config.worker_version, "2.0.0");
        assert!(!config.enable_auto_registration);
        assert!(config.enable_telemetry);
    }

    #[test]
    fn test_parse_event_type() {
        assert_eq!(
            WorkflowExecutorWorker::parse_event_type("WORKFLOW_STARTED"),
            EventType::WorkflowStarted
        );
        assert_eq!(
            WorkflowExecutorWorker::parse_event_type("OPERATION_COMPLETED"),
            EventType::OperationCompleted
        );
        assert_eq!(
            WorkflowExecutorWorker::parse_event_type("TASK_COMPLETED"),
            EventType::TaskCompleted
        );
        assert_eq!(
            WorkflowExecutorWorker::parse_event_type("UNKNOWN_TYPE"),
            EventType::WorkflowStarted
        );
    }

    #[test]
    fn test_convert_complete_workflow_command() {
        let command = WorkflowCommand::CompleteWorkflow {
            sequence_number: 1,
            output: serde_json::json!({"result": "success"}),
        };

        let proto = WorkflowExecutorWorker::convert_command_to_proto(&command);
        assert_eq!(proto.sequence_number, 1);
        assert_eq!(
            proto.command_type,
            flovyn_v1::CommandType::CompleteWorkflow as i32
        );
    }

    #[test]
    fn test_convert_fail_workflow_command() {
        let command = WorkflowCommand::FailWorkflow {
            sequence_number: 2,
            error: "Something went wrong".to_string(),
            stack_trace: "at line 42".to_string(),
            failure_type: Some("UserError".to_string()),
        };

        let proto = WorkflowExecutorWorker::convert_command_to_proto(&command);
        assert_eq!(proto.sequence_number, 2);
        assert_eq!(
            proto.command_type,
            flovyn_v1::CommandType::FailWorkflow as i32
        );
    }

    #[test]
    fn test_convert_schedule_task_command() {
        let command = WorkflowCommand::ScheduleTask {
            sequence_number: 3,
            kind: "send-email".to_string(),
            input: serde_json::json!({"to": "user@example.com"}),
            task_execution_id: Uuid::new_v4(),
            priority_seconds: None,
            max_retries: None,
            timeout_ms: None,
            queue: None,
            idempotency_key: None,
            idempotency_key_ttl_seconds: None,
        };

        let proto = WorkflowExecutorWorker::convert_command_to_proto(&command);
        assert_eq!(proto.sequence_number, 3);
        assert_eq!(
            proto.command_type,
            flovyn_v1::CommandType::ScheduleTask as i32
        );
    }

    #[test]
    fn test_convert_start_timer_command() {
        let command = WorkflowCommand::StartTimer {
            sequence_number: 4,
            timer_id: "timer-1".to_string(),
            duration_ms: 5000,
        };

        let proto = WorkflowExecutorWorker::convert_command_to_proto(&command);
        assert_eq!(proto.sequence_number, 4);
        assert_eq!(
            proto.command_type,
            flovyn_v1::CommandType::StartTimer as i32
        );
    }
}
