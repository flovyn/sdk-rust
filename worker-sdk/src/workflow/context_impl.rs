//! WorkflowContextImpl - Concrete implementation of WorkflowContext

use crate::error::{DeterminismViolationError, FlovynError, Result};
use crate::workflow::command::WorkflowCommand;
use crate::workflow::context::{
    DeterministicRandom, PromiseOptions, ScheduleTaskOptions, WorkflowContext,
};
use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::future::{
    ChildWorkflowFuture, ChildWorkflowFutureContext, ChildWorkflowFutureRaw, OperationFuture,
    OperationFutureRaw, PromiseFuture, PromiseFutureContext, PromiseFutureRaw, SuspensionContext,
    TaskFuture, TaskFutureContext, TaskFutureRaw, TimerFuture, TimerFutureContext,
};
use crate::workflow::recorder::CommandRecorder;
use async_trait::async_trait;
use flovyn_worker_core::generated::flovyn_v1::ExecutionSpan;
use flovyn_worker_core::workflow::execution::SeededRandom;
use flovyn_worker_core::workflow::ReplayEngine;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::time::Duration;
use uuid::Uuid;

/// Concrete implementation of WorkflowContext
pub struct WorkflowContextImpl<R: CommandRecorder> {
    /// Unique workflow execution ID
    workflow_execution_id: Uuid,

    /// Org ID
    org_id: Uuid,

    /// Workflow input
    input: Value,

    /// Command recorder for determinism validation
    recorder: RwLock<R>,

    /// Replay engine for event pre-filtering, sequence management, and terminal event lookup
    replay_engine: ReplayEngine,

    /// Current sequence number (1-indexed)
    sequence_number: AtomicI32,

    /// Deterministic time (milliseconds since epoch)
    current_time: AtomicI64,

    /// UUID counter for deterministic UUID generation
    uuid_counter: AtomicI64,

    /// Seeded random number generator
    random: SeededRandom,

    /// Whether cancellation has been requested
    cancellation_requested: AtomicBool,

    /// Pending tasks that need to be resolved
    pending_tasks: RwLock<HashMap<String, Value>>,

    /// Pending promises that need to be resolved
    pending_promises: RwLock<HashMap<String, Value>>,

    /// Pending timers
    pending_timers: RwLock<HashMap<String, i64>>,

    /// Pending child workflows
    pending_child_workflows: RwLock<HashMap<String, Value>>,

    /// Counter for deterministic sleep/timer ID generation.
    /// Separate from sequence_number because sequence depends on existingEvents.size
    /// which changes between executions. This counter ensures timer IDs are consistent.
    sleep_call_counter: AtomicI32,

    /// Whether telemetry is enabled for span recording
    #[allow(dead_code)]
    enable_telemetry: bool,

    /// Recorded execution spans (collected by worker after execution)
    recorded_spans: RwLock<Vec<ExecutionSpan>>,

    /// Shared suspension cell for this workflow execution.
    /// Used to signal suspension without relying on thread-local storage,
    /// which is not safe when multiple workflows execute concurrently on the same thread.
    /// This Arc is shared with all futures created by this context.
    suspension_cell: SuspensionCell,
}

/// Shared suspension cell that can be passed to futures.
/// This allows futures to signal suspension to the workflow context
/// without needing a direct reference to the context.
#[derive(Clone)]
pub struct SuspensionCell(std::sync::Arc<parking_lot::Mutex<Option<String>>>);

impl SuspensionCell {
    /// Create a new suspension cell.
    pub fn new() -> Self {
        Self(std::sync::Arc::new(parking_lot::Mutex::new(None)))
    }

    /// Signal suspension with the given reason.
    pub fn signal(&self, reason: String) {
        tracing::trace!(reason = %reason, "Signaling suspension via cell");
        *self.0.lock() = Some(reason);
    }

    /// Take the suspension reason if set.
    pub fn take(&self) -> Option<String> {
        self.0.lock().take()
    }

    /// Clear any pending suspension reason.
    pub fn clear(&self) {
        *self.0.lock() = None;
    }
}

impl Default for SuspensionCell {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: CommandRecorder> WorkflowContextImpl<R> {
    /// Create a new WorkflowContextImpl
    pub fn new(
        workflow_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        recorder: R,
        existing_events: Vec<ReplayEvent>,
        start_time_millis: i64,
    ) -> Self {
        Self::new_with_telemetry(
            workflow_execution_id,
            org_id,
            input,
            recorder,
            existing_events,
            start_time_millis,
            false, // telemetry disabled by default
        )
    }

    /// Create a new WorkflowContextImpl with telemetry option
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_telemetry(
        workflow_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        recorder: R,
        existing_events: Vec<ReplayEvent>,
        start_time_millis: i64,
        enable_telemetry: bool,
    ) -> Self {
        // Create seed from workflow execution ID for deterministic randomness
        let seed = workflow_execution_id.as_u128() as u64;

        // Start sequence number from the next available position
        // If there are existing events (replay), continue from after them
        let initial_sequence = (existing_events.len() as i32) + 1;

        // Create ReplayEngine which handles:
        // - Event pre-filtering by type
        // - Per-type sequence counters
        // - Terminal event lookup
        // - State and operation cache management
        let replay_engine = ReplayEngine::new(existing_events);

        Self {
            workflow_execution_id,
            org_id,
            input,
            recorder: RwLock::new(recorder),
            replay_engine,
            sequence_number: AtomicI32::new(initial_sequence),
            current_time: AtomicI64::new(start_time_millis),
            uuid_counter: AtomicI64::new(0),
            random: SeededRandom::new(seed),
            cancellation_requested: AtomicBool::new(false),
            pending_tasks: RwLock::new(HashMap::new()),
            pending_promises: RwLock::new(HashMap::new()),
            pending_timers: RwLock::new(HashMap::new()),
            pending_child_workflows: RwLock::new(HashMap::new()),
            sleep_call_counter: AtomicI32::new(0),
            enable_telemetry,
            recorded_spans: RwLock::new(Vec::new()),
            suspension_cell: SuspensionCell::new(),
        }
    }

    /// Get the next sequence number and increment
    fn next_sequence(&self) -> i32 {
        self.sequence_number.fetch_add(1, Ordering::SeqCst)
    }

    /// Record a command via the recorder
    fn record_command(&self, command: WorkflowCommand) -> Result<()> {
        let mut recorder = self.recorder.write();
        recorder
            .record_command(command)
            .map_err(FlovynError::DeterminismViolation)
    }

    // Terminal event lookup methods are now provided by ReplayEngine:
    // - replay_engine.find_terminal_task_event()
    // - replay_engine.find_terminal_timer_event()
    // - replay_engine.find_terminal_promise_event()
    // - replay_engine.find_terminal_child_workflow_event()

    /// Request cancellation
    pub fn request_cancellation(&self) {
        self.cancellation_requested.store(true, Ordering::SeqCst);
    }

    /// Get all recorded commands
    pub fn get_commands(&self) -> Vec<WorkflowCommand> {
        self.recorder.read().get_commands()
    }

    /// Take all recorded commands (clears the recorder)
    pub fn take_commands(&self) -> Vec<WorkflowCommand> {
        self.recorder.write().take_commands()
    }

    /// Resolve a pending task with a result
    pub fn resolve_task(&self, task_type: &str, result: Value) {
        let mut pending = self.pending_tasks.write();
        pending.insert(task_type.to_string(), result);
    }

    /// Resolve a pending promise with a value
    pub fn resolve_promise(&self, name: &str, value: Value) {
        let mut pending = self.pending_promises.write();
        pending.insert(name.to_string(), value);
    }

    /// Fire a timer
    pub fn fire_timer(&self, timer_id: &str) {
        let mut pending = self.pending_timers.write();
        pending.remove(timer_id);
    }

    /// Resolve a child workflow
    pub fn resolve_child_workflow(&self, name: &str, result: Value) {
        let mut pending = self.pending_child_workflows.write();
        pending.insert(name.to_string(), result);
    }

    /// Take all recorded execution spans (clears the internal buffer).
    /// Called by workflow worker to collect spans after execution.
    pub fn take_recorded_spans(&self) -> Vec<ExecutionSpan> {
        std::mem::take(&mut self.recorded_spans.write())
    }

    /// Signal workflow suspension with the given reason.
    ///
    /// This is called by futures when they need to suspend (e.g., waiting for a task).
    /// Unlike thread-local suspension, this is scoped to the workflow context and is
    /// safe when multiple workflows execute concurrently on the same thread.
    pub fn signal_suspension(&self, reason: String) {
        self.suspension_cell.signal(reason);
    }

    /// Take the suspension reason if set.
    ///
    /// Called by the executor after polling a workflow future that returned `Pending`.
    /// Returns `Some(reason)` if the workflow is suspended, `None` otherwise.
    pub fn take_suspension(&self) -> Option<String> {
        self.suspension_cell.take()
    }

    /// Clear any pending suspension reason.
    ///
    /// Called before polling a workflow to ensure clean state.
    pub fn clear_suspension(&self) {
        self.suspension_cell.clear();
    }

    /// Get a clone of the suspension cell to pass to futures.
    ///
    /// Futures use this cell to signal suspension back to the workflow context.
    pub fn suspension_cell(&self) -> SuspensionCell {
        self.suspension_cell.clone()
    }
}

// Implement SuspensionContext for WorkflowContextImpl
impl<R: CommandRecorder + Send + Sync> SuspensionContext for WorkflowContextImpl<R> {
    fn signal_suspension(&self, reason: String) {
        // Delegate to the public method
        WorkflowContextImpl::signal_suspension(self, reason);
    }
}

#[async_trait]
impl<R: CommandRecorder + Send + Sync> WorkflowContext for WorkflowContextImpl<R> {
    fn workflow_execution_id(&self) -> Uuid {
        self.workflow_execution_id
    }

    fn org_id(&self) -> Uuid {
        self.org_id
    }

    fn input_raw(&self) -> &Value {
        &self.input
    }

    fn current_time_millis(&self) -> i64 {
        self.current_time.load(Ordering::SeqCst)
    }

    fn random_uuid(&self) -> Uuid {
        // Generate deterministic UUID using UUID v5 with workflow execution ID as namespace
        let counter = self.uuid_counter.fetch_add(1, Ordering::SeqCst);
        let name = format!("{}:{}", self.workflow_execution_id, counter);
        Uuid::new_v5(&self.workflow_execution_id, name.as_bytes())
    }

    fn random(&self) -> &dyn DeterministicRandom {
        &self.random
    }

    // All scheduling methods (schedule_raw, sleep, promise_raw, schedule_workflow_raw, run_raw)
    // are implemented below as Future-returning methods for parallel execution support.

    async fn get_raw(&self, key: &str) -> Result<Option<Value>> {
        Ok(self.replay_engine.get_state(key))
    }

    async fn set_raw(&self, key: &str, value: Value) -> Result<()> {
        // Get per-type sequence and increment atomically.
        // This assigns State(0), State(1), etc. to each set()/clear() call.
        let state_seq = self.replay_engine.next_state_seq();

        // Look for event at this per-type index
        if let Some(state_event) = self.replay_engine.get_state_event(state_seq) {
            // Validate event type is StateSet
            if state_event.event_type() != EventType::StateSet {
                panic!(
                    "Determinism violation: {}",
                    DeterminismViolationError::TypeMismatch {
                        sequence: state_seq as i32,
                        expected: EventType::StateSet,
                        actual: state_event.event_type(),
                    }
                );
            }

            // Validate key matches
            let event_key = state_event
                .get_string("key")
                .unwrap_or_default()
                .to_string();

            if event_key != key {
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::StateKeyMismatch {
                        sequence: state_seq as i32,
                        expected: event_key,
                        actual: key.to_string(),
                    },
                ));
            }

            // State was already set - update replay engine state
            self.replay_engine.set_state(key, value);
            return Ok(());
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::SetState {
            sequence_number: sequence,
            key: key.to_string(),
            value: value.clone(),
        })?;

        // Update state in replay engine
        self.replay_engine.set_state(key, value);
        Ok(())
    }

    async fn clear(&self, key: &str) -> Result<()> {
        // Get per-type sequence and increment atomically.
        // This assigns State(0), State(1), etc. to each set()/clear() call.
        let state_seq = self.replay_engine.next_state_seq();

        // Look for event at this per-type index
        if let Some(state_event) = self.replay_engine.get_state_event(state_seq) {
            // Validate event type is StateCleared
            if state_event.event_type() != EventType::StateCleared {
                panic!(
                    "Determinism violation: {}",
                    DeterminismViolationError::TypeMismatch {
                        sequence: state_seq as i32,
                        expected: EventType::StateCleared,
                        actual: state_event.event_type(),
                    }
                );
            }

            // Validate key matches
            let event_key = state_event
                .get_string("key")
                .unwrap_or_default()
                .to_string();

            if event_key != key {
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::StateKeyMismatch {
                        sequence: state_seq as i32,
                        expected: event_key,
                        actual: key.to_string(),
                    },
                ));
            }

            // State was already cleared - update replay engine state
            self.replay_engine.clear_state(key);
            return Ok(());
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::ClearState {
            sequence_number: sequence,
            key: key.to_string(),
        })?;

        // Update state in replay engine
        self.replay_engine.clear_state(key);
        Ok(())
    }

    async fn clear_all(&self) -> Result<()> {
        // Clear all keys by clearing each one
        let keys = self.replay_engine.state_keys();

        for key in keys {
            self.clear(&key).await?;
        }
        Ok(())
    }

    async fn state_keys(&self) -> Result<Vec<String>> {
        Ok(self.replay_engine.state_keys())
    }

    fn is_cancellation_requested(&self) -> bool {
        self.cancellation_requested.load(Ordering::SeqCst)
    }

    async fn check_cancellation(&self) -> Result<()> {
        if self.is_cancellation_requested() {
            Err(FlovynError::WorkflowCancelled(
                "Cancellation requested".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    // =========================================================================
    // Future-Returning Methods for Parallel Execution
    // =========================================================================

    fn schedule_raw(&self, task_type: &str, input: Value) -> TaskFutureRaw {
        self.schedule_with_options_raw(task_type, input, ScheduleTaskOptions::default())
    }

    fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> TaskFutureRaw {
        // Get per-type sequence and increment atomically.
        let task_seq = self.replay_engine.next_task_seq();

        // CRITICAL: Always generate UUID to keep counter synchronized across replay.
        // This ensures deterministic UUID generation even when extending a replay.
        // See: Issue 1 in synchronous-scheduling-implementation.md
        let generated_id = self.random_uuid();

        // Look for event at this per-type index (replay case)
        if let Some(scheduled_event) = self.replay_engine.get_task_event(task_seq) {
            // Validate task kind matches
            let event_kind = scheduled_event
                .get_string("kind")
                .unwrap_or_default()
                .to_string();

            if event_kind != task_type {
                return TaskFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::TaskTypeMismatch {
                        sequence: task_seq as i32,
                        expected: event_kind,
                        actual: task_type.to_string(),
                    },
                ));
            }

            // Get task execution ID from the scheduled event (ignore generated_id)
            let task_execution_id = scheduled_event
                .get_string("taskExecutionId")
                .or_else(|| scheduled_event.get_string("taskId"))
                .map(|s| Uuid::parse_str(s).unwrap_or(Uuid::nil()))
                .unwrap_or(Uuid::nil());

            // Look for terminal event (completed/failed) for this task
            if let Some(terminal_event) = self
                .replay_engine
                .find_terminal_task_event(&task_execution_id.to_string())
            {
                if terminal_event.event_type() == EventType::TaskCompleted {
                    let result = terminal_event
                        .get("result")
                        .or_else(|| terminal_event.get("output"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    return TaskFuture::from_replay_with_cell(
                        task_seq,
                        task_execution_id,
                        self.suspension_cell(),
                        Ok(result),
                    );
                } else {
                    let error = terminal_event
                        .get_string("error")
                        .unwrap_or("Task failed")
                        .to_string();
                    return TaskFuture::from_replay_with_cell(
                        task_seq,
                        task_execution_id,
                        self.suspension_cell(),
                        Err(FlovynError::TaskFailed(error)),
                    );
                }
            }

            // Task scheduled but not completed yet - return pending future
            return TaskFuture::new_with_cell(task_seq, task_execution_id, self.suspension_cell());
        }

        // No event at this per-type index → new command with client-generated ID
        let task_execution_id = generated_id;

        // Record the command with the client-generated task_execution_id
        let sequence = self.next_sequence();
        if let Err(e) = self.record_command(WorkflowCommand::ScheduleTask {
            sequence_number: sequence,
            kind: task_type.to_string(),
            task_execution_id,
            input,
            priority_seconds: options.priority_seconds,
            max_retries: options.max_retries,
            timeout_ms: options.timeout.map(|d| d.as_millis() as i64),
            queue: options.queue.clone(),
            idempotency_key: options.idempotency_key.clone(),
            idempotency_key_ttl_seconds: options.idempotency_key_ttl_seconds,
        }) {
            return TaskFuture::with_error(e);
        }

        // Return pending future - when polled, it will return Err(Suspended)
        // The workflow will suspend and be replayed when the task completes
        TaskFuture::new_with_cell(task_seq, task_execution_id, self.suspension_cell())
    }

    fn sleep(&self, duration: Duration) -> TimerFuture {
        // Get per-type sequence and increment atomically.
        let timer_seq = self.replay_engine.next_timer_seq();

        // Generate deterministic timer ID
        let sleep_count = self.sleep_call_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let timer_id = format!("sleep-{}", sleep_count);
        let duration_ms = duration.as_millis() as i64;

        // Look for event at this per-type index (replay case)
        if let Some(started_event) = self.replay_engine.get_timer_event(timer_seq) {
            // Validate timer ID matches
            let event_timer_id = started_event
                .get_string("timerId")
                .unwrap_or_default()
                .to_string();

            if event_timer_id != timer_id {
                return TimerFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::TimerIdMismatch {
                        sequence: timer_seq as i32,
                        expected: event_timer_id,
                        actual: timer_id,
                    },
                ));
            }

            // Look for terminal event (fired/cancelled) for this timer
            if let Some(terminal_event) = self
                .replay_engine
                .find_terminal_timer_event(&event_timer_id)
            {
                if terminal_event.event_type() == EventType::TimerFired {
                    return TimerFuture::from_replay_with_cell(
                        timer_seq,
                        event_timer_id,
                        self.suspension_cell(),
                        true,
                    );
                } else {
                    return TimerFuture::from_replay_with_cell(
                        timer_seq,
                        event_timer_id,
                        self.suspension_cell(),
                        false,
                    );
                }
            }

            // Timer started but not fired yet - return pending future
            return TimerFuture::new_with_cell(timer_seq, event_timer_id, self.suspension_cell());
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        if let Err(e) = self.record_command(WorkflowCommand::StartTimer {
            sequence_number: sequence,
            timer_id: timer_id.clone(),
            duration_ms,
        }) {
            return TimerFuture::with_error(e);
        }

        // Return pending future
        TimerFuture::new_with_cell(timer_seq, timer_id, self.suspension_cell())
    }

    fn schedule_workflow_raw(
        &self,
        name: &str,
        kind: &str,
        input: Value,
    ) -> ChildWorkflowFutureRaw {
        // Get per-type sequence and increment atomically.
        let cw_seq = self.replay_engine.next_child_workflow_seq();

        // Look for event at this per-type index (replay case)
        if let Some(initiated_event) = self.replay_engine.get_child_workflow_event(cw_seq) {
            // Validate child workflow name matches
            let event_name = initiated_event
                .get_string("childExecutionName")
                .unwrap_or_default()
                .to_string();

            if event_name != name {
                return ChildWorkflowFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::ChildWorkflowMismatch {
                        sequence: cw_seq as i32,
                        field: "name".to_string(),
                        expected: event_name,
                        actual: name.to_string(),
                    },
                ));
            }

            // Validate child workflow kind matches
            let event_kind = initiated_event
                .get_string("childWorkflowKind")
                .unwrap_or_default()
                .to_string();

            if !event_kind.is_empty() && event_kind != kind {
                return ChildWorkflowFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::ChildWorkflowMismatch {
                        sequence: cw_seq as i32,
                        field: "kind".to_string(),
                        expected: event_kind,
                        actual: kind.to_string(),
                    },
                ));
            }

            // Get child execution ID
            let child_execution_id = initiated_event
                .get_string("childExecutionId")
                .map(|s| Uuid::parse_str(s).unwrap_or(Uuid::nil()))
                .unwrap_or(Uuid::nil());

            // IMPORTANT: Increment uuid_counter to stay in sync with UUIDs generated
            // in the original execution. Without this, subsequent schedule_workflow_raw
            // calls after replay would generate the same UUID as the first child.
            // See bug report: .dev/docs/bugs/20260107_uuid_counter_not_incremented_during_replay.md
            let _ = self.uuid_counter.fetch_add(1, Ordering::SeqCst);

            // Look for terminal event (completed/failed) for this child workflow
            if let Some(terminal_event) = self
                .replay_engine
                .find_terminal_child_workflow_event(&event_name)
            {
                if terminal_event.event_type() == EventType::ChildWorkflowCompleted {
                    let result = terminal_event.get("output").cloned().unwrap_or(Value::Null);
                    return ChildWorkflowFuture::from_replay_with_cell(
                        cw_seq,
                        child_execution_id,
                        event_name,
                        self.suspension_cell.clone(),
                        Ok(result),
                    );
                } else {
                    let error = terminal_event
                        .get_string("error")
                        .unwrap_or("Child workflow failed")
                        .to_string();
                    let exec_id = terminal_event
                        .get_string("childworkflowExecutionId")
                        .unwrap_or("unknown")
                        .to_string();
                    return ChildWorkflowFuture::from_replay_with_cell(
                        cw_seq,
                        child_execution_id,
                        name.to_string(),
                        self.suspension_cell.clone(),
                        Err(FlovynError::ChildWorkflowFailed {
                            execution_id: exec_id,
                            name: name.to_string(),
                            error,
                        }),
                    );
                }
            }

            // Child workflow initiated but not completed yet - return pending future
            return ChildWorkflowFuture::new_with_cell(
                cw_seq,
                child_execution_id,
                event_name,
                self.suspension_cell.clone(),
            );
        }

        // No event at this per-type index → new command
        let child_execution_id = self.random_uuid();
        let sequence = self.next_sequence();
        if let Err(e) = self.record_command(WorkflowCommand::ScheduleChildWorkflow {
            sequence_number: sequence,
            name: name.to_string(),
            kind: Some(kind.to_string()),
            definition_id: None,
            child_execution_id,
            input,
            queue: String::new(),
            priority_seconds: 0,
        }) {
            return ChildWorkflowFuture::with_error(e);
        }

        // Return pending future
        ChildWorkflowFuture::new_with_cell(
            cw_seq,
            child_execution_id,
            name.to_string(),
            self.suspension_cell.clone(),
        )
    }

    fn promise_raw(&self, name: &str) -> PromiseFutureRaw {
        // Get per-type sequence and increment atomically.
        let promise_seq = self.replay_engine.next_promise_seq();

        // Look for event at this per-type index (replay case)
        if let Some(created_event) = self.replay_engine.get_promise_event(promise_seq) {
            // Validate promise name matches
            let event_promise_name = created_event
                .get_string("promiseName")
                .unwrap_or_default()
                .to_string();

            if event_promise_name != name {
                return PromiseFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::PromiseNameMismatch {
                        sequence: promise_seq as i32,
                        expected: event_promise_name,
                        actual: name.to_string(),
                    },
                ));
            }

            // Get promiseId (UUID) for terminal event lookup
            let promise_id = created_event
                .get_string("promiseId")
                .unwrap_or_default()
                .to_string();

            // Look for terminal event (resolved/rejected/timeout) by promiseId
            if let Some(terminal_event) =
                self.replay_engine.find_terminal_promise_event(&promise_id)
            {
                match terminal_event.event_type() {
                    EventType::PromiseResolved => {
                        let value = terminal_event.get("value").cloned().unwrap_or(Value::Null);
                        return PromiseFuture::from_replay_with_cell(
                            promise_seq,
                            event_promise_name,
                            self.suspension_cell.clone(),
                            Ok(value),
                        );
                    }
                    EventType::PromiseRejected => {
                        let error = terminal_event
                            .get_string("error")
                            .unwrap_or("Promise rejected")
                            .to_string();
                        return PromiseFuture::from_replay_with_cell(
                            promise_seq,
                            event_promise_name,
                            self.suspension_cell.clone(),
                            Err(FlovynError::PromiseRejected {
                                name: name.to_string(),
                                error,
                            }),
                        );
                    }
                    EventType::PromiseTimeout => {
                        return PromiseFuture::from_replay_with_cell(
                            promise_seq,
                            event_promise_name,
                            self.suspension_cell.clone(),
                            Err(FlovynError::PromiseTimeout {
                                name: name.to_string(),
                            }),
                        );
                    }
                    _ => {}
                }
            }

            // Promise created but not resolved yet - return pending future
            return PromiseFuture::new_with_cell(
                promise_seq,
                event_promise_name,
                self.suspension_cell.clone(),
            );
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        if let Err(e) = self.record_command(WorkflowCommand::CreatePromise {
            sequence_number: sequence,
            promise_id: name.to_string(),
            timeout_ms: None,
            idempotency_key: None,
            idempotency_key_ttl_seconds: None,
        }) {
            return PromiseFuture::with_error(e);
        }

        // Return pending future
        PromiseFuture::new_with_cell(promise_seq, name.to_string(), self.suspension_cell.clone())
    }

    fn promise_with_timeout_raw(&self, name: &str, _timeout: Duration) -> PromiseFutureRaw {
        // TODO: Handle timeout in the future implementation
        // For now, just delegate to promise_raw - the server handles timeout
        self.promise_raw(name)
    }

    fn promise_with_options_raw(&self, name: &str, options: PromiseOptions) -> PromiseFutureRaw {
        // Get per-type sequence and increment atomically.
        let promise_seq = self.replay_engine.next_promise_seq();

        // Look for event at this per-type index (replay case)
        if let Some(created_event) = self.replay_engine.get_promise_event(promise_seq) {
            // Validate promise name matches
            let event_promise_name = created_event
                .get_string("promiseName")
                .unwrap_or_default()
                .to_string();

            if event_promise_name != name {
                return PromiseFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::PromiseNameMismatch {
                        sequence: promise_seq as i32,
                        expected: event_promise_name,
                        actual: name.to_string(),
                    },
                ));
            }

            // Get promiseId (UUID) for terminal event lookup
            let promise_id = created_event
                .get_string("promiseId")
                .unwrap_or_default()
                .to_string();

            // Check for terminal event (resolved/rejected/timeout) by promiseId
            if let Some(terminal_event) =
                self.replay_engine.find_terminal_promise_event(&promise_id)
            {
                match terminal_event.event_type() {
                    EventType::PromiseResolved => {
                        let value = terminal_event.get("value").cloned().unwrap_or(Value::Null);
                        return PromiseFuture::from_replay_with_cell(
                            promise_seq,
                            event_promise_name,
                            self.suspension_cell.clone(),
                            Ok(value),
                        );
                    }
                    EventType::PromiseRejected => {
                        let error = terminal_event
                            .get_string("error")
                            .unwrap_or("Promise rejected")
                            .to_string();
                        return PromiseFuture::from_replay_with_cell(
                            promise_seq,
                            event_promise_name,
                            self.suspension_cell.clone(),
                            Err(FlovynError::PromiseRejected {
                                name: name.to_string(),
                                error,
                            }),
                        );
                    }
                    EventType::PromiseTimeout => {
                        return PromiseFuture::from_replay_with_cell(
                            promise_seq,
                            event_promise_name,
                            self.suspension_cell.clone(),
                            Err(FlovynError::PromiseTimeout {
                                name: name.to_string(),
                            }),
                        );
                    }
                    _ => {}
                }
            }

            // Promise created but not resolved yet - return pending future
            return PromiseFuture::new_with_cell(
                promise_seq,
                event_promise_name,
                self.suspension_cell.clone(),
            );
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        let timeout_ms = options.timeout.map(|d| d.as_millis() as i64);
        if let Err(e) = self.record_command(WorkflowCommand::CreatePromise {
            sequence_number: sequence,
            promise_id: name.to_string(),
            timeout_ms,
            idempotency_key: options.idempotency_key,
            idempotency_key_ttl_seconds: options.idempotency_key_ttl_seconds,
        }) {
            return PromiseFuture::with_error(e);
        }

        // Return pending future
        PromiseFuture::new_with_cell(promise_seq, name.to_string(), self.suspension_cell.clone())
    }

    fn run_raw(&self, name: &str, result: Value) -> OperationFutureRaw {
        // Get per-type sequence and increment atomically.
        let op_seq = self.replay_engine.next_operation_seq();

        // Look for event at this per-type index (replay case)
        if let Some(operation_event) = self.replay_engine.get_operation_event(op_seq) {
            // Validate operation name matches
            // Note: Server may use "name" or "operationName" for the field
            let event_op_name = operation_event
                .get_string("operationName")
                .or_else(|| operation_event.get_string("name"))
                .unwrap_or_default()
                .to_string();

            if event_op_name != name {
                return OperationFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::OperationNameMismatch {
                        sequence: op_seq as i32,
                        expected: event_op_name,
                        actual: name.to_string(),
                    },
                ));
            }

            // Return the cached result from the event
            let cached_result = operation_event
                .get("result")
                .cloned()
                .unwrap_or(Value::Null);

            return OperationFuture::new(op_seq, name.to_string(), Ok(cached_result));
        }

        // No event at this per-type index → new operation
        let sequence = self.next_sequence();
        if let Err(e) = self.record_command(WorkflowCommand::RecordOperation {
            sequence_number: sequence,
            operation_name: name.to_string(),
            result: result.clone(),
        }) {
            return OperationFuture::with_error(e);
        }

        // Operations complete immediately - return ready future
        OperationFuture::new(op_seq, name.to_string(), Ok(result))
    }
}

// Dummy context types for futures that don't need context reference
// These are used as type parameters for Weak::new() to create empty weak references
#[allow(dead_code)]
struct DummyTaskFutureContext;
impl SuspensionContext for DummyTaskFutureContext {
    fn signal_suspension(&self, _reason: String) {
        // No-op for dummy context - futures will fall back to thread-local
    }
}
impl TaskFutureContext for DummyTaskFutureContext {
    fn find_task_result(&self, _: &Uuid) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_task(&self, _: &Uuid) {}
}

#[allow(dead_code)]
struct DummyTimerFutureContext;
impl SuspensionContext for DummyTimerFutureContext {
    fn signal_suspension(&self, _reason: String) {}
}
impl TimerFutureContext for DummyTimerFutureContext {
    fn find_timer_result(&self, _: &str) -> Option<Result<()>> {
        None
    }
    fn record_cancel_timer(&self, _: &str) {}
}

#[allow(dead_code)]
struct DummyChildWorkflowFutureContext;
impl SuspensionContext for DummyChildWorkflowFutureContext {
    fn signal_suspension(&self, _reason: String) {}
}
impl ChildWorkflowFutureContext for DummyChildWorkflowFutureContext {
    fn find_child_workflow_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_child_workflow(&self, _: &Uuid) {}
}

#[allow(dead_code)]
struct DummyPromiseFutureContext;
impl SuspensionContext for DummyPromiseFutureContext {
    fn signal_suspension(&self, _reason: String) {}
}
impl PromiseFutureContext for DummyPromiseFutureContext {
    fn find_promise_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::recorder::CommandCollector;

    fn create_test_context() -> WorkflowContextImpl<CommandCollector> {
        WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            vec![],
            1700000000000, // Fixed timestamp for tests
        )
    }

    // === SeededRandom Tests ===

    #[test]
    fn test_seeded_random_deterministic() {
        let random1 = SeededRandom::new(12345);
        let random2 = SeededRandom::new(12345);

        // Same seed should produce same sequence
        assert_eq!(random1.next_int(0, 100), random2.next_int(0, 100));
        assert_eq!(random1.next_long(0, 1000), random2.next_long(0, 1000));
        assert_eq!(random1.next_double(), random2.next_double());
        assert_eq!(random1.next_bool(), random2.next_bool());
    }

    #[test]
    fn test_seeded_random_different_seeds() {
        let random1 = SeededRandom::new(12345);
        let random2 = SeededRandom::new(54321);

        // Different seeds should (likely) produce different values
        let val1 = random1.next_int(0, 1000000);
        let val2 = random2.next_int(0, 1000000);
        // Note: There's a tiny chance they could be equal, but astronomically unlikely
        assert_ne!(val1, val2);
    }

    #[test]
    fn test_seeded_random_range() {
        let random = SeededRandom::new(42);

        for _ in 0..100 {
            let val = random.next_int(10, 20);
            assert!((10..20).contains(&val));
        }

        for _ in 0..100 {
            let val = random.next_long(100, 200);
            assert!((100..200).contains(&val));
        }

        for _ in 0..100 {
            let val = random.next_double();
            assert!((0.0..1.0).contains(&val));
        }
    }

    // === WorkflowContextImpl Tests ===

    #[test]
    fn test_workflow_execution_id() {
        let exec_id = Uuid::new_v4();
        let ctx = WorkflowContextImpl::new(
            exec_id,
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            vec![],
            0,
        );
        assert_eq!(ctx.workflow_execution_id(), exec_id);
    }

    #[test]
    fn test_org_id() {
        let org_id = Uuid::new_v4();
        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            org_id,
            serde_json::json!({}),
            CommandCollector::new(),
            vec![],
            0,
        );
        assert_eq!(ctx.org_id(), org_id);
    }

    #[test]
    fn test_input_raw() {
        let input = serde_json::json!({"key": "value", "num": 42});
        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            input.clone(),
            CommandCollector::new(),
            vec![],
            0,
        );
        assert_eq!(ctx.input_raw(), &input);
    }

    #[test]
    fn test_current_time_millis() {
        let ctx = create_test_context();
        assert_eq!(ctx.current_time_millis(), 1700000000000);
    }

    #[test]
    fn test_random_uuid_deterministic() {
        let exec_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        let ctx1 = WorkflowContextImpl::new(
            exec_id,
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            vec![],
            0,
        );

        let ctx2 = WorkflowContextImpl::new(
            exec_id,
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            vec![],
            0,
        );

        // Same workflow execution ID should produce same UUIDs
        assert_eq!(ctx1.random_uuid(), ctx2.random_uuid());
        assert_eq!(ctx1.random_uuid(), ctx2.random_uuid());
        assert_eq!(ctx1.random_uuid(), ctx2.random_uuid());
    }

    #[test]
    fn test_random_uuid_unique_sequence() {
        let ctx = create_test_context();

        let uuid1 = ctx.random_uuid();
        let uuid2 = ctx.random_uuid();
        let uuid3 = ctx.random_uuid();

        // Each UUID should be different
        assert_ne!(uuid1, uuid2);
        assert_ne!(uuid2, uuid3);
        assert_ne!(uuid1, uuid3);
    }

    #[test]
    fn test_random_deterministic() {
        let exec_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        let ctx1 = WorkflowContextImpl::new(
            exec_id,
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            vec![],
            0,
        );

        let ctx2 = WorkflowContextImpl::new(
            exec_id,
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            vec![],
            0,
        );

        // Same execution ID should produce same random sequence
        assert_eq!(
            ctx1.random().next_int(0, 100),
            ctx2.random().next_int(0, 100)
        );
    }

    #[test]
    fn test_cancellation() {
        let ctx = create_test_context();

        assert!(!ctx.is_cancellation_requested());

        ctx.request_cancellation();

        assert!(ctx.is_cancellation_requested());
    }

    #[tokio::test]
    async fn test_check_cancellation_not_cancelled() {
        let ctx = create_test_context();
        assert!(ctx.check_cancellation().await.is_ok());
    }

    #[tokio::test]
    async fn test_check_cancellation_cancelled() {
        let ctx = create_test_context();
        ctx.request_cancellation();

        let result = ctx.check_cancellation().await;
        assert!(matches!(result, Err(FlovynError::WorkflowCancelled(_))));
    }

    #[tokio::test]
    async fn test_run_raw_records_command() {
        let ctx = create_test_context();

        let result = ctx
            .run_raw("my-operation", serde_json::json!({"data": 42}))
            .await
            .unwrap();

        assert_eq!(result, serde_json::json!({"data": 42}));

        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            WorkflowCommand::RecordOperation { operation_name, .. } if operation_name == "my-operation"
        ));
    }

    #[tokio::test]
    async fn test_run_raw_each_call_is_distinct_operation() {
        // With sequence-based replay, each run_raw call is a distinct operation
        // (Operation(0), Operation(1), etc.) regardless of name
        let ctx = create_test_context();

        // First call - Operation(0)
        let result1 = ctx
            .run_raw("op-name", serde_json::json!({"first": true}))
            .await
            .unwrap();

        // Second call - Operation(1), distinct from first
        let result2 = ctx
            .run_raw("op-name", serde_json::json!({"second": true}))
            .await
            .unwrap();

        // Each call returns its own result
        assert_eq!(result1, serde_json::json!({"first": true}));
        assert_eq!(result2, serde_json::json!({"second": true}));

        // Both operations are recorded as separate commands
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 2);
    }

    #[tokio::test]
    async fn test_state_management() {
        let ctx = create_test_context();

        // Initially empty
        assert!(ctx.state_keys().await.unwrap().is_empty());
        assert!(ctx.get_raw("key1").await.unwrap().is_none());

        // Set a value
        ctx.set_raw("key1", serde_json::json!({"value": 1}))
            .await
            .unwrap();

        // Verify it's set
        let value = ctx.get_raw("key1").await.unwrap().unwrap();
        assert_eq!(value, serde_json::json!({"value": 1}));

        // Verify keys
        let keys = ctx.state_keys().await.unwrap();
        assert_eq!(keys.len(), 1);
        assert!(keys.contains(&"key1".to_string()));

        // Clear the key
        ctx.clear("key1").await.unwrap();

        // Verify it's gone
        assert!(ctx.get_raw("key1").await.unwrap().is_none());
    }

    #[test]
    fn test_schedule_raw_creates_pending_future() {
        // Use context with task submitter since schedule_raw now submits tasks via gRPC
        let ctx = create_test_context();

        // Create a future - it should be pending (not complete)
        let future = ctx.schedule_raw("my-task", serde_json::json!({"input": true}));

        // Verify the future has correct sequence
        assert_eq!(future.task_seq, 0);

        // Verify command was recorded immediately when future was created
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            WorkflowCommand::ScheduleTask { kind, .. } if kind == "my-task"
        ));
    }

    #[tokio::test]
    async fn test_schedule_raw_returns_completed_from_replay() {
        use chrono::Utc;
        // Create context with replay events showing task was already scheduled and completed
        let task_execution_id = Uuid::new_v4();
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "my-task",
                    "taskExecutionId": task_execution_id.to_string(),
                    "input": {"input": true}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_execution_id.to_string(),
                    "output": {"result": 42}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the task was already completed in replay, we should get the result
        let result = ctx
            .schedule_raw("my-task", serde_json::json!({"input": true}))
            .await
            .unwrap();

        assert_eq!(result, serde_json::json!({"result": 42}));
    }

    #[test]
    fn test_sleep_creates_pending_future() {
        let ctx = create_test_context();

        // Create a timer future - it should be pending (not fired)
        let timer = ctx.sleep(Duration::from_secs(60));

        // Verify the future has correct sequence and ID
        assert_eq!(timer.timer_seq, 0);
        assert!(!timer.timer_id.is_empty());

        // Verify command was recorded immediately when future was created
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], WorkflowCommand::StartTimer { .. }));
    }

    #[tokio::test]
    async fn test_sleep_returns_immediately_when_timer_fired() {
        use chrono::Utc;
        // Create context with replay events showing timer was already started and fired
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TimerStarted,
                serde_json::json!({
                    "timerId": "sleep-1",
                    "durationMs": 60000
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TimerFired,
                serde_json::json!({
                    "timerId": "sleep-1"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the timer was already fired in replay, sleep should return immediately
        let result = ctx.sleep(Duration::from_secs(60)).await;
        assert!(result.is_ok());

        // No command should be recorded (we're replaying, not creating new commands)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_tasks_same_type_each_consume_one_event() {
        use chrono::Utc;
        // Test that multiple schedule() calls with the same taskType each consume a different event.
        // This is critical for handling patterns like:
        //   let result1 = ctx.schedule("fast-task", input1).await;
        //   let result2 = ctx.schedule("fast-task", input2).await;
        // Each call should get its respective result, not both getting the first.

        let task_id_1 = Uuid::new_v4();
        let task_id_2 = Uuid::new_v4();
        let replay_events = vec![
            // First task scheduled and completed
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "fast-task",
                    "taskExecutionId": task_id_1.to_string(),
                    "input": {"seq": 1}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_1.to_string(),
                    "output": {"result": "first"}
                }),
                Utc::now(),
            ),
            // Second task scheduled and completed
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "fast-task",
                    "taskExecutionId": task_id_2.to_string(),
                    "input": {"seq": 2}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_2.to_string(),
                    "output": {"result": "second"}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // First schedule call should return "first" result
        let result1 = ctx
            .schedule_raw("fast-task", serde_json::json!({"seq": 1}))
            .await
            .unwrap();
        assert_eq!(result1, serde_json::json!({"result": "first"}));

        // Second schedule call should return "second" result (not "first" again)
        let result2 = ctx
            .schedule_raw("fast-task", serde_json::json!({"seq": 2}))
            .await
            .unwrap();
        assert_eq!(result2, serde_json::json!({"result": "second"}));
    }

    #[test]
    fn test_task_still_running_creates_pending_future() {
        use chrono::Utc;
        // Test that when a task is scheduled but not yet completed, we get a pending future
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "slow-task",
                    "taskExecutionId": "task-pending",
                    "input": {}
                }),
                Utc::now(),
            ),
            // No TaskCompleted event - task is still running
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Create a future - it should be pending (task not completed yet)
        let future = ctx.schedule_raw("slow-task", serde_json::json!({}));

        // Verify the future has correct sequence from replay
        assert_eq!(future.task_seq, 0);

        // No new command should be recorded (task was already scheduled in replay)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[tokio::test]
    async fn test_task_failed_returns_error() {
        use chrono::Utc;
        // Test that when a task fails, the error is propagated
        let task_execution_id = Uuid::new_v4();
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "failing-task",
                    "taskExecutionId": task_execution_id.to_string(),
                    "input": {}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskFailed,
                serde_json::json!({
                    "taskExecutionId": task_execution_id.to_string(),
                    "error": "Connection timeout"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        let result = ctx
            .schedule_raw("failing-task", serde_json::json!({}))
            .await;
        assert!(matches!(result, Err(FlovynError::TaskFailed(msg)) if msg == "Connection timeout"));
    }

    #[test]
    fn test_promise_creates_pending_future() {
        let ctx = create_test_context();

        // Create a promise future - it should be pending (not resolved)
        let promise = ctx.promise_raw("my-promise");

        // Verify the future has correct sequence
        assert_eq!(promise.promise_seq, 0);
        assert_eq!(promise.promise_id, "my-promise");

        // Verify command was recorded immediately when future was created
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(&commands[0], WorkflowCommand::CreatePromise { promise_id, .. } if promise_id == "my-promise")
        );
    }

    // Note: test_promise_returns_resolved was removed because resolve_promise() sets
    // in-memory state that the new Future-returning API doesn't check. Use replay events instead.

    #[tokio::test]
    async fn test_promise_returns_immediately_when_resolved_during_replay() {
        use chrono::Utc;
        let promise_id = "550e8400-e29b-41d4-a716-446655440000";
        // Create context with replay events showing promise was already created and resolved
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                serde_json::json!({
                    "promiseName": "user-approval",
                    "promiseId": promise_id,
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseResolved,
                serde_json::json!({
                    "promiseName": "user-approval",
                    "promiseId": promise_id,
                    "value": {"approved": true, "approver": "admin"}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the promise was already resolved in replay, promise_raw should return immediately
        let result = ctx.promise_raw("user-approval").await;
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            serde_json::json!({"approved": true, "approver": "admin"})
        );

        // No command should be recorded (we're replaying, not creating new commands)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[tokio::test]
    async fn test_promise_returns_error_when_rejected_during_replay() {
        use chrono::Utc;
        let promise_id = "550e8400-e29b-41d4-a716-446655440001";
        // Create context with replay events showing promise was created and rejected
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                serde_json::json!({
                    "promiseName": "user-approval",
                    "promiseId": promise_id,
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseRejected,
                serde_json::json!({
                    "promiseName": "user-approval",
                    "promiseId": promise_id,
                    "error": "Approval denied by supervisor"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the promise was rejected in replay, promise_raw should return error
        let result = ctx.promise_raw("user-approval").await;
        assert!(
            matches!(result, Err(FlovynError::PromiseRejected { name, error })
            if name == "user-approval" && error == "Approval denied by supervisor")
        );

        // No command should be recorded
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[test]
    fn test_promise_pending_when_created_but_not_resolved_during_replay() {
        use chrono::Utc;
        let promise_id = "550e8400-e29b-41d4-a716-446655440002";
        // Create context with replay events showing promise was created but not resolved
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                serde_json::json!({
                    "promiseName": "user-approval",
                    "promiseId": promise_id,
                }),
                Utc::now(),
            ),
            // No PROMISE_RESOLVED event - promise is still pending
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the promise was created but not resolved, the future should be pending
        // (not await it - that would block forever)
        let promise = ctx.promise_raw("user-approval");

        // Verify the future was created with correct info from replay
        assert_eq!(promise.promise_seq, 0);
        assert_eq!(promise.promise_id, "user-approval");

        // No command should be recorded (promise was already created in replay)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[test]
    fn test_schedule_workflow_creates_pending_future() {
        let ctx = create_test_context();

        // Create a child workflow future - it should be pending (not completed)
        let child = ctx.schedule_workflow_raw("child-1", "payment-workflow", serde_json::json!({}));

        // Verify the future has correct sequence
        assert_eq!(child.child_workflow_seq, 0);
        assert_eq!(child.child_execution_name, "child-1");

        // Verify command was recorded immediately when future was created
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            WorkflowCommand::ScheduleChildWorkflow { name, kind, .. }
                if name == "child-1" && kind == &Some("payment-workflow".to_string())
        ));
    }

    // Note: test_schedule_workflow_returns_resolved was removed because resolve_child_workflow() sets
    // in-memory state that the new Future-returning API doesn't check. Use replay events instead.

    #[tokio::test]
    async fn test_schedule_workflow_returns_immediately_when_completed_during_replay() {
        use chrono::Utc;
        // Create context with replay events showing child workflow was already initiated and completed
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                serde_json::json!({
                    "childExecutionName": "payment-child",
                    "childworkflowExecutionId": "child-123"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::ChildWorkflowCompleted,
                serde_json::json!({
                    "childExecutionName": "payment-child",
                    "childworkflowExecutionId": "child-123",
                    "output": {"payment": "confirmed", "amount": 100}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the child workflow was already completed in replay, schedule_workflow_raw should return immediately
        let result = ctx
            .schedule_workflow_raw("payment-child", "payment-workflow", serde_json::json!({}))
            .await;
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            serde_json::json!({"payment": "confirmed", "amount": 100})
        );

        // No command should be recorded (we're replaying, not creating new commands)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[tokio::test]
    async fn test_schedule_workflow_returns_error_when_failed_during_replay() {
        use chrono::Utc;
        // Create context with replay events showing child workflow was initiated and failed
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                serde_json::json!({
                    "childExecutionName": "payment-child",
                    "childworkflowExecutionId": "child-123"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::ChildWorkflowFailed,
                serde_json::json!({
                    "childExecutionName": "payment-child",
                    "childworkflowExecutionId": "child-123",
                    "error": "Payment gateway unavailable"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the child workflow was already failed in replay, should return error
        let result = ctx
            .schedule_workflow_raw("payment-child", "payment-workflow", serde_json::json!({}))
            .await;
        assert!(
            matches!(result, Err(FlovynError::ChildWorkflowFailed { name, error, .. })
            if name == "payment-child" && error == "Payment gateway unavailable")
        );

        // No command should be recorded
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[test]
    fn test_schedule_workflow_pending_when_initiated_but_not_completed_during_replay() {
        use chrono::Utc;
        // Create context with replay events showing child workflow was initiated but not completed
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                serde_json::json!({
                    "childExecutionName": "payment-child",
                    "childworkflowExecutionId": "child-123"
                }),
                Utc::now(),
            ),
            // No CHILD_WORKFLOW_COMPLETED event - child workflow is still running
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Since the child workflow was initiated but not completed, the future should be pending
        // (not await it - that would block forever)
        let child =
            ctx.schedule_workflow_raw("payment-child", "payment-workflow", serde_json::json!({}));

        // Verify the future was created with correct info from replay
        assert_eq!(child.child_workflow_seq, 0);
        assert_eq!(child.child_execution_name, "payment-child");

        // No command should be recorded (child workflow was already initiated in replay)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    /// Test that multiple child workflows after replay get unique UUIDs.
    ///
    /// This is a regression test for the bug where uuid_counter wasn't incremented
    /// during replay, causing subsequent child workflows to get the same UUID.
    /// See: .dev/docs/bugs/20260107_uuid_counter_not_incremented_during_replay.md
    #[test]
    fn test_multiple_child_workflows_after_replay_get_unique_uuids() {
        use chrono::Utc;
        let workflow_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        // Create context with replay events showing first child workflow completed
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                serde_json::json!({
                    "childExecutionName": "process-item-0",
                    "childExecutionId": "11111111-1111-1111-1111-111111111111",
                    "childWorkflowKind": "item-processor"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::ChildWorkflowCompleted,
                serde_json::json!({
                    "childExecutionName": "process-item-0",
                    "output": {"result": "done"}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            workflow_id,
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // First call: replays the completed child workflow (shouldn't generate new command)
        let child1 =
            ctx.schedule_workflow_raw("process-item-0", "item-processor", serde_json::json!({}));
        assert_eq!(child1.child_workflow_seq, 0);

        // No command for replayed child
        let commands_after_first = ctx.get_commands();
        assert!(
            commands_after_first.is_empty(),
            "First child should not generate command (replayed)"
        );

        // Second call: NEW child workflow - should generate unique UUID
        let child2 =
            ctx.schedule_workflow_raw("process-item-1", "item-processor", serde_json::json!({}));
        assert_eq!(child2.child_workflow_seq, 1);

        // Should have generated a command with a new UUID
        let commands_after_second = ctx.get_commands();
        assert_eq!(
            commands_after_second.len(),
            1,
            "Second child should generate command"
        );

        // Extract the child execution ID from the command
        if let WorkflowCommand::ScheduleChildWorkflow {
            child_execution_id, ..
        } = &commands_after_second[0]
        {
            // The second child's UUID should NOT be the same as the first child's
            // First child had UUID "11111111-1111-1111-1111-111111111111" from replay
            let first_child_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
            assert_ne!(
                *child_execution_id, first_child_id,
                "Second child must have different UUID than first child"
            );

            // Additionally, verify the UUID is deterministic based on counter=1
            // (counter=0 was used for first child during original execution)
            let expected_uuid = Uuid::new_v5(&workflow_id, format!("{}:1", workflow_id).as_bytes());
            assert_eq!(
                *child_execution_id, expected_uuid,
                "Second child UUID should use counter=1"
            );
        } else {
            panic!("Expected ScheduleChildWorkflow command");
        }
    }

    // ============= Sequence-Based Replay Tests =============
    // Tests for the per-type cursor matching behavior

    #[tokio::test]
    async fn test_sequence_based_task_type_mismatch_violation() {
        use chrono::Utc;
        // Create context with replay event for a different task type
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::TaskScheduled,
            serde_json::json!({
                "kind": "send-email",
                "taskExecutionId": "task-abc-123"
            }),
            Utc::now(),
        )];

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to schedule a different task type - should return determinism violation error
        let result = ctx
            .schedule_raw("process-payment", serde_json::json!({}))
            .await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_operation_name_mismatch_violation() {
        use chrono::Utc;
        // Create context with replay event for a different operation name
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            serde_json::json!({
                "operationName": "fetch-user",
                "result": {"id": 123}
            }),
            Utc::now(),
        )];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to run a different operation - should return determinism violation error
        let result = ctx.run_raw("calculate-total", serde_json::json!(100)).await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_timer_id_mismatch_violation() {
        use chrono::Utc;
        // Create context with replay event for a timer with different ID
        // The timer ID is generated as "sleep-N" based on the sleep_call_counter
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::TimerStarted,
            serde_json::json!({
                "timerId": "sleep-99",  // Mismatched - first sleep should have ID "sleep-1"
                "durationMs": 5000
            }),
            Utc::now(),
        )];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to sleep - should return determinism violation error because timer ID doesn't match
        let result = ctx.sleep(Duration::from_secs(5)).await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_promise_name_mismatch_violation() {
        use chrono::Utc;
        let promise_id = "550e8400-e29b-41d4-a716-446655440003";
        // Create context with replay event for a different promise name
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::PromiseCreated,
            serde_json::json!({
                "promiseName": "approval-promise",
                "promiseId": promise_id,
            }),
            Utc::now(),
        )];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to create a different promise - should return determinism violation error
        let result = ctx.promise_raw("payment-confirmation").await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_child_workflow_name_mismatch_violation() {
        use chrono::Utc;
        // Create context with replay event for a different child workflow name
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::ChildWorkflowInitiated,
            serde_json::json!({
                "childExecutionName": "email-notification",
                "childworkflowExecutionId": "child-xyz"
            }),
            Utc::now(),
        )];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to schedule a different child workflow - should return determinism violation error
        let result = ctx
            .schedule_workflow_raw("payment-processor", "payment-wf", serde_json::json!({}))
            .await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_interleaved_operations_replay_correctly() {
        use chrono::Utc;
        // Create context with interleaved operations of different types
        let replay_events = vec![
            // Operation(0)
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                serde_json::json!({
                    "operationName": "op-1",
                    "result": "result-1"
                }),
                Utc::now(),
            ),
            // Timer(0)
            ReplayEvent::new(
                2,
                EventType::TimerStarted,
                serde_json::json!({
                    "timerId": "sleep-1",
                    "durationMs": 1000
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                3,
                EventType::TimerFired,
                serde_json::json!({
                    "timerId": "sleep-1"
                }),
                Utc::now(),
            ),
            // Operation(1)
            ReplayEvent::new(
                4,
                EventType::OperationCompleted,
                serde_json::json!({
                    "operationName": "op-2",
                    "result": "result-2"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Execute in the same order as the replay events
        let result1 = ctx
            .run_raw("op-1", serde_json::json!("new-1"))
            .await
            .unwrap();
        assert_eq!(result1, serde_json::json!("result-1")); // Returns replayed value

        let sleep_result = ctx.sleep(Duration::from_secs(1)).await;
        assert!(sleep_result.is_ok()); // Timer already fired

        let result2 = ctx
            .run_raw("op-2", serde_json::json!("new-2"))
            .await
            .unwrap();
        assert_eq!(result2, serde_json::json!("result-2")); // Returns replayed value
    }

    #[tokio::test]
    async fn test_sequence_based_state_operations_validate_key() {
        use chrono::Utc;
        // Create context with replay events for state operations
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::StateSet,
                serde_json::json!({
                    "key": "counter",
                    "value": 42
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::StateCleared,
                serde_json::json!({
                    "key": "temp-data"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // These should succeed because keys match
        ctx.set_raw("counter", serde_json::json!(100))
            .await
            .unwrap();
        ctx.clear("temp-data").await.unwrap();

        // No new commands should be recorded (replaying)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[tokio::test]
    async fn test_sequence_based_state_key_mismatch_violation() {
        use chrono::Utc;
        // Create context with replay event for a different state key
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::StateSet,
            serde_json::json!({
                "key": "user-id",
                "value": 123
            }),
            Utc::now(),
        )];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to set a different key - should return determinism violation error
        let result = ctx.set_raw("order-id", serde_json::json!(456)).await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_new_commands_beyond_replay_history() {
        use chrono::Utc;
        // Create context with one operation in history
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            serde_json::json!({
                "operationName": "op-1",
                "result": "cached-result"
            }),
            Utc::now(),
        )];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // First operation - replays from history
        let result1 = ctx
            .run_raw("op-1", serde_json::json!("ignored"))
            .await
            .unwrap();
        assert_eq!(result1, serde_json::json!("cached-result"));

        // Second operation - new, beyond history
        let result2 = ctx
            .run_raw("op-2", serde_json::json!("new-result"))
            .await
            .unwrap();
        assert_eq!(result2, serde_json::json!("new-result"));

        // Only the second operation should have recorded a command
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            WorkflowCommand::RecordOperation { operation_name, .. } if operation_name == "op-2"
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_loop_extended_is_valid() {
        use chrono::Utc;
        // Original execution: 2 tasks in a loop
        // New execution: 3 tasks in a loop (loop extended)
        // This should be valid - new commands beyond history are allowed
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                serde_json::json!({
                    "operationName": "loop-item-0",
                    "result": "done-0"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::OperationCompleted,
                serde_json::json!({
                    "operationName": "loop-item-1",
                    "result": "done-1"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Simulate loop with 3 iterations (extended from original 2)
        for i in 0..3 {
            let op_name = format!("loop-item-{}", i);
            let result = ctx
                .run_raw(&op_name, serde_json::json!(format!("new-{}", i)))
                .await
                .unwrap();

            if i < 2 {
                // First 2 iterations replay from history
                assert_eq!(result, serde_json::json!(format!("done-{}", i)));
            } else {
                // 3rd iteration is new
                assert_eq!(result, serde_json::json!("new-2"));
            }
        }

        // Only the 3rd operation (new) should have recorded a command
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
    }

    #[tokio::test]
    async fn test_sequence_based_mixed_task_types_in_loop_must_match_order() {
        use chrono::Utc;
        // History: task-A, task-B, task-A, task-B (alternating pattern)
        let task_id_1 = Uuid::new_v4();
        let task_id_2 = Uuid::new_v4();
        let task_id_3 = Uuid::new_v4();
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "task-A",
                    "taskExecutionId": task_id_1.to_string()
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_1.to_string(),
                    "result": "A-result-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "task-B",
                    "taskExecutionId": task_id_2.to_string()
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_2.to_string(),
                    "result": "B-result-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                5,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "task-A",
                    "taskExecutionId": task_id_3.to_string()
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                6,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_3.to_string(),
                    "result": "A-result-2"
                }),
                Utc::now(),
            ),
        ];

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Replay in same order: A, B, A - should succeed
        let result_a1 = ctx
            .schedule_raw("task-A", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result_a1, serde_json::json!("A-result-1"));

        let result_b1 = ctx
            .schedule_raw("task-B", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result_b1, serde_json::json!("B-result-1"));

        let result_a2 = ctx
            .schedule_raw("task-A", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result_a2, serde_json::json!("A-result-2"));
    }

    #[tokio::test]
    async fn test_sequence_based_mixed_task_types_wrong_order_raises_violation() {
        use chrono::Utc;
        // History: task-A first
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "task-A",
                    "taskExecutionId": "task-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-1",
                    "result": "A-result"
                }),
                Utc::now(),
            ),
        ];

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to schedule task-B first (wrong order) - should return determinism violation error
        let result = ctx.schedule_raw("task-B", serde_json::json!({})).await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_child_workflow_kind_mismatch_violation() {
        use chrono::Utc;
        // Create context with replay event for a different child workflow kind
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::ChildWorkflowInitiated,
            serde_json::json!({
                "childExecutionName": "payment-child",
                "childWorkflowKind": "payment-workflow-v1",
                "childworkflowExecutionId": "child-xyz"
            }),
            Utc::now(),
        )];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Try to schedule with different kind - should return determinism violation error
        let result = ctx
            .schedule_workflow_raw(
                "payment-child",
                "payment-workflow-v2",
                serde_json::json!({}),
            )
            .await;
        assert!(matches!(
            result,
            Err(crate::error::FlovynError::DeterminismViolation(_))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_task_matches_at_correct_sequence() {
        use chrono::Utc;
        // Create context with task that completed successfully
        let task_execution_id = Uuid::new_v4();
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "process-order",
                    "taskExecutionId": task_execution_id.to_string()
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_execution_id.to_string(),
                    "result": {"orderId": 12345, "status": "processed"}
                }),
                Utc::now(),
            ),
        ];

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Should match the task at sequence 0 and return result
        let result = ctx
            .schedule_raw("process-order", serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(
            result,
            serde_json::json!({"orderId": 12345, "status": "processed"})
        );

        // No new commands should be recorded (replaying)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[tokio::test]
    async fn test_sequence_based_child_workflow_matches_by_sequence_and_name() {
        use chrono::Utc;
        // Create context with child workflow that completed
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                serde_json::json!({
                    "childExecutionName": "notify-user",
                    "childWorkflowKind": "notification-workflow",
                    "childworkflowExecutionId": "child-123"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::ChildWorkflowCompleted,
                serde_json::json!({
                    "childExecutionName": "notify-user",
                    "childworkflowExecutionId": "child-123",
                    "output": {"notified": true, "channel": "email"}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Should match by sequence (0) and validate name matches
        let result = ctx
            .schedule_workflow_raw(
                "notify-user",
                "notification-workflow",
                serde_json::json!({}),
            )
            .await
            .unwrap();

        assert_eq!(
            result,
            serde_json::json!({"notified": true, "channel": "email"})
        );

        // No new commands should be recorded (replaying)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    /// Test that a workflow can complete with fewer iterations than the original.
    /// Orphaned events (not accessed during replay) are allowed.
    #[tokio::test]
    async fn test_sequence_based_loop_shortened_is_valid() {
        use chrono::Utc;
        // Original execution: 3 task iterations
        // Replay execution: 2 task iterations (workflow code changed)
        // Expected: Valid - orphaned events are allowed
        let task_id_1 = Uuid::new_v4();
        let task_id_2 = Uuid::new_v4();
        let task_id_3 = Uuid::new_v4();
        let replay_events = vec![
            // Task iteration 1
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "process-item",
                    "taskExecutionId": task_id_1.to_string()
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_1.to_string(),
                    "result": {"value": 1}
                }),
                Utc::now(),
            ),
            // Task iteration 2
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "process-item",
                    "taskExecutionId": task_id_2.to_string()
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_2.to_string(),
                    "result": {"value": 2}
                }),
                Utc::now(),
            ),
            // Task iteration 3 (will be orphaned - never accessed)
            ReplayEvent::new(
                5,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "process-item",
                    "taskExecutionId": task_id_3.to_string()
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                6,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_id_3.to_string(),
                    "result": {"value": 3}
                }),
                Utc::now(),
            ),
        ];

        let ctx: WorkflowContextImpl<CommandCollector> = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Shortened workflow: only 2 iterations instead of 3
        let result1 = ctx
            .schedule_raw("process-item", serde_json::json!({}))
            .await;
        assert!(result1.is_ok(), "First task should replay: {:?}", result1);
        assert_eq!(result1.unwrap(), serde_json::json!({"value": 1}));

        let result2 = ctx
            .schedule_raw("process-item", serde_json::json!({}))
            .await;
        assert!(result2.is_ok(), "Second task should replay: {:?}", result2);
        assert_eq!(result2.unwrap(), serde_json::json!({"value": 2}));

        // Workflow completes here without accessing task-3
        // Events 5-6 are orphaned, but this is valid behavior
        // No determinism violation should occur

        // No new commands should be recorded (both tasks were replayed)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    // === Async Method Tests ===

    #[test]
    fn test_schedule_async_assigns_sequence_at_creation() {
        let ctx = create_test_context();

        // Create a future - sequence should be assigned immediately
        let future1 = ctx.schedule_raw("task-a", serde_json::json!({}));
        let future2 = ctx.schedule_raw("task-b", serde_json::json!({}));

        // Verify futures have different sequence numbers
        assert_eq!(future1.task_seq, 0);
        assert_eq!(future2.task_seq, 1);
    }

    #[test]
    fn test_schedule_async_multiple_tasks_get_sequential_ids() {
        let ctx = create_test_context();

        // Create multiple futures of the same type
        let futures: Vec<_> = (0..5)
            .map(|i| ctx.schedule_raw("same-task", serde_json::json!({"i": i})))
            .collect();

        // Each should have a unique, sequential sequence number
        for (i, f) in futures.iter().enumerate() {
            assert_eq!(f.task_seq, i as u32, "Future {} has wrong sequence", i);
        }
    }

    #[test]
    fn test_sleep_returns_timer_future() {
        let ctx = create_test_context();

        let timer = ctx.sleep(Duration::from_secs(60));

        // Verify it's a timer future with proper sequence
        assert_eq!(timer.timer_seq, 0);
        assert!(!timer.timer_id.is_empty());
    }

    #[test]
    fn test_async_methods_record_commands_immediately() {
        let ctx = create_test_context();

        // Commands should be empty initially
        assert!(ctx.get_commands().is_empty());

        // Create futures - commands should be recorded immediately
        let _task1 = ctx.schedule_raw("task-1", serde_json::json!({}));
        assert_eq!(ctx.get_commands().len(), 1);

        let _task2 = ctx.schedule_raw("task-2", serde_json::json!({}));
        assert_eq!(ctx.get_commands().len(), 2);

        let _timer = ctx.sleep(Duration::from_secs(30));
        assert_eq!(ctx.get_commands().len(), 3);

        // Verify command types
        let commands = ctx.get_commands();
        assert!(matches!(
            &commands[0],
            WorkflowCommand::ScheduleTask { kind, .. } if kind == "task-1"
        ));
        assert!(matches!(
            &commands[1],
            WorkflowCommand::ScheduleTask { kind, .. } if kind == "task-2"
        ));
        assert!(matches!(&commands[2], WorkflowCommand::StartTimer { .. }));
    }

    #[test]
    fn test_parallel_tasks_match_by_per_type_sequence() {
        use chrono::Utc;
        // Test that parallel tasks are matched by their per-type sequence number,
        // not by task type name alone

        let replay_events = vec![
            // Task(0) - first "task-a"
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "task-a",
                    "taskExecutionId": "exec-a-0"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "exec-a-0",
                    "output": {"result": "a0"}
                }),
                Utc::now(),
            ),
            // Task(1) - first "task-b"
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "task-b",
                    "taskExecutionId": "exec-b-0"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "exec-b-0",
                    "output": {"result": "b0"}
                }),
                Utc::now(),
            ),
            // Task(2) - second "task-a"
            ReplayEvent::new(
                5,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "task-a",
                    "taskExecutionId": "exec-a-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                6,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "exec-a-1",
                    "output": {"result": "a1"}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Create futures in parallel (same order as replay)
        let future_a0 = ctx.schedule_raw("task-a", serde_json::json!({}));
        let future_b0 = ctx.schedule_raw("task-b", serde_json::json!({}));
        let future_a1 = ctx.schedule_raw("task-a", serde_json::json!({}));

        // Verify sequence numbers
        assert_eq!(future_a0.task_seq, 0);
        assert_eq!(future_b0.task_seq, 1);
        assert_eq!(future_a1.task_seq, 2);
    }

    #[test]
    fn test_parallel_tasks_and_timer_independent_matching() {
        use chrono::Utc;
        // Tasks and timers should have independent sequence counters

        let replay_events = vec![
            // Task(0)
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "my-task",
                    "taskExecutionId": "task-0"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-0",
                    "output": {"value": 1}
                }),
                Utc::now(),
            ),
            // Timer(0)
            ReplayEvent::new(
                3,
                EventType::TimerStarted,
                serde_json::json!({
                    "timerId": "timer-0",
                    "durationMs": 1000
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TimerFired,
                serde_json::json!({
                    "timerId": "timer-0"
                }),
                Utc::now(),
            ),
            // Task(1) - another task after timer
            ReplayEvent::new(
                5,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "my-task",
                    "taskExecutionId": "task-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                6,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-1",
                    "output": {"value": 2}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Create futures in parallel
        let task0 = ctx.schedule_raw("my-task", serde_json::json!({}));
        let timer0 = ctx.sleep(Duration::from_secs(1));
        let task1 = ctx.schedule_raw("my-task", serde_json::json!({}));

        // Task sequence: 0, 1
        assert_eq!(task0.task_seq, 0);
        assert_eq!(task1.task_seq, 1);

        // Timer sequence: 0 (independent from tasks)
        assert_eq!(timer0.timer_seq, 0);
    }

    #[tokio::test]
    async fn test_parallel_completion_order_does_not_affect_replay() {
        use chrono::Utc;
        // Test that futures can be awaited in any order and still get correct results

        // Use valid UUIDs for task execution IDs
        let slow_exec_id = "11111111-1111-1111-1111-111111111111";
        let fast_exec_id = "22222222-2222-2222-2222-222222222222";

        let replay_events = vec![
            // Task(0) scheduled first, completes last
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "slow-task",
                    "taskExecutionId": slow_exec_id
                }),
                Utc::now(),
            ),
            // Task(1) scheduled second, completes first
            ReplayEvent::new(
                2,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "fast-task",
                    "taskExecutionId": fast_exec_id
                }),
                Utc::now(),
            ),
            // Fast task completes first
            ReplayEvent::new(
                3,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": fast_exec_id,
                    "output": {"result": "fast"}
                }),
                Utc::now(),
            ),
            // Slow task completes second
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": slow_exec_id,
                    "output": {"result": "slow"}
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Create futures
        let slow_future = ctx.schedule_raw("slow-task", serde_json::json!({}));
        let fast_future = ctx.schedule_raw("fast-task", serde_json::json!({}));

        // Await in different order than scheduled
        // Fast future first (even though slow was scheduled first)
        let fast_result = fast_future.await.unwrap();
        assert_eq!(fast_result, serde_json::json!({"result": "fast"}));

        // Slow future second
        let slow_result = slow_future.await.unwrap();
        assert_eq!(slow_result, serde_json::json!({"result": "slow"}));
    }

    #[tokio::test]
    async fn test_mixed_parallel_operations_replay_correctly() {
        use chrono::Utc;
        // Test mixing tasks, timers, and operations in parallel

        // Use valid UUID for task execution ID
        let fetch_exec_id = "33333333-3333-3333-3333-333333333333";

        let replay_events = vec![
            // Task(0)
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "kind": "fetch-data",
                    "taskExecutionId": fetch_exec_id
                }),
                Utc::now(),
            ),
            // Timer(0) - timer ID matches what sleep generates: "sleep-1"
            ReplayEvent::new(
                2,
                EventType::TimerStarted,
                serde_json::json!({
                    "timerId": "sleep-1",
                    "durationMs": 5000
                }),
                Utc::now(),
            ),
            // Operation(0)
            ReplayEvent::new(
                3,
                EventType::OperationCompleted,
                serde_json::json!({
                    "operationName": "compute",
                    "result": {"computed": true}
                }),
                Utc::now(),
            ),
            // Task completes
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": fetch_exec_id,
                    "output": {"data": "fetched"}
                }),
                Utc::now(),
            ),
            // Timer fires
            ReplayEvent::new(
                5,
                EventType::TimerFired,
                serde_json::json!({
                    "timerId": "sleep-1"
                }),
                Utc::now(),
            ),
        ];

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
        );

        // Create all futures in parallel
        let task = ctx.schedule_raw("fetch-data", serde_json::json!({}));
        let timer = ctx.sleep(Duration::from_secs(5));
        let op = ctx.run_raw("compute", serde_json::json!({"computed": true}));

        // Await in any order
        let op_result = op.await.unwrap();
        assert_eq!(op_result, serde_json::json!({"computed": true}));

        let task_result = task.await.unwrap();
        assert_eq!(task_result, serde_json::json!({"data": "fetched"}));

        let timer_result = timer.await;
        assert!(timer_result.is_ok());
    }
}
