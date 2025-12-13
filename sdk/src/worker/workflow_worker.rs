//! WorkflowExecutorWorker - Polling service for workflow execution

use crate::client::hook::WorkflowHook;
use crate::client::worker_lifecycle::{WorkerLifecycleClient, WorkerType};
use crate::client::{WorkflowDispatch, WorkflowExecutionInfo};
use crate::error::{FlovynError, Result};
use crate::generated::flovyn_v1;
use crate::worker::executor::WorkflowStatus;
use crate::worker::registry::WorkflowRegistry;
use crate::workflow::command::WorkflowCommand;
use crate::workflow::context_impl::WorkflowContextImpl;
use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::recorder::CommandCollector;
use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Notify, Semaphore};
use tonic::transport::Channel;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Configuration for WorkflowExecutorWorker
#[derive(Debug, Clone)]
pub struct WorkflowWorkerConfig {
    /// Unique worker identifier
    pub worker_id: String,
    /// Tenant ID
    pub tenant_id: Uuid,
    /// Task queue to poll from
    pub task_queue: String,
    /// Long polling timeout
    pub poll_timeout: Duration,
    /// Maximum concurrent workflow executions
    pub max_concurrent: usize,
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Human-readable worker name for registration
    pub worker_name: Option<String>,
    /// Worker version for registration
    pub worker_version: String,
    /// Space ID (None = tenant-level)
    pub space_id: Option<Uuid>,
    /// Enable automatic worker registration on startup
    pub enable_auto_registration: bool,
    /// Enable notification subscription for instant work notifications
    pub enable_notifications: bool,
}

impl Default for WorkflowWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: Uuid::new_v4().to_string(),
            tenant_id: Uuid::nil(),
            task_queue: "default".to_string(),
            poll_timeout: Duration::from_secs(60),
            max_concurrent: 1,
            heartbeat_interval: Duration::from_secs(30),
            worker_name: None,
            worker_version: "1.0.0".to_string(),
            space_id: None,
            enable_auto_registration: true,
            enable_notifications: true,
        }
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
    /// Notify when work is available (from notification subscription)
    work_available_notify: Arc<Notify>,
}

impl WorkflowExecutorWorker {
    /// Create a new worker with the given configuration
    pub fn new(
        config: WorkflowWorkerConfig,
        registry: Arc<WorkflowRegistry>,
        channel: Channel,
    ) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
        Self {
            config,
            registry,
            client: WorkflowDispatch::new(channel.clone()),
            channel,
            running: Arc::new(AtomicBool::new(false)),
            semaphore,
            shutdown_tx: None,
            server_worker_id: None,
            workflow_hook: None,
            ready_notify: Arc::new(Notify::new()),
            work_available_notify: Arc::new(Notify::new()),
        }
    }

    /// Set the workflow hook for lifecycle events
    pub fn with_hook(mut self, hook: Arc<dyn WorkflowHook>) -> Self {
        self.workflow_hook = Some(hook);
        self
    }

    /// Get the ready notification handle
    pub fn ready_notify(&self) -> Arc<Notify> {
        self.ready_notify.clone()
    }

    /// Check if the worker is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Register worker with the server
    async fn register_with_server(&mut self) -> Result<()> {
        if !self.config.enable_auto_registration {
            debug!("Auto-registration disabled, skipping worker registration");
            return Ok(());
        }

        let worker_name = self
            .config
            .worker_name
            .clone()
            .unwrap_or_else(|| self.config.worker_id.clone());

        let workflows = self.registry.get_all_metadata();

        info!(
            worker_name = %worker_name,
            workflow_count = workflows.len(),
            "Registering workflow worker with server"
        );

        let mut lifecycle_client = WorkerLifecycleClient::new(self.channel.clone());

        let result = lifecycle_client
            .register_worker(
                &worker_name,
                &self.config.worker_version,
                WorkerType::Workflow,
                self.config.tenant_id,
                self.config.space_id,
                workflows,
                vec![], // No tasks for workflow worker
            )
            .await?;

        if result.success {
            self.server_worker_id = Some(result.worker_id);
            info!(
                server_worker_id = %result.worker_id,
                "Worker registered successfully"
            );
        } else {
            warn!(
                error = ?result.error,
                workflow_conflicts = result.workflow_conflicts.len(),
                "Worker registration failed or had conflicts"
            );
        }

        Ok(())
    }

    /// Run heartbeat loop
    async fn heartbeat_loop(
        channel: Channel,
        server_worker_id: Option<Uuid>,
        heartbeat_interval: Duration,
        running: Arc<AtomicBool>,
    ) {
        if server_worker_id.is_none() {
            debug!("No server worker ID, skipping heartbeat loop");
            return;
        }

        let worker_id = server_worker_id.unwrap();
        let mut client = WorkerLifecycleClient::new(channel);

        while running.load(Ordering::SeqCst) {
            tokio::time::sleep(heartbeat_interval).await;

            if !running.load(Ordering::SeqCst) {
                break;
            }

            if let Err(e) = client.send_heartbeat(worker_id).await {
                warn!("Failed to send heartbeat: {}", e);
            } else {
                debug!("Heartbeat sent successfully");
            }
        }
    }

    /// Run notification subscription loop
    async fn notification_subscription_loop(
        channel: Channel,
        config: WorkflowWorkerConfig,
        running: Arc<AtomicBool>,
        work_available_notify: Arc<Notify>,
    ) {
        info!(
            worker_id = %config.worker_id,
            "Starting notification subscription"
        );

        while running.load(Ordering::SeqCst) {
            let mut client = WorkflowDispatch::new(channel.clone());

            // Subscribe to notifications
            let stream_result = client
                .subscribe_to_notifications(
                    &config.worker_id,
                    &config.tenant_id.to_string(),
                    &config.task_queue,
                )
                .await;

            match stream_result {
                Ok(mut stream) => {
                    debug!("Notification subscription established");

                    // Process notifications until stream ends or worker stops
                    loop {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }

                        use tokio_stream::StreamExt;
                        match stream.next().await {
                            Some(Ok(event)) => {
                                debug!(
                                    tenant_id = %event.tenant_id,
                                    task_queue = %event.task_queue,
                                    "Received work available notification"
                                );
                                // Signal the polling loop to wake up immediately
                                work_available_notify.notify_one();
                            }
                            Some(Err(e)) => {
                                warn!("Notification stream error: {}", e);
                                break; // Reconnect
                            }
                            None => {
                                debug!("Notification stream ended");
                                break; // Reconnect
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to subscribe to notifications: {}", e);
                }
            }

            // Wait before reconnecting (unless shutting down)
            if running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }

        debug!("Notification subscription loop stopped");
    }

    /// Start the worker
    pub async fn start(&mut self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(FlovynError::Other("Worker already running".to_string()));
        }

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        info!(
            worker_id = %self.config.worker_id,
            tenant_id = %self.config.tenant_id,
            task_queue = %self.config.task_queue,
            "Starting workflow worker"
        );

        // Register with server
        if let Err(e) = self.register_with_server().await {
            warn!("Failed to register worker: {}", e);
            // Continue anyway - registration is best-effort
        }

        // Clone what we need for the polling loop
        let running = self.running.clone();
        let semaphore = self.semaphore.clone();
        let registry = self.registry.clone();
        let mut client = self.client.clone();
        let config = self.config.clone();
        let workflow_hook = self.workflow_hook.clone();

        // Start heartbeat loop in background
        let heartbeat_running = self.running.clone();
        let heartbeat_channel = self.channel.clone();
        let heartbeat_worker_id = self.server_worker_id;
        let heartbeat_interval = self.config.heartbeat_interval;

        tokio::spawn(async move {
            Self::heartbeat_loop(
                heartbeat_channel,
                heartbeat_worker_id,
                heartbeat_interval,
                heartbeat_running,
            )
            .await;
        });

        // Start notification subscription loop in background (if enabled)
        if self.config.enable_notifications {
            let notification_running = self.running.clone();
            let notification_channel = self.channel.clone();
            let notification_config = self.config.clone();
            let work_available_notify = self.work_available_notify.clone();

            tokio::spawn(async move {
                Self::notification_subscription_loop(
                    notification_channel,
                    notification_config,
                    notification_running,
                    work_available_notify,
                )
                .await;
            });
        }

        // Signal that worker is ready
        self.ready_notify.notify_waiters();

        // Clone work_available_notify for the polling loop
        let work_available_notify = self.work_available_notify.clone();

        // Polling loop
        while running.load(Ordering::SeqCst) {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!("Received shutdown signal");
                    break;
                }
                result = Self::poll_and_execute(
                    &mut client,
                    registry.clone(),
                    &config,
                    semaphore.clone(),
                    running.clone(),
                    workflow_hook.clone(),
                    work_available_notify.clone(),
                ) => {
                    if let Err(e) = result {
                        // Check if it's a connection error
                        let is_connection_error = e.to_string().contains("UNAVAILABLE")
                            || e.to_string().contains("Connection refused");

                        if is_connection_error {
                            warn!("Server unavailable, will retry: {}", e);
                        } else {
                            error!("Polling loop error: {}", e);
                        }

                        // Wait before retrying
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);
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
    async fn poll_and_execute(
        client: &mut WorkflowDispatch,
        registry: Arc<WorkflowRegistry>,
        config: &WorkflowWorkerConfig,
        semaphore: Arc<Semaphore>,
        running: Arc<AtomicBool>,
        workflow_hook: Option<Arc<dyn WorkflowHook>>,
        work_available_notify: Arc<Notify>,
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
                &config.tenant_id.to_string(),
                &config.task_queue,
                config.poll_timeout,
            )
            .await?;

        match workflow_info {
            Some(info) => {
                // Spawn execution in background
                let mut client = client.clone();
                let hook = workflow_hook.clone();

                tokio::spawn(async move {
                    let _permit = permit; // Keep permit alive during execution
                    let _running = running; // Keep reference

                    if let Err(e) = Self::execute_workflow(&mut client, &registry, info, hook).await
                    {
                        error!("Workflow execution error: {}", e);
                    }
                });

                Ok(())
            }
            None => {
                // No workflow available - release permit and wait
                drop(permit);

                // Wait for notification or timeout before next poll
                // This allows instant wake-up when work becomes available
                tokio::select! {
                    _ = work_available_notify.notified() => {
                        debug!("Work available notification received, polling immediately");
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        // Regular poll interval
                    }
                }
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
    ) -> Result<()> {
        let workflow_id = workflow_info.id;
        let kind = &workflow_info.kind;

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

        // Create workflow context
        let recorder = CommandCollector::new();
        let current_sequence = replay_events.len() as i32;
        let workflow_task_time = workflow_info.workflow_task_time_millis;

        let ctx = Arc::new(WorkflowContextImpl::new(
            workflow_id,
            workflow_info.tenant_id,
            workflow_info.input.clone(),
            recorder,
            replay_events.clone(),
            workflow_task_time,
        ));

        // Call hook: workflow started
        if let Some(ref hook) = workflow_hook {
            hook.on_workflow_started(workflow_id, kind, &workflow_info.input)
                .await;
        }

        // Execute workflow
        let result = registered
            .execute(ctx.clone(), workflow_info.input.clone())
            .await;

        // Get commands and determine status
        let mut commands = ctx.get_commands();
        let mut final_sequence = current_sequence;

        let (status, output_commands, output_for_hook) = match result {
            Ok(output) => {
                // Add complete command if not already present
                final_sequence += 1;
                let output_clone = output.clone();
                commands.push(WorkflowCommand::CompleteWorkflow {
                    sequence_number: final_sequence,
                    output,
                });
                (WorkflowStatus::Completed, commands, Some(output_clone))
            }
            Err(FlovynError::Suspended { reason }) => {
                final_sequence += 1;
                commands.push(WorkflowCommand::SuspendWorkflow {
                    sequence_number: final_sequence,
                    reason,
                });
                (WorkflowStatus::Suspended, commands, None)
            }
            Err(e) => {
                final_sequence += 1;
                let error_msg = e.to_string();
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
                (WorkflowStatus::Failed, commands, None)
            }
        };

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
                task_type,
                input,
                task_execution_id,
                ..
            } => (
                flovyn_v1::CommandType::ScheduleTask as i32,
                Some(CommandData::ScheduleTask(flovyn_v1::ScheduleTaskCommand {
                    task_type: task_type.clone(),
                    input: serde_json::to_vec(input).unwrap_or_default(),
                    task_execution_id: task_execution_id.to_string(),
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
                ..
            } => (
                flovyn_v1::CommandType::CreatePromise as i32,
                Some(CommandData::CreatePromise(
                    flovyn_v1::CreatePromiseCommand {
                        promise_id: promise_id.clone(),
                        timeout_ms: *timeout_ms,
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
                task_queue,
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
                        task_queue: task_queue.clone(),
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
        assert_eq!(config.task_queue, "default");
        assert_eq!(config.poll_timeout, Duration::from_secs(60));
        assert_eq!(config.max_concurrent, 1);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.worker_version, "1.0.0");
        assert!(config.enable_auto_registration);
        assert!(config.enable_notifications);
        assert!(config.worker_name.is_none());
        assert!(config.space_id.is_none());
    }

    #[test]
    fn test_workflow_worker_config_custom() {
        let config = WorkflowWorkerConfig {
            worker_id: "test-worker".to_string(),
            tenant_id: Uuid::new_v4(),
            task_queue: "high-priority".to_string(),
            poll_timeout: Duration::from_secs(30),
            max_concurrent: 10,
            heartbeat_interval: Duration::from_secs(15),
            worker_name: Some("My Worker".to_string()),
            worker_version: "2.0.0".to_string(),
            space_id: Some(Uuid::new_v4()),
            enable_auto_registration: false,
            enable_notifications: false,
        };

        assert_eq!(config.worker_id, "test-worker");
        assert_eq!(config.task_queue, "high-priority");
        assert_eq!(config.max_concurrent, 10);
        assert_eq!(config.worker_name, Some("My Worker".to_string()));
        assert_eq!(config.worker_version, "2.0.0");
        assert!(!config.enable_auto_registration);
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
            task_type: "send-email".to_string(),
            input: serde_json::json!({"to": "user@example.com"}),
            task_execution_id: Uuid::new_v4(),
            priority_seconds: None,
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
