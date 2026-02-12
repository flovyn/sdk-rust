//! Worker internals for shared state management.

use super::events::{WorkType, WorkerLifecycleEvent};
use super::hooks::HookChain;
use super::metrics::WorkerMetrics;
use super::types::{
    ConnectionInfo, RegistrationInfo, StopReason, WorkerControlError, WorkerStatus,
};
use parking_lot::RwLock;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

/// Default capacity for the event broadcast channel.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Shared internal state for a worker.
///
/// This struct manages the worker's status, metrics, connection info,
/// and event broadcasting. It is designed to be shared across async tasks
/// using `Arc<WorkerInternals>`.
pub struct WorkerInternals {
    /// Current worker status.
    status: RwLock<WorkerStatus>,

    /// Registration information (populated after successful registration).
    registration_info: RwLock<Option<RegistrationInfo>>,

    /// Connection state information.
    connection_info: RwLock<ConnectionInfo>,

    /// Runtime metrics.
    metrics: RwLock<WorkerMetrics>,

    /// Broadcast sender for lifecycle events.
    event_tx: broadcast::Sender<WorkerLifecycleEvent>,

    /// Lifecycle hooks chain.
    hooks: HookChain,

    /// When the worker was created.
    created_at: Instant,

    /// System time when the worker was created (for uptime reporting).
    started_at: SystemTime,

    /// Worker identifier.
    worker_id: String,

    /// Worker name (optional).
    worker_name: Option<String>,

    /// Pause control channel sender.
    pause_tx: watch::Sender<bool>,

    /// Pause control channel receiver (for cloning to workers).
    pause_rx: watch::Receiver<bool>,
}

impl WorkerInternals {
    /// Create a new WorkerInternals instance.
    pub fn new(worker_id: String, worker_name: Option<String>, hooks: HookChain) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let (pause_tx, pause_rx) = watch::channel(false);

        Self {
            status: RwLock::new(WorkerStatus::Initializing),
            registration_info: RwLock::new(None),
            connection_info: RwLock::new(ConnectionInfo::default()),
            metrics: RwLock::new(WorkerMetrics::new()),
            event_tx,
            hooks,
            created_at: Instant::now(),
            started_at: SystemTime::now(),
            worker_id,
            worker_name,
            pause_tx,
            pause_rx,
        }
    }

    /// Get the current worker status.
    pub fn status(&self) -> WorkerStatus {
        self.status.read().clone()
    }

    /// Get the registration info if available.
    pub fn registration_info(&self) -> Option<RegistrationInfo> {
        self.registration_info.read().clone()
    }

    /// Get the current connection info.
    pub fn connection_info(&self) -> ConnectionInfo {
        self.connection_info.read().clone()
    }

    /// Get a snapshot of current metrics.
    pub fn metrics(&self) -> WorkerMetrics {
        let mut metrics = self.metrics.read().clone();
        metrics.update_uptime(self.created_at.elapsed());
        metrics
    }

    /// Get the worker ID.
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// Get the worker name.
    pub fn worker_name(&self) -> Option<&str> {
        self.worker_name.as_deref()
    }

    /// Get the uptime duration.
    pub fn uptime(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Get the time when the worker was started.
    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }

    /// Subscribe to lifecycle events.
    ///
    /// Returns a receiver that will receive all future events.
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerLifecycleEvent> {
        self.event_tx.subscribe()
    }

    /// Check if the worker is currently paused.
    pub fn is_paused(&self) -> bool {
        *self.pause_rx.borrow()
    }

    /// Get a pause receiver clone for use in polling loops.
    ///
    /// The receiver can be used to wait for pause/resume signals.
    pub fn pause_receiver(&self) -> watch::Receiver<bool> {
        self.pause_rx.clone()
    }

    /// Pause the worker.
    ///
    /// This will:
    /// 1. Validate the worker is in a Running state
    /// 2. Call hooks to check if pause is allowed
    /// 3. Update status to Paused
    /// 4. Signal workers to pause via the pause channel
    /// 5. Emit a Paused event
    pub async fn pause(&self, reason: &str) -> Result<(), WorkerControlError> {
        // Check current status - must be Running
        let current_status = self.status();
        if !matches!(current_status, WorkerStatus::Running { .. }) {
            return Err(WorkerControlError::InvalidState(format!(
                "Expected Running, got {:?}",
                current_status
            )));
        }

        // Check with hooks if pause is allowed
        if !self.hooks.on_pause_requested(reason).await {
            return Err(WorkerControlError::RejectedByHook("pause".to_string()));
        }

        // Update status to Paused
        self.update_status(WorkerStatus::Paused {
            reason: reason.to_string(),
        })
        .await;

        // Signal workers to pause
        let _ = self.pause_tx.send(true);

        // Emit Paused event
        self.emit(WorkerLifecycleEvent::Paused {
            reason: reason.to_string(),
        })
        .await;

        Ok(())
    }

    /// Resume the worker from a paused state.
    ///
    /// This will:
    /// 1. Validate the worker is in a Paused state
    /// 2. Call hooks to check if resume is allowed
    /// 3. Update status to Running
    /// 4. Signal workers to resume via the pause channel
    /// 5. Emit a Resumed event
    pub async fn resume(&self) -> Result<(), WorkerControlError> {
        // Check current status - must be Paused
        let current_status = self.status();
        let server_worker_id = match &current_status {
            WorkerStatus::Paused { .. } => {
                // Get the server worker id from registration info
                self.registration_info().map(|r| r.worker_id)
            }
            _ => {
                return Err(WorkerControlError::InvalidState(format!(
                    "Expected Paused, got {:?}",
                    current_status
                )));
            }
        };

        // Check with hooks if resume is allowed
        if !self.hooks.on_resume_requested().await {
            return Err(WorkerControlError::RejectedByHook("resume".to_string()));
        }

        // Update status to Running
        self.update_status(WorkerStatus::Running {
            server_worker_id,
            started_at: SystemTime::now(),
        })
        .await;

        // Signal workers to resume
        let _ = self.pause_tx.send(false);

        // Emit Resumed event
        self.emit(WorkerLifecycleEvent::Resumed).await;

        Ok(())
    }

    /// Emit a lifecycle event.
    ///
    /// This will:
    /// 1. Update metrics based on event type
    /// 2. Call all registered hooks
    /// 3. Broadcast to all subscribers
    pub async fn emit(&self, event: WorkerLifecycleEvent) {
        // Update metrics based on event type
        self.update_metrics_for_event(&event);

        // Call hooks
        self.hooks.emit(event.clone()).await;

        // Broadcast to subscribers (ignore errors if no subscribers)
        let _ = self.event_tx.send(event);
    }

    /// Update the worker status.
    ///
    /// This will notify hooks of the status change.
    pub async fn update_status(&self, new_status: WorkerStatus) {
        let old_status = {
            let mut status = self.status.write();
            let old = status.clone();
            *status = new_status.clone();
            old
        };

        // Notify hooks of status change
        self.hooks.on_status_change(old_status, new_status).await;
    }

    /// Set registration info after successful registration.
    pub async fn set_registration_info(&self, info: RegistrationInfo) {
        let success = info.success;
        let has_conflicts = info.has_conflicts();
        let server_worker_id = info.worker_id;

        {
            let mut reg_info = self.registration_info.write();
            *reg_info = Some(info.clone());
        }

        // Update connection info
        {
            let mut conn_info = self.connection_info.write();
            conn_info.connected = success;
        }

        // Emit appropriate event
        if success {
            self.emit(WorkerLifecycleEvent::Registered { info }).await;

            // Update status to Running
            self.update_status(WorkerStatus::Running {
                server_worker_id: Some(server_worker_id),
                started_at: SystemTime::now(),
            })
            .await;
        } else {
            self.emit(WorkerLifecycleEvent::RegistrationFailed {
                error: if has_conflicts {
                    "Registration conflicts detected".to_string()
                } else {
                    "Registration failed".to_string()
                },
                will_retry: false,
            })
            .await;
        }
    }

    /// Record a successful heartbeat.
    pub async fn record_heartbeat_success(&self) {
        {
            let mut conn_info = self.connection_info.write();
            conn_info.last_heartbeat = Some(SystemTime::now());
            conn_info.heartbeat_failures = 0;
        }

        self.emit(WorkerLifecycleEvent::HeartbeatSent).await;
    }

    /// Record a failed heartbeat.
    pub async fn record_heartbeat_failure(&self, error: String) {
        let consecutive_failures = {
            let mut conn_info = self.connection_info.write();
            conn_info.heartbeat_failures += 1;
            conn_info.heartbeat_failures
        };

        self.emit(WorkerLifecycleEvent::HeartbeatFailed {
            error,
            consecutive_failures,
        })
        .await;
    }

    /// Record a successful poll.
    pub fn record_poll_success(&self) {
        let mut conn_info = self.connection_info.write();
        conn_info.last_poll = Some(SystemTime::now());
        conn_info.poll_failures = 0;
    }

    /// Record a failed poll.
    pub fn record_poll_failure(&self) {
        let mut conn_info = self.connection_info.write();
        conn_info.poll_failures += 1;
    }

    /// Record that work was received.
    pub async fn record_work_received(&self, work_type: WorkType, execution_id: Uuid) {
        {
            let mut metrics = self.metrics.write();
            match work_type {
                WorkType::Workflow => metrics.record_workflow_started(),
                WorkType::Task => metrics.record_task_started(),
                WorkType::Agent => metrics.record_agent_started(),
            }
        }

        self.emit(WorkerLifecycleEvent::WorkReceived {
            work_type,
            execution_id,
        })
        .await;
    }

    /// Record that work completed successfully.
    pub async fn record_work_completed(
        &self,
        work_type: WorkType,
        execution_id: Uuid,
        duration: Duration,
    ) {
        {
            let mut metrics = self.metrics.write();
            match work_type {
                WorkType::Workflow => metrics.record_workflow_completed(duration),
                WorkType::Task => metrics.record_task_completed(duration),
                WorkType::Agent => metrics.record_agent_completed(duration),
            }
        }

        self.emit(WorkerLifecycleEvent::WorkCompleted {
            work_type,
            execution_id,
            duration,
        })
        .await;
    }

    /// Record that work failed.
    pub async fn record_work_failed(
        &self,
        work_type: WorkType,
        execution_id: Uuid,
        error: String,
        duration: Duration,
    ) {
        {
            let mut metrics = self.metrics.write();
            match work_type {
                WorkType::Workflow => metrics.record_workflow_failed(duration),
                WorkType::Task => metrics.record_task_failed(duration),
                WorkType::Agent => metrics.record_agent_failed(duration),
            }
        }

        self.emit(WorkerLifecycleEvent::WorkFailed {
            work_type,
            execution_id,
            error,
        })
        .await;
    }

    /// Record disconnection.
    pub async fn record_disconnected(&self, error: String) {
        {
            let mut conn_info = self.connection_info.write();
            conn_info.connected = false;
        }

        self.emit(WorkerLifecycleEvent::Disconnected { error })
            .await;

        self.update_status(WorkerStatus::Reconnecting {
            attempts: 0,
            disconnected_at: SystemTime::now(),
            last_error: None,
        })
        .await;
    }

    /// Record reconnection attempt.
    pub async fn record_reconnecting(&self, attempt: u32, last_error: Option<String>) {
        {
            let mut conn_info = self.connection_info.write();
            conn_info.reconnect_attempt = Some(attempt);
        }

        // Update status
        {
            let mut status = self.status.write();
            if let WorkerStatus::Reconnecting {
                disconnected_at, ..
            } = &*status
            {
                *status = WorkerStatus::Reconnecting {
                    attempts: attempt,
                    disconnected_at: *disconnected_at,
                    last_error,
                };
            }
        }

        self.emit(WorkerLifecycleEvent::Reconnecting { attempt })
            .await;
    }

    /// Record successful reconnection.
    pub async fn record_reconnected(&self, server_worker_id: Option<Uuid>) {
        {
            let mut conn_info = self.connection_info.write();
            conn_info.connected = true;
            conn_info.reconnect_attempt = None;
        }

        self.emit(WorkerLifecycleEvent::Reconnected).await;

        self.update_status(WorkerStatus::Running {
            server_worker_id,
            started_at: SystemTime::now(),
        })
        .await;
    }

    /// Record that the worker is ready (polling started).
    pub async fn record_ready(&self, server_worker_id: Option<Uuid>) {
        self.emit(WorkerLifecycleEvent::Ready { server_worker_id })
            .await;
    }

    /// Record worker pause.
    pub async fn record_paused(&self, reason: String) {
        self.update_status(WorkerStatus::Paused {
            reason: reason.clone(),
        })
        .await;

        self.emit(WorkerLifecycleEvent::Paused { reason }).await;
    }

    /// Record worker resume.
    pub async fn record_resumed(&self, server_worker_id: Option<Uuid>) {
        self.update_status(WorkerStatus::Running {
            server_worker_id,
            started_at: SystemTime::now(),
        })
        .await;

        self.emit(WorkerLifecycleEvent::Resumed).await;
    }

    /// Record shutdown request.
    pub async fn record_shutting_down(&self, graceful: bool, in_flight_count: usize) {
        self.update_status(WorkerStatus::ShuttingDown {
            requested_at: SystemTime::now(),
            in_flight_count,
        })
        .await;

        self.emit(WorkerLifecycleEvent::ShuttingDown { graceful })
            .await;

        // Call before_stop hooks
        self.hooks.before_stop(graceful).await;
    }

    /// Record worker stopped.
    pub async fn record_stopped(&self, reason: StopReason) {
        let uptime = self.created_at.elapsed();

        self.update_status(WorkerStatus::Stopped {
            stopped_at: SystemTime::now(),
            reason: reason.clone(),
        })
        .await;

        self.emit(WorkerLifecycleEvent::Stopped { reason, uptime })
            .await;
    }

    /// Record worker starting.
    pub async fn record_starting(&self) {
        self.emit(WorkerLifecycleEvent::Starting {
            worker_id: self.worker_id.clone(),
            worker_name: self.worker_name.clone(),
        })
        .await;

        self.update_status(WorkerStatus::Registering).await;
    }

    /// Check if hooks allow starting.
    pub async fn hooks_allow_start(&self) -> bool {
        self.hooks.before_start().await
    }

    /// Check if hooks allow pausing.
    pub async fn hooks_allow_pause(&self, reason: &str) -> bool {
        self.hooks.on_pause_requested(reason).await
    }

    /// Check if hooks allow resuming.
    pub async fn hooks_allow_resume(&self) -> bool {
        self.hooks.on_resume_requested().await
    }

    /// Update metrics based on event type.
    fn update_metrics_for_event(&self, event: &WorkerLifecycleEvent) {
        // Most metric updates are handled by the specific record_* methods
        // This is for any additional event-based metrics updates
        if let WorkerLifecycleEvent::HeartbeatFailed { .. } = event {
            // Heartbeat failures are tracked in connection_info
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_worker_internals_new() {
        let internals = WorkerInternals::new(
            "test-worker".to_string(),
            Some("Test Worker".to_string()),
            HookChain::new(),
        );

        assert_eq!(internals.worker_id(), "test-worker");
        assert_eq!(internals.worker_name(), Some("Test Worker"));
        assert_eq!(internals.status(), WorkerStatus::Initializing);
        assert!(internals.registration_info().is_none());
    }

    #[tokio::test]
    async fn test_status_update() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        assert_eq!(internals.status(), WorkerStatus::Initializing);

        internals.update_status(WorkerStatus::Registering).await;
        assert_eq!(internals.status(), WorkerStatus::Registering);

        internals
            .update_status(WorkerStatus::Running {
                server_worker_id: None,
                started_at: SystemTime::now(),
            })
            .await;

        match internals.status() {
            WorkerStatus::Running { .. } => {}
            _ => panic!("Expected Running status"),
        }
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        let mut rx = internals.subscribe();

        internals.emit(WorkerLifecycleEvent::HeartbeatSent).await;

        let event = rx.recv().await.unwrap();
        assert_eq!(event.event_name(), "heartbeat_sent");
    }

    #[tokio::test]
    async fn test_heartbeat_tracking() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        // Record successful heartbeat
        internals.record_heartbeat_success().await;
        let conn_info = internals.connection_info();
        assert!(conn_info.last_heartbeat.is_some());
        assert_eq!(conn_info.heartbeat_failures, 0);

        // Record failed heartbeat
        internals
            .record_heartbeat_failure("timeout".to_string())
            .await;
        let conn_info = internals.connection_info();
        assert_eq!(conn_info.heartbeat_failures, 1);

        // Record another failure
        internals
            .record_heartbeat_failure("connection reset".to_string())
            .await;
        let conn_info = internals.connection_info();
        assert_eq!(conn_info.heartbeat_failures, 2);

        // Success resets the counter
        internals.record_heartbeat_success().await;
        let conn_info = internals.connection_info();
        assert_eq!(conn_info.heartbeat_failures, 0);
    }

    #[tokio::test]
    async fn test_work_metrics() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        let exec_id = Uuid::new_v4();

        // Record workflow work
        internals
            .record_work_received(WorkType::Workflow, exec_id)
            .await;
        let metrics = internals.metrics();
        assert_eq!(metrics.workflows_in_progress, 1);

        internals
            .record_work_completed(WorkType::Workflow, exec_id, Duration::from_millis(100))
            .await;
        let metrics = internals.metrics();
        assert_eq!(metrics.workflows_in_progress, 0);
        assert_eq!(metrics.workflows_executed, 1);

        // Record task work
        let task_id = Uuid::new_v4();
        internals
            .record_work_received(WorkType::Task, task_id)
            .await;
        internals
            .record_work_failed(
                WorkType::Task,
                task_id,
                "error".to_string(),
                Duration::from_millis(50),
            )
            .await;

        let metrics = internals.metrics();
        assert_eq!(metrics.tasks_executed, 1);
        assert_eq!(metrics.task_errors, 1);
    }

    #[tokio::test]
    async fn test_reconnection_flow() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        // Simulate connection loss
        internals
            .record_disconnected("connection lost".to_string())
            .await;

        match internals.status() {
            WorkerStatus::Reconnecting { attempts, .. } => {
                assert_eq!(attempts, 0);
            }
            _ => panic!("Expected Reconnecting status"),
        }

        // Reconnection attempts
        internals.record_reconnecting(1, None).await;
        match internals.status() {
            WorkerStatus::Reconnecting { attempts, .. } => {
                assert_eq!(attempts, 1);
            }
            _ => panic!("Expected Reconnecting status"),
        }

        // Successful reconnection
        internals.record_reconnected(Some(Uuid::new_v4())).await;
        match internals.status() {
            WorkerStatus::Running { .. } => {}
            _ => panic!("Expected Running status"),
        }
    }

    #[tokio::test]
    async fn test_shutdown_flow() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        internals.record_shutting_down(true, 2).await;

        match internals.status() {
            WorkerStatus::ShuttingDown {
                in_flight_count, ..
            } => {
                assert_eq!(in_flight_count, 2);
            }
            _ => panic!("Expected ShuttingDown status"),
        }

        internals.record_stopped(StopReason::Graceful).await;

        match internals.status() {
            WorkerStatus::Stopped { reason, .. } => {
                assert_eq!(reason, StopReason::Graceful);
            }
            _ => panic!("Expected Stopped status"),
        }
    }

    #[tokio::test]
    async fn test_hooks_are_called() {
        use super::super::hooks::WorkerLifecycleHook;
        use async_trait::async_trait;

        struct CountingHook {
            event_count: AtomicU32,
        }

        #[async_trait]
        impl WorkerLifecycleHook for CountingHook {
            async fn on_event(&self, _event: WorkerLifecycleEvent) {
                self.event_count.fetch_add(1, Ordering::SeqCst);
            }
        }

        let hook = Arc::new(CountingHook {
            event_count: AtomicU32::new(0),
        });

        let mut chain = HookChain::new();
        chain.add_arc(hook.clone());

        let internals = WorkerInternals::new("test".to_string(), None, chain);

        internals.emit(WorkerLifecycleEvent::HeartbeatSent).await;
        internals.emit(WorkerLifecycleEvent::Reconnected).await;

        assert_eq!(hook.event_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_uptime_tracking() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        // Small delay to ensure uptime is measurable
        tokio::time::sleep(Duration::from_millis(10)).await;

        let uptime = internals.uptime();
        assert!(uptime >= Duration::from_millis(10));

        let metrics = internals.metrics();
        assert!(metrics.uptime >= Duration::from_millis(10));
    }

    #[test]
    fn test_poll_tracking() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        internals.record_poll_success();
        let conn_info = internals.connection_info();
        assert!(conn_info.last_poll.is_some());
        assert_eq!(conn_info.poll_failures, 0);

        internals.record_poll_failure();
        internals.record_poll_failure();
        let conn_info = internals.connection_info();
        assert_eq!(conn_info.poll_failures, 2);

        internals.record_poll_success();
        let conn_info = internals.connection_info();
        assert_eq!(conn_info.poll_failures, 0);
    }

    #[tokio::test]
    async fn test_pause_resume() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        // Must be in Running state to pause - update status directly
        internals
            .update_status(WorkerStatus::Running {
                server_worker_id: Some(Uuid::new_v4()),
                started_at: SystemTime::now(),
            })
            .await;

        assert!(!internals.is_paused());

        // Pause the worker
        let result = internals.pause("maintenance").await;
        assert!(result.is_ok(), "pause failed: {:?}", result);
        assert!(internals.is_paused());

        match internals.status() {
            WorkerStatus::Paused { reason } => {
                assert_eq!(reason, "maintenance");
            }
            _ => panic!("Expected Paused status"),
        }

        // Resume the worker
        let result = internals.resume().await;
        assert!(result.is_ok());
        assert!(!internals.is_paused());

        match internals.status() {
            WorkerStatus::Running { .. } => {}
            _ => panic!("Expected Running status"),
        }
    }

    #[tokio::test]
    async fn test_pause_invalid_state() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        // Cannot pause when not running (initial state is Initializing)
        let result = internals.pause("test").await;
        assert!(result.is_err());

        match result {
            Err(WorkerControlError::InvalidState(_)) => {}
            _ => panic!("Expected InvalidState error"),
        }
    }

    #[tokio::test]
    async fn test_resume_invalid_state() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        // Cannot resume when not paused
        let result = internals.resume().await;
        assert!(result.is_err());

        match result {
            Err(WorkerControlError::InvalidState(_)) => {}
            _ => panic!("Expected InvalidState error"),
        }
    }

    #[test]
    fn test_pause_receiver() {
        let internals = WorkerInternals::new("test".to_string(), None, HookChain::new());

        // Get pause receiver and verify initial state
        let rx = internals.pause_receiver();
        assert!(!*rx.borrow());
    }
}
