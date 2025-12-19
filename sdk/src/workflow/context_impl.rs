//! WorkflowContextImpl - Concrete implementation of WorkflowContext

use crate::error::{DeterminismViolationError, FlovynError, Result};
use crate::generated::flovyn_v1::ExecutionSpan;
use crate::workflow::command::WorkflowCommand;
use crate::workflow::context::{DeterministicRandom, ScheduleTaskOptions, WorkflowContext};
use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::future::{
    ChildWorkflowFuture, ChildWorkflowFutureContext, ChildWorkflowFutureRaw, OperationFuture,
    OperationFutureRaw, PromiseFuture, PromiseFutureContext, PromiseFutureRaw, TaskFuture,
    TaskFutureContext, TaskFutureRaw, TimerFuture, TimerFutureContext,
};
use crate::workflow::recorder::CommandRecorder;
use crate::workflow::task_submitter::TaskSubmitter;
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use uuid::Uuid;

/// Seeded deterministic random number generator
pub struct SeededRandom {
    /// Current seed state
    state: RwLock<u64>,
}

impl SeededRandom {
    /// Create a new seeded random with the given seed
    pub fn new(seed: u64) -> Self {
        Self {
            state: RwLock::new(seed),
        }
    }

    /// Get next random u64 using xorshift64
    fn next_u64(&self) -> u64 {
        let mut state = self.state.write();
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }
}

impl DeterministicRandom for SeededRandom {
    fn next_int(&self, min: i32, max: i32) -> i32 {
        if min >= max {
            return min;
        }
        let range = (max - min) as u64;
        let random = self.next_u64();
        min + (random % range) as i32
    }

    fn next_long(&self, min: i64, max: i64) -> i64 {
        if min >= max {
            return min;
        }
        let range = (max - min) as u64;
        let random = self.next_u64();
        min + (random % range) as i64
    }

    fn next_double(&self) -> f64 {
        let random = self.next_u64();
        // Convert to f64 in range [0, 1)
        (random as f64) / (u64::MAX as f64)
    }

    fn next_bool(&self) -> bool {
        self.next_u64().is_multiple_of(2)
    }
}

/// Concrete implementation of WorkflowContext
pub struct WorkflowContextImpl<R: CommandRecorder> {
    /// Unique workflow execution ID
    workflow_execution_id: Uuid,

    /// Tenant ID
    tenant_id: Uuid,

    /// Workflow input
    input: Value,

    /// Command recorder for determinism validation
    recorder: RwLock<R>,

    /// Existing events from previous execution (for replay)
    existing_events: Vec<ReplayEvent>,

    /// Current sequence number (1-indexed)
    sequence_number: AtomicI32,

    /// Deterministic time (milliseconds since epoch)
    current_time: AtomicI64,

    /// UUID counter for deterministic UUID generation
    uuid_counter: AtomicI64,

    /// Seeded random number generator
    random: SeededRandom,

    /// Workflow state
    state: RwLock<HashMap<String, Value>>,

    /// Operation result cache (operation_name -> result)
    operation_cache: RwLock<HashMap<String, Value>>,

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

    // ============= Per-Type Event Lists for Sequence-Based Replay =============
    // Pre-filtered event lists for O(1) lookup by per-type index.
    // During replay, commands are matched to events by (type, per_type_index).
    /// TaskScheduled events only
    task_events: Vec<ReplayEvent>,
    /// ChildWorkflowInitiated events only
    child_workflow_events: Vec<ReplayEvent>,
    /// TimerStarted events only
    timer_events: Vec<ReplayEvent>,
    /// OperationCompleted events only
    operation_events: Vec<ReplayEvent>,
    /// PromiseCreated events only
    promise_events: Vec<ReplayEvent>,
    /// StateSet and StateCleared events only
    state_events: Vec<ReplayEvent>,

    // ============= Per-Type Sequence Counters =============
    // Counters for tracking which event to match next within each type.
    // Commands are matched by position in their type-specific event list.
    /// Counter for task scheduling (Task(0), Task(1), ...)
    next_task_seq: AtomicU32,
    /// Counter for child workflows (ChildWorkflow(0), ChildWorkflow(1), ...)
    next_child_workflow_seq: AtomicU32,
    /// Counter for timers (Timer(0), Timer(1), ...)
    next_timer_seq: AtomicU32,
    /// Counter for operations (Operation(0), Operation(1), ...)
    next_operation_seq: AtomicU32,
    /// Counter for promises (Promise(0), Promise(1), ...)
    next_promise_seq: AtomicU32,
    /// Counter for state operations (State(0), State(1), ...)
    next_state_seq: AtomicU32,

    /// Task submitter for submitting tasks to the server.
    /// If None, scheduling tasks will fail with an error.
    task_submitter: Option<Arc<dyn TaskSubmitter>>,

    /// Whether telemetry is enabled for span recording
    enable_telemetry: bool,

    /// Recorded execution spans (collected by worker after execution)
    recorded_spans: RwLock<Vec<ExecutionSpan>>,
}

impl<R: CommandRecorder> WorkflowContextImpl<R> {
    /// Create a new WorkflowContextImpl
    pub fn new(
        workflow_execution_id: Uuid,
        tenant_id: Uuid,
        input: Value,
        recorder: R,
        existing_events: Vec<ReplayEvent>,
        start_time_millis: i64,
    ) -> Self {
        Self::new_with_task_submitter(
            workflow_execution_id,
            tenant_id,
            input,
            recorder,
            existing_events,
            start_time_millis,
            None,
        )
    }

    /// Create a new WorkflowContextImpl with an optional task submitter
    pub fn new_with_task_submitter(
        workflow_execution_id: Uuid,
        tenant_id: Uuid,
        input: Value,
        recorder: R,
        existing_events: Vec<ReplayEvent>,
        start_time_millis: i64,
        task_submitter: Option<Arc<dyn TaskSubmitter>>,
    ) -> Self {
        Self::new_with_telemetry(
            workflow_execution_id,
            tenant_id,
            input,
            recorder,
            existing_events,
            start_time_millis,
            task_submitter,
            false, // telemetry disabled by default
        )
    }

    /// Create a new WorkflowContextImpl with task submitter and telemetry
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_telemetry(
        workflow_execution_id: Uuid,
        tenant_id: Uuid,
        input: Value,
        recorder: R,
        existing_events: Vec<ReplayEvent>,
        start_time_millis: i64,
        task_submitter: Option<Arc<dyn TaskSubmitter>>,
        enable_telemetry: bool,
    ) -> Self {
        // Create seed from workflow execution ID for deterministic randomness
        let seed = workflow_execution_id.as_u128() as u64;

        // Pre-populate operation cache from existing events
        let mut operation_cache = HashMap::new();
        let mut state = HashMap::new();

        for event in &existing_events {
            match event.event_type() {
                EventType::OperationCompleted => {
                    if let Some(name) = event.get_string("operationName") {
                        if let Some(result) = event.get("result") {
                            operation_cache.insert(name.to_string(), result.clone());
                        }
                    }
                }
                EventType::StateSet => {
                    if let Some(key) = event.get_string("key") {
                        if let Some(value) = event.get("value") {
                            state.insert(key.to_string(), value.clone());
                        }
                    }
                }
                EventType::StateCleared => {
                    if let Some(key) = event.get_string("key") {
                        state.remove(key);
                    }
                }
                _ => {}
            }
        }

        // Pre-filter events by type for O(1) lookup during replay
        // Each list contains events of a specific type, in sequence order
        let task_events: Vec<ReplayEvent> = existing_events
            .iter()
            .filter(|e| e.event_type() == EventType::TaskScheduled)
            .cloned()
            .collect();

        let child_workflow_events: Vec<ReplayEvent> = existing_events
            .iter()
            .filter(|e| e.event_type() == EventType::ChildWorkflowInitiated)
            .cloned()
            .collect();

        let timer_events: Vec<ReplayEvent> = existing_events
            .iter()
            .filter(|e| e.event_type() == EventType::TimerStarted)
            .cloned()
            .collect();

        let operation_events: Vec<ReplayEvent> = existing_events
            .iter()
            .filter(|e| e.event_type() == EventType::OperationCompleted)
            .cloned()
            .collect();

        let promise_events: Vec<ReplayEvent> = existing_events
            .iter()
            .filter(|e| e.event_type() == EventType::PromiseCreated)
            .cloned()
            .collect();

        let state_events: Vec<ReplayEvent> = existing_events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type(),
                    EventType::StateSet | EventType::StateCleared
                )
            })
            .cloned()
            .collect();

        // Start sequence number from the next available position
        // If there are existing events (replay), continue from after them
        let initial_sequence = (existing_events.len() as i32) + 1;

        Self {
            workflow_execution_id,
            tenant_id,
            input,
            recorder: RwLock::new(recorder),
            existing_events,
            sequence_number: AtomicI32::new(initial_sequence),
            current_time: AtomicI64::new(start_time_millis),
            uuid_counter: AtomicI64::new(0),
            random: SeededRandom::new(seed),
            state: RwLock::new(state),
            operation_cache: RwLock::new(operation_cache),
            cancellation_requested: AtomicBool::new(false),
            pending_tasks: RwLock::new(HashMap::new()),
            pending_promises: RwLock::new(HashMap::new()),
            pending_timers: RwLock::new(HashMap::new()),
            pending_child_workflows: RwLock::new(HashMap::new()),
            sleep_call_counter: AtomicI32::new(0),
            task_events,
            child_workflow_events,
            timer_events,
            operation_events,
            promise_events,
            state_events,
            next_task_seq: AtomicU32::new(0),
            next_child_workflow_seq: AtomicU32::new(0),
            next_timer_seq: AtomicU32::new(0),
            next_operation_seq: AtomicU32::new(0),
            next_promise_seq: AtomicU32::new(0),
            next_state_seq: AtomicU32::new(0),
            task_submitter,
            enable_telemetry,
            recorded_spans: RwLock::new(Vec::new()),
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

    /// Find terminal event (TaskCompleted or TaskFailed) for a task by taskExecutionId.
    /// Returns the latest terminal event if multiple exist (e.g., retries).
    fn find_terminal_task_event(&self, task_execution_id: &str) -> Option<&ReplayEvent> {
        self.existing_events
            .iter()
            .filter(|e| {
                let matches_id = e
                    .get_string("taskExecutionId")
                    .or_else(|| e.get_string("taskId"))
                    .map(|id| id == task_execution_id)
                    .unwrap_or(false);
                matches_id
                    && (e.event_type() == EventType::TaskCompleted
                        || e.event_type() == EventType::TaskFailed)
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Find terminal event (ChildWorkflowCompleted or ChildWorkflowFailed) for a child workflow by name.
    /// Returns the terminal event if it exists.
    fn find_terminal_child_workflow_event(&self, name: &str) -> Option<&ReplayEvent> {
        self.existing_events
            .iter()
            .filter(|e| {
                e.get_string("childExecutionName")
                    .map(|n| n == name)
                    .unwrap_or(false)
                    && (e.event_type() == EventType::ChildWorkflowCompleted
                        || e.event_type() == EventType::ChildWorkflowFailed)
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Find terminal event (TimerFired or TimerCancelled) for a timer by timerId.
    fn find_terminal_timer_event(&self, timer_id: &str) -> Option<&ReplayEvent> {
        self.existing_events
            .iter()
            .filter(|e| {
                e.get_string("timerId")
                    .map(|id| id == timer_id)
                    .unwrap_or(false)
                    && (e.event_type() == EventType::TimerFired
                        || e.event_type() == EventType::TimerCancelled)
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Find terminal event (PromiseResolved, PromiseRejected, or PromiseTimeout) for a promise.
    fn find_terminal_promise_event(&self, promise_name: &str) -> Option<&ReplayEvent> {
        self.existing_events
            .iter()
            .filter(|e| {
                // PromiseCreated uses promiseId, terminal events use promiseName
                let matches_name = e
                    .get_string("promiseName")
                    .or_else(|| e.get_string("promiseId"))
                    .map(|n| n == promise_name)
                    .unwrap_or(false);
                matches_name
                    && (e.event_type() == EventType::PromiseResolved
                        || e.event_type() == EventType::PromiseRejected
                        || e.event_type() == EventType::PromiseTimeout)
            })
            .max_by_key(|e| e.sequence_number())
    }

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
}

#[async_trait]
impl<R: CommandRecorder + Send + Sync> WorkflowContext for WorkflowContextImpl<R> {
    fn workflow_execution_id(&self) -> Uuid {
        self.workflow_execution_id
    }

    fn tenant_id(&self) -> Uuid {
        self.tenant_id
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

    async fn run_raw(&self, name: &str, result: Value) -> Result<Value> {
        // Get per-type sequence and increment atomically.
        // This assigns Operation(0), Operation(1), etc. to each run() call.
        let op_seq = self.next_operation_seq.fetch_add(1, Ordering::SeqCst) as usize;

        // Look for event at this per-type index
        if let Some(operation_event) = self.operation_events.get(op_seq) {
            // Validate operation name matches
            let event_op_name = operation_event
                .get_string("operationName")
                .unwrap_or_default()
                .to_string();

            if event_op_name != name {
                return Err(FlovynError::DeterminismViolation(
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

            return Ok(cached_result);
        }

        // No event at this per-type index → new operation
        // Record and cache the result
        {
            let mut cache = self.operation_cache.write();
            cache.insert(name.to_string(), result.clone());
        }

        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::RecordOperation {
            sequence_number: sequence,
            operation_name: name.to_string(),
            result: result.clone(),
        })?;

        // Record run.execute span for telemetry (only for new operations, not replay)
        if self.enable_telemetry {
            let span =
                crate::telemetry::run_execute_span(&self.workflow_execution_id.to_string(), name)
                    .finish();
            self.recorded_spans.write().push(span);
        }

        Ok(result)
    }

    async fn schedule_raw(&self, task_type: &str, input: Value) -> Result<Value> {
        self.schedule_with_options_raw(task_type, input, ScheduleTaskOptions::default())
            .await
    }

    async fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> Result<Value> {
        // Get per-type sequence and increment atomically.
        // This assigns Task(0), Task(1), etc. to each schedule() call.
        let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst) as usize;

        // Look for event at this per-type index
        if let Some(scheduled_event) = self.task_events.get(task_seq) {
            // Validate task type matches
            let event_task_type = scheduled_event
                .get_string("taskType")
                .unwrap_or_default()
                .to_string();

            if event_task_type != task_type {
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::TaskTypeMismatch {
                        sequence: task_seq as i32,
                        expected: event_task_type,
                        actual: task_type.to_string(),
                    },
                ));
            }

            // Get task execution ID from the scheduled event
            let task_execution_id = scheduled_event
                .get_string("taskExecutionId")
                .or_else(|| scheduled_event.get_string("taskId"))
                .ok_or_else(|| {
                    FlovynError::Other("TaskScheduled event missing taskExecutionId".to_string())
                })?
                .to_string();

            // Look for terminal event (completed/failed) for this task by ID
            let terminal_event = self.find_terminal_task_event(&task_execution_id);

            match terminal_event {
                Some(event) if event.event_type() == EventType::TaskCompleted => {
                    // Task completed - return result
                    let result = event
                        .get("result")
                        .or_else(|| event.get("output"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    return Ok(result);
                }
                Some(event) if event.event_type() == EventType::TaskFailed => {
                    // Task failed - return error
                    let error = event
                        .get_string("error")
                        .unwrap_or("Task failed")
                        .to_string();
                    return Err(FlovynError::TaskFailed(error));
                }
                _ => {
                    // Task scheduled but no terminal event - still running, suspend
                    return Err(FlovynError::Suspended {
                        reason: format!("Task is still running: {}", task_type),
                    });
                }
            }
        }

        // No event at this per-type index → new command, submit task
        let task_submitter = self.task_submitter.as_ref().ok_or_else(|| {
            FlovynError::Other(format!(
                "TaskSubmitter is not configured. Cannot schedule task '{}'",
                task_type
            ))
        })?;

        let submit_options = crate::workflow::task_submitter::TaskSubmitOptions {
            max_retries: options.max_retries.unwrap_or(3),
            timeout: options.timeout.unwrap_or(Duration::from_secs(300)),
            queue: options.queue.clone(),
            priority_seconds: options.priority_seconds,
        };

        let task_execution_id = task_submitter
            .submit_task(
                self.workflow_execution_id,
                self.tenant_id,
                task_type,
                input.clone(),
                submit_options,
            )
            .await?;

        // Record the command with the server-assigned task_execution_id
        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::ScheduleTask {
            sequence_number: sequence,
            task_type: task_type.to_string(),
            task_execution_id,
            input,
            priority_seconds: options.priority_seconds,
        })?;

        // Suspend workflow - will be resumed when task completes
        Err(FlovynError::Suspended {
            reason: format!("Task scheduled: {}", task_type),
        })
    }

    async fn get_raw(&self, key: &str) -> Result<Option<Value>> {
        let state = self.state.read();
        Ok(state.get(key).cloned())
    }

    async fn set_raw(&self, key: &str, value: Value) -> Result<()> {
        // Get per-type sequence and increment atomically.
        // This assigns State(0), State(1), etc. to each set()/clear() call.
        let state_seq = self.next_state_seq.fetch_add(1, Ordering::SeqCst) as usize;

        // Look for event at this per-type index
        if let Some(state_event) = self.state_events.get(state_seq) {
            // Validate event type is StateSet
            if state_event.event_type() != EventType::StateSet {
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::TypeMismatch {
                        sequence: state_seq as i32,
                        expected: EventType::StateSet,
                        actual: state_event.event_type(),
                    },
                ));
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

            // State was already set - update local state (it's already up to date from constructor)
            let mut state = self.state.write();
            state.insert(key.to_string(), value);
            return Ok(());
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::SetState {
            sequence_number: sequence,
            key: key.to_string(),
            value: value.clone(),
        })?;

        // Update state
        let mut state = self.state.write();
        state.insert(key.to_string(), value);
        Ok(())
    }

    async fn clear(&self, key: &str) -> Result<()> {
        // Get per-type sequence and increment atomically.
        // This assigns State(0), State(1), etc. to each set()/clear() call.
        let state_seq = self.next_state_seq.fetch_add(1, Ordering::SeqCst) as usize;

        // Look for event at this per-type index
        if let Some(state_event) = self.state_events.get(state_seq) {
            // Validate event type is StateCleared
            if state_event.event_type() != EventType::StateCleared {
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::TypeMismatch {
                        sequence: state_seq as i32,
                        expected: EventType::StateCleared,
                        actual: state_event.event_type(),
                    },
                ));
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

            // State was already cleared - update local state
            let mut state = self.state.write();
            state.remove(key);
            return Ok(());
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::ClearState {
            sequence_number: sequence,
            key: key.to_string(),
        })?;

        // Update state
        let mut state = self.state.write();
        state.remove(key);
        Ok(())
    }

    async fn clear_all(&self) -> Result<()> {
        // Clear all keys by clearing each one
        let keys: Vec<String> = {
            let state = self.state.read();
            state.keys().cloned().collect()
        };

        for key in keys {
            self.clear(&key).await?;
        }
        Ok(())
    }

    async fn state_keys(&self) -> Result<Vec<String>> {
        let state = self.state.read();
        Ok(state.keys().cloned().collect())
    }

    async fn sleep(&self, duration: Duration) -> Result<()> {
        // Get per-type sequence and increment atomically.
        // This assigns Timer(0), Timer(1), etc. to each sleep() call.
        let timer_seq = self.next_timer_seq.fetch_add(1, Ordering::SeqCst) as usize;

        // Generate deterministic timer ID using dedicated counter.
        // This counter is separate from sequence_number to ensure timer IDs are
        // consistent across replays (sequence depends on existingEvents.size).
        let sleep_count = self.sleep_call_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let timer_id = format!("sleep-{}", sleep_count);
        let duration_ms = duration.as_millis() as i64;

        // Look for event at this per-type index
        if let Some(started_event) = self.timer_events.get(timer_seq) {
            // Validate timer ID matches
            let event_timer_id = started_event
                .get_string("timerId")
                .unwrap_or_default()
                .to_string();

            if event_timer_id != timer_id {
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::TimerIdMismatch {
                        sequence: timer_seq as i32,
                        expected: event_timer_id,
                        actual: timer_id,
                    },
                ));
            }

            // Look for terminal event (TimerFired or TimerCancelled) for this timer
            let terminal_event = self.find_terminal_timer_event(&event_timer_id);

            match terminal_event {
                Some(event) if event.event_type() == EventType::TimerFired => {
                    // Timer already fired - return immediately
                    return Ok(());
                }
                Some(event) if event.event_type() == EventType::TimerCancelled => {
                    // Timer was cancelled - return error
                    return Err(FlovynError::TimerError(format!(
                        "Timer '{}' was cancelled",
                        event_timer_id
                    )));
                }
                _ => {
                    // Timer started but no terminal event - still waiting, suspend
                    return Err(FlovynError::Suspended {
                        reason: format!("Waiting for timer: {}", event_timer_id),
                    });
                }
            }
        }

        // No event at this per-type index → new command
        // Record START_TIMER command
        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::StartTimer {
            sequence_number: sequence,
            timer_id: timer_id.clone(),
            duration_ms,
        })?;

        // Suspend workflow execution (will be resumed when timer fires)
        Err(FlovynError::Suspended {
            reason: format!("Waiting for timer: {}", timer_id),
        })
    }

    async fn promise_raw(&self, name: &str) -> Result<Value> {
        self.promise_with_timeout_raw(name, Duration::from_secs(u64::MAX))
            .await
    }

    async fn promise_with_timeout_raw(&self, name: &str, timeout: Duration) -> Result<Value> {
        // Get per-type sequence and increment atomically.
        // This assigns Promise(0), Promise(1), etc. to each promise() call.
        let promise_seq = self.next_promise_seq.fetch_add(1, Ordering::SeqCst) as usize;

        let timeout_ms = if timeout.as_secs() == u64::MAX {
            None
        } else {
            Some(timeout.as_millis() as i64)
        };

        // Look for event at this per-type index
        if let Some(created_event) = self.promise_events.get(promise_seq) {
            // Validate promise name/ID matches
            let event_promise_id = created_event
                .get_string("promiseId")
                .or_else(|| created_event.get_string("promiseName"))
                .unwrap_or_default()
                .to_string();

            if event_promise_id != name {
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::PromiseNameMismatch {
                        sequence: promise_seq as i32,
                        expected: event_promise_id,
                        actual: name.to_string(),
                    },
                ));
            }

            // Look for terminal event (resolved/rejected/timeout) for this promise
            let terminal_event = self.find_terminal_promise_event(&event_promise_id);

            match terminal_event {
                Some(event) if event.event_type() == EventType::PromiseResolved => {
                    // Promise resolved - return value
                    let value = event.get("value").cloned().unwrap_or(Value::Null);
                    return Ok(value);
                }
                Some(event) if event.event_type() == EventType::PromiseRejected => {
                    // Promise rejected - return error
                    let error = event
                        .get_string("error")
                        .unwrap_or("Promise rejected")
                        .to_string();
                    return Err(FlovynError::PromiseRejected {
                        name: name.to_string(),
                        error,
                    });
                }
                Some(event) if event.event_type() == EventType::PromiseTimeout => {
                    // Promise timed out - return error
                    return Err(FlovynError::PromiseTimeout {
                        name: name.to_string(),
                    });
                }
                _ => {
                    // Promise created but no terminal event - still waiting, suspend
                    return Err(FlovynError::Suspended {
                        reason: format!("Waiting for promise: {}", name),
                    });
                }
            }
        }

        // No event at this per-type index → new command
        // Record CREATE_PROMISE command
        let sequence = self.next_sequence();
        self.record_command(WorkflowCommand::CreatePromise {
            sequence_number: sequence,
            promise_id: name.to_string(),
            timeout_ms,
        })?;

        // Check if already resolved in pending_promises (for in-process testing)
        {
            let pending = self.pending_promises.read();
            if let Some(value) = pending.get(name) {
                return Ok(value.clone());
            }
        }

        // Promise not yet resolved - suspend
        Err(FlovynError::Suspended {
            reason: format!("Waiting for promise: {}", name),
        })
    }

    async fn schedule_workflow_raw(&self, name: &str, kind: &str, input: Value) -> Result<Value> {
        // Get per-type sequence and increment atomically.
        // This assigns ChildWorkflow(0), ChildWorkflow(1), etc. to each schedule_workflow() call.
        let cw_seq = self.next_child_workflow_seq.fetch_add(1, Ordering::SeqCst) as usize;

        // Look for event at this per-type index
        if let Some(initiated_event) = self.child_workflow_events.get(cw_seq) {
            // Validate child workflow name matches
            let event_name = initiated_event
                .get_string("childExecutionName")
                .unwrap_or_default()
                .to_string();

            if event_name != name {
                return Err(FlovynError::DeterminismViolation(
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
                return Err(FlovynError::DeterminismViolation(
                    DeterminismViolationError::ChildWorkflowMismatch {
                        sequence: cw_seq as i32,
                        field: "kind".to_string(),
                        expected: event_kind,
                        actual: kind.to_string(),
                    },
                ));
            }

            // Look for terminal event (completed/failed) for this child workflow by name
            let terminal_event = self.find_terminal_child_workflow_event(&event_name);

            match terminal_event {
                Some(event) if event.event_type() == EventType::ChildWorkflowCompleted => {
                    // Child workflow completed - return result
                    let result = event.get("output").cloned().unwrap_or(Value::Null);
                    return Ok(result);
                }
                Some(event) if event.event_type() == EventType::ChildWorkflowFailed => {
                    // Child workflow failed - return error
                    let error = event
                        .get_string("error")
                        .unwrap_or("Child workflow failed")
                        .to_string();
                    let execution_id = event
                        .get_string("childworkflowExecutionId")
                        .unwrap_or("unknown")
                        .to_string();
                    return Err(FlovynError::ChildWorkflowFailed {
                        execution_id,
                        name: name.to_string(),
                        error,
                    });
                }
                _ => {
                    // Child workflow initiated but no terminal event - still running, suspend
                    return Err(FlovynError::Suspended {
                        reason: format!("Waiting for child workflow: {}", name),
                    });
                }
            }
        }

        // No event at this per-type index → new command
        // Record SCHEDULE_CHILD_WORKFLOW command
        let sequence = self.next_sequence();
        let child_execution_id = self.random_uuid();
        self.record_command(WorkflowCommand::ScheduleChildWorkflow {
            sequence_number: sequence,
            name: name.to_string(),
            kind: Some(kind.to_string()),
            definition_id: None,
            child_execution_id,
            input,
            task_queue: String::new(), // Empty = inherit from parent
            priority_seconds: 0,
        })?;

        // Check if already resolved in pending_child_workflows (for in-process testing)
        {
            let pending = self.pending_child_workflows.read();
            if let Some(result) = pending.get(name) {
                return Ok(result.clone());
            }
        }

        // Child workflow not yet completed - suspend
        Err(FlovynError::Suspended {
            reason: format!("Waiting for child workflow: {}", name),
        })
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
    // Async Operation Methods for Parallel Execution
    // =========================================================================

    fn schedule_async_raw(&self, task_type: &str, input: Value) -> TaskFutureRaw {
        self.schedule_async_with_options_raw(task_type, input, ScheduleTaskOptions::default())
    }

    fn schedule_async_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> TaskFutureRaw {
        // Get per-type sequence and increment atomically.
        let task_seq = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

        // Look for event at this per-type index (replay case)
        if let Some(scheduled_event) = self.task_events.get(task_seq as usize) {
            // Validate task type matches
            let event_task_type = scheduled_event
                .get_string("taskType")
                .unwrap_or_default()
                .to_string();

            if event_task_type != task_type {
                return TaskFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::TaskTypeMismatch {
                        sequence: task_seq as i32,
                        expected: event_task_type,
                        actual: task_type.to_string(),
                    },
                ));
            }

            // Get task execution ID from the scheduled event
            let task_execution_id = scheduled_event
                .get_string("taskExecutionId")
                .or_else(|| scheduled_event.get_string("taskId"))
                .map(|s| Uuid::parse_str(s).unwrap_or(Uuid::nil()))
                .unwrap_or(Uuid::nil());

            // Look for terminal event (completed/failed) for this task
            if let Some(terminal_event) =
                self.find_terminal_task_event(&task_execution_id.to_string())
            {
                if terminal_event.event_type() == EventType::TaskCompleted {
                    let result = terminal_event
                        .get("result")
                        .or_else(|| terminal_event.get("output"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    return TaskFuture::from_replay(
                        task_seq,
                        task_execution_id,
                        Weak::<DummyTaskFutureContext>::new(),
                        Ok(result),
                    );
                } else {
                    let error = terminal_event
                        .get_string("error")
                        .unwrap_or("Task failed")
                        .to_string();
                    return TaskFuture::from_replay(
                        task_seq,
                        task_execution_id,
                        Weak::<DummyTaskFutureContext>::new(),
                        Err(FlovynError::TaskFailed(error)),
                    );
                }
            }

            // Task scheduled but not completed yet - return pending future
            return TaskFuture::new(
                task_seq,
                task_execution_id,
                Weak::<DummyTaskFutureContext>::new(),
            );
        }

        // No event at this per-type index → new command
        // Generate task execution ID
        let task_execution_id = self.random_uuid();

        // Record the command
        let sequence = self.next_sequence();
        if let Err(e) = self.record_command(WorkflowCommand::ScheduleTask {
            sequence_number: sequence,
            task_type: task_type.to_string(),
            task_execution_id,
            input,
            priority_seconds: options.priority_seconds,
        }) {
            return TaskFuture::with_error(e);
        }

        // Return pending future
        TaskFuture::new(
            task_seq,
            task_execution_id,
            Weak::<DummyTaskFutureContext>::new(),
        )
    }

    fn sleep_async(&self, duration: Duration) -> TimerFuture {
        // Get per-type sequence and increment atomically.
        let timer_seq = self.next_timer_seq.fetch_add(1, Ordering::SeqCst);

        // Generate deterministic timer ID
        let sleep_count = self.sleep_call_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let timer_id = format!("sleep-{}", sleep_count);
        let duration_ms = duration.as_millis() as i64;

        // Look for event at this per-type index (replay case)
        if let Some(started_event) = self.timer_events.get(timer_seq as usize) {
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
            if let Some(terminal_event) = self.find_terminal_timer_event(&event_timer_id) {
                if terminal_event.event_type() == EventType::TimerFired {
                    return TimerFuture::from_replay(
                        timer_seq,
                        event_timer_id,
                        Weak::<DummyTimerFutureContext>::new(),
                        true,
                    );
                } else {
                    return TimerFuture::from_replay(
                        timer_seq,
                        event_timer_id,
                        Weak::<DummyTimerFutureContext>::new(),
                        false,
                    );
                }
            }

            // Timer started but not fired yet - return pending future
            return TimerFuture::new(
                timer_seq,
                event_timer_id,
                Weak::<DummyTimerFutureContext>::new(),
            );
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
        TimerFuture::new(timer_seq, timer_id, Weak::<DummyTimerFutureContext>::new())
    }

    fn schedule_workflow_async_raw(
        &self,
        name: &str,
        kind: &str,
        input: Value,
    ) -> ChildWorkflowFutureRaw {
        // Get per-type sequence and increment atomically.
        let cw_seq = self.next_child_workflow_seq.fetch_add(1, Ordering::SeqCst);

        // Look for event at this per-type index (replay case)
        if let Some(initiated_event) = self.child_workflow_events.get(cw_seq as usize) {
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

            // Look for terminal event (completed/failed) for this child workflow
            if let Some(terminal_event) = self.find_terminal_child_workflow_event(&event_name) {
                if terminal_event.event_type() == EventType::ChildWorkflowCompleted {
                    let result = terminal_event.get("output").cloned().unwrap_or(Value::Null);
                    return ChildWorkflowFuture::from_replay(
                        cw_seq,
                        child_execution_id,
                        event_name,
                        Weak::<DummyChildWorkflowFutureContext>::new(),
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
                    return ChildWorkflowFuture::from_replay(
                        cw_seq,
                        child_execution_id,
                        name.to_string(),
                        Weak::<DummyChildWorkflowFutureContext>::new(),
                        Err(FlovynError::ChildWorkflowFailed {
                            execution_id: exec_id,
                            name: name.to_string(),
                            error,
                        }),
                    );
                }
            }

            // Child workflow initiated but not completed yet - return pending future
            return ChildWorkflowFuture::new(
                cw_seq,
                child_execution_id,
                event_name,
                Weak::<DummyChildWorkflowFutureContext>::new(),
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
            task_queue: String::new(),
            priority_seconds: 0,
        }) {
            return ChildWorkflowFuture::with_error(e);
        }

        // Return pending future
        ChildWorkflowFuture::new(
            cw_seq,
            child_execution_id,
            name.to_string(),
            Weak::<DummyChildWorkflowFutureContext>::new(),
        )
    }

    fn promise_async_raw(&self, name: &str) -> PromiseFutureRaw {
        // Get per-type sequence and increment atomically.
        let promise_seq = self.next_promise_seq.fetch_add(1, Ordering::SeqCst);

        // Look for event at this per-type index (replay case)
        if let Some(created_event) = self.promise_events.get(promise_seq as usize) {
            // Validate promise name/ID matches
            let event_promise_id = created_event
                .get_string("promiseId")
                .or_else(|| created_event.get_string("promiseName"))
                .unwrap_or_default()
                .to_string();

            if event_promise_id != name {
                return PromiseFuture::with_error(FlovynError::DeterminismViolation(
                    DeterminismViolationError::PromiseNameMismatch {
                        sequence: promise_seq as i32,
                        expected: event_promise_id,
                        actual: name.to_string(),
                    },
                ));
            }

            // Look for terminal event (resolved/rejected/timeout)
            if let Some(terminal_event) = self.find_terminal_promise_event(&event_promise_id) {
                match terminal_event.event_type() {
                    EventType::PromiseResolved => {
                        let value = terminal_event.get("value").cloned().unwrap_or(Value::Null);
                        return PromiseFuture::from_replay(
                            promise_seq,
                            event_promise_id,
                            Weak::<DummyPromiseFutureContext>::new(),
                            Ok(value),
                        );
                    }
                    EventType::PromiseRejected => {
                        let error = terminal_event
                            .get_string("error")
                            .unwrap_or("Promise rejected")
                            .to_string();
                        return PromiseFuture::from_replay(
                            promise_seq,
                            event_promise_id,
                            Weak::<DummyPromiseFutureContext>::new(),
                            Err(FlovynError::PromiseRejected {
                                name: name.to_string(),
                                error,
                            }),
                        );
                    }
                    EventType::PromiseTimeout => {
                        return PromiseFuture::from_replay(
                            promise_seq,
                            event_promise_id,
                            Weak::<DummyPromiseFutureContext>::new(),
                            Err(FlovynError::PromiseTimeout {
                                name: name.to_string(),
                            }),
                        );
                    }
                    _ => {}
                }
            }

            // Promise created but not resolved yet - return pending future
            return PromiseFuture::new(
                promise_seq,
                event_promise_id,
                Weak::<DummyPromiseFutureContext>::new(),
            );
        }

        // No event at this per-type index → new command
        let sequence = self.next_sequence();
        if let Err(e) = self.record_command(WorkflowCommand::CreatePromise {
            sequence_number: sequence,
            promise_id: name.to_string(),
            timeout_ms: None,
        }) {
            return PromiseFuture::with_error(e);
        }

        // Return pending future
        PromiseFuture::new(
            promise_seq,
            name.to_string(),
            Weak::<DummyPromiseFutureContext>::new(),
        )
    }

    fn run_async_raw(&self, name: &str, result: Value) -> OperationFutureRaw {
        // Get per-type sequence and increment atomically.
        let op_seq = self.next_operation_seq.fetch_add(1, Ordering::SeqCst);

        // Look for event at this per-type index (replay case)
        if let Some(operation_event) = self.operation_events.get(op_seq as usize) {
            // Validate operation name matches
            let event_op_name = operation_event
                .get_string("operationName")
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
        // Record and cache the result
        {
            let mut cache = self.operation_cache.write();
            cache.insert(name.to_string(), result.clone());
        }

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
struct DummyTaskFutureContext;
impl TaskFutureContext for DummyTaskFutureContext {
    fn find_task_result(&self, _: &Uuid) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_task(&self, _: &Uuid) {}
}

struct DummyTimerFutureContext;
impl TimerFutureContext for DummyTimerFutureContext {
    fn find_timer_result(&self, _: &str) -> Option<Result<()>> {
        None
    }
    fn record_cancel_timer(&self, _: &str) {}
}

struct DummyChildWorkflowFutureContext;
impl ChildWorkflowFutureContext for DummyChildWorkflowFutureContext {
    fn find_child_workflow_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_child_workflow(&self, _: &Uuid) {}
}

struct DummyPromiseFutureContext;
impl PromiseFutureContext for DummyPromiseFutureContext {
    fn find_promise_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::recorder::CommandCollector;
    use crate::workflow::task_submitter::{TaskSubmitOptions, TaskSubmitter};

    /// Mock task submitter that always returns a fixed task execution ID
    struct MockTaskSubmitter;

    #[async_trait]
    impl TaskSubmitter for MockTaskSubmitter {
        async fn submit_task(
            &self,
            _workflow_execution_id: Uuid,
            _tenant_id: Uuid,
            _task_type: &str,
            _input: Value,
            _options: TaskSubmitOptions,
        ) -> Result<Uuid> {
            Ok(Uuid::new_v4())
        }
    }

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

    fn create_test_context_with_task_submitter() -> WorkflowContextImpl<CommandCollector> {
        WorkflowContextImpl::new_with_task_submitter(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({"test": true}),
            CommandCollector::new(),
            vec![],
            1700000000000, // Fixed timestamp for tests
            Some(Arc::new(MockTaskSubmitter)),
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
    fn test_tenant_id() {
        let tenant_id = Uuid::new_v4();
        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            tenant_id,
            serde_json::json!({}),
            CommandCollector::new(),
            vec![],
            0,
        );
        assert_eq!(ctx.tenant_id(), tenant_id);
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

    #[tokio::test]
    async fn test_schedule_raw_suspends() {
        // Use context with task submitter since schedule_raw now submits tasks via gRPC
        let ctx = create_test_context_with_task_submitter();

        let result = ctx
            .schedule_raw("my-task", serde_json::json!({"input": true}))
            .await;

        assert!(matches!(result, Err(FlovynError::Suspended { .. })));

        // Verify command was recorded
        let commands = ctx.get_commands();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            WorkflowCommand::ScheduleTask { task_type, .. } if task_type == "my-task"
        ));
    }

    #[tokio::test]
    async fn test_schedule_raw_returns_completed_from_replay() {
        use chrono::Utc;
        // Create context with replay events showing task was already scheduled and completed
        let task_execution_id = "test-task-123";
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "my-task",
                    "taskExecutionId": task_execution_id,
                    "input": {"input": true}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": task_execution_id,
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

    #[tokio::test]
    async fn test_sleep_suspends() {
        let ctx = create_test_context();

        let result = ctx.sleep(Duration::from_secs(60)).await;

        assert!(matches!(result, Err(FlovynError::Suspended { .. })));
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

        let replay_events = vec![
            // First task scheduled and completed
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "fast-task",
                    "taskExecutionId": "task-111",
                    "input": {"seq": 1}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-111",
                    "output": {"result": "first"}
                }),
                Utc::now(),
            ),
            // Second task scheduled and completed
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "fast-task",
                    "taskExecutionId": "task-222",
                    "input": {"seq": 2}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-222",
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

    #[tokio::test]
    async fn test_task_still_running_suspends() {
        use chrono::Utc;
        // Test that when a task is scheduled but not yet completed, workflow suspends
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "slow-task",
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

        let result = ctx.schedule_raw("slow-task", serde_json::json!({})).await;
        assert!(
            matches!(result, Err(FlovynError::Suspended { reason }) if reason.contains("still running"))
        );
    }

    #[tokio::test]
    async fn test_task_failed_returns_error() {
        use chrono::Utc;
        // Test that when a task fails, the error is propagated
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "failing-task",
                    "taskExecutionId": "task-failed",
                    "input": {}
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskFailed,
                serde_json::json!({
                    "taskExecutionId": "task-failed",
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

    #[tokio::test]
    async fn test_promise_suspends() {
        let ctx = create_test_context();

        let result = ctx.promise_raw("my-promise").await;

        assert!(matches!(result, Err(FlovynError::Suspended { .. })));
    }

    #[tokio::test]
    async fn test_promise_returns_resolved() {
        let ctx = create_test_context();

        // Pre-resolve the promise
        ctx.resolve_promise("my-promise", serde_json::json!({"approved": true}));

        let result = ctx.promise_raw("my-promise").await.unwrap();

        assert_eq!(result, serde_json::json!({"approved": true}));
    }

    #[tokio::test]
    async fn test_promise_returns_immediately_when_resolved_during_replay() {
        use chrono::Utc;
        // Create context with replay events showing promise was already created and resolved
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                serde_json::json!({
                    "promiseId": "user-approval",
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseResolved,
                serde_json::json!({
                    "promiseName": "user-approval",
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
        // Create context with replay events showing promise was created and rejected
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                serde_json::json!({
                    "promiseId": "user-approval",
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseRejected,
                serde_json::json!({
                    "promiseName": "user-approval",
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

    #[tokio::test]
    async fn test_promise_suspends_when_created_but_not_resolved_during_replay() {
        use chrono::Utc;
        // Create context with replay events showing promise was created but not resolved
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                serde_json::json!({
                    "promiseId": "user-approval",
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

        // Since the promise was created but not resolved, should suspend without recording new command
        let result = ctx.promise_raw("user-approval").await;
        assert!(
            matches!(result, Err(FlovynError::Suspended { reason }) if reason.contains("user-approval"))
        );

        // No command should be recorded (promise was already created)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
    }

    #[tokio::test]
    async fn test_schedule_workflow_suspends() {
        let ctx = create_test_context();

        let result = ctx
            .schedule_workflow_raw("child-1", "payment-workflow", serde_json::json!({}))
            .await;

        assert!(matches!(result, Err(FlovynError::Suspended { .. })));
    }

    #[tokio::test]
    async fn test_schedule_workflow_returns_resolved() {
        let ctx = create_test_context();

        // Pre-resolve the child workflow
        ctx.resolve_child_workflow("child-1", serde_json::json!({"completed": true}));

        let result = ctx
            .schedule_workflow_raw("child-1", "payment-workflow", serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(result, serde_json::json!({"completed": true}));
    }

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

    #[tokio::test]
    async fn test_schedule_workflow_suspends_when_initiated_but_not_completed_during_replay() {
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

        // Since the child workflow was initiated but not completed, should suspend without recording new command
        let result = ctx
            .schedule_workflow_raw("payment-child", "payment-workflow", serde_json::json!({}))
            .await;
        assert!(
            matches!(result, Err(FlovynError::Suspended { reason }) if reason.contains("payment-child"))
        );

        // No command should be recorded (child workflow was already initiated)
        let commands = ctx.get_commands();
        assert!(commands.is_empty());
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
                "taskType": "send-email",
                "taskExecutionId": "task-abc-123"
            }),
            Utc::now(),
        )];

        let ctx: WorkflowContextImpl<CommandCollector> =
            WorkflowContextImpl::new_with_task_submitter(
                Uuid::new_v4(),
                Uuid::new_v4(),
                serde_json::json!({}),
                CommandCollector::new(),
                replay_events,
                1700000000000,
                Some(Arc::new(MockTaskSubmitter)),
            );

        // Try to schedule a different task type - should get determinism violation
        let result = ctx
            .schedule_raw("process-payment", serde_json::json!({}))
            .await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::TaskTypeMismatch { .. }
            ))
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

        // Try to run a different operation - should get determinism violation
        let result = ctx.run_raw("calculate-total", serde_json::json!(100)).await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::OperationNameMismatch { .. }
            ))
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

        // Try to sleep - should get determinism violation because timer ID doesn't match
        let result = ctx.sleep(Duration::from_secs(5)).await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::TimerIdMismatch { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_promise_name_mismatch_violation() {
        use chrono::Utc;
        // Create context with replay event for a different promise name
        let replay_events = vec![ReplayEvent::new(
            1,
            EventType::PromiseCreated,
            serde_json::json!({
                "promiseId": "approval-promise"
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

        // Try to create a different promise - should get determinism violation
        let result = ctx.promise_raw("payment-confirmation").await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::PromiseNameMismatch { .. }
            ))
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

        // Try to schedule a different child workflow - should get determinism violation
        let result = ctx
            .schedule_workflow_raw("payment-processor", "payment-wf", serde_json::json!({}))
            .await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::ChildWorkflowMismatch { .. }
            ))
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

        // Try to set a different key - should get determinism violation
        let result = ctx.set_raw("order-id", serde_json::json!(456)).await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::StateKeyMismatch { .. }
            ))
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
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "task-A",
                    "taskExecutionId": "task-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-1",
                    "result": "A-result-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "task-B",
                    "taskExecutionId": "task-2"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-2",
                    "result": "B-result-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                5,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "task-A",
                    "taskExecutionId": "task-3"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                6,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-3",
                    "result": "A-result-2"
                }),
                Utc::now(),
            ),
        ];

        let ctx: WorkflowContextImpl<CommandCollector> =
            WorkflowContextImpl::new_with_task_submitter(
                Uuid::new_v4(),
                Uuid::new_v4(),
                serde_json::json!({}),
                CommandCollector::new(),
                replay_events,
                1700000000000,
                Some(Arc::new(MockTaskSubmitter)),
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
                    "taskType": "task-A",
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

        let ctx: WorkflowContextImpl<CommandCollector> =
            WorkflowContextImpl::new_with_task_submitter(
                Uuid::new_v4(),
                Uuid::new_v4(),
                serde_json::json!({}),
                CommandCollector::new(),
                replay_events,
                1700000000000,
                Some(Arc::new(MockTaskSubmitter)),
            );

        // Try to schedule task-B first (wrong order) - should fail
        let result = ctx.schedule_raw("task-B", serde_json::json!({})).await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::TaskTypeMismatch { .. }
            ))
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

        // Try to schedule with different kind - should get determinism violation
        let result = ctx
            .schedule_workflow_raw(
                "payment-child",
                "payment-workflow-v2",
                serde_json::json!({}),
            )
            .await;

        assert!(matches!(
            result,
            Err(FlovynError::DeterminismViolation(
                crate::error::DeterminismViolationError::ChildWorkflowMismatch { field, .. }
            )) if field == "kind"
        ));
    }

    #[tokio::test]
    async fn test_sequence_based_task_matches_at_correct_sequence() {
        use chrono::Utc;
        // Create context with task that completed successfully
        let replay_events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "process-order",
                    "taskExecutionId": "task-abc"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-abc",
                    "result": {"orderId": 12345, "status": "processed"}
                }),
                Utc::now(),
            ),
        ];

        let ctx: WorkflowContextImpl<CommandCollector> =
            WorkflowContextImpl::new_with_task_submitter(
                Uuid::new_v4(),
                Uuid::new_v4(),
                serde_json::json!({}),
                CommandCollector::new(),
                replay_events,
                1700000000000,
                Some(Arc::new(MockTaskSubmitter)),
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
        let replay_events = vec![
            // Task iteration 1
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "process-item",
                    "taskExecutionId": "task-1"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-1",
                    "result": {"value": 1}
                }),
                Utc::now(),
            ),
            // Task iteration 2
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "process-item",
                    "taskExecutionId": "task-2"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-2",
                    "result": {"value": 2}
                }),
                Utc::now(),
            ),
            // Task iteration 3 (will be orphaned - never accessed)
            ReplayEvent::new(
                5,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "process-item",
                    "taskExecutionId": "task-3"
                }),
                Utc::now(),
            ),
            ReplayEvent::new(
                6,
                EventType::TaskCompleted,
                serde_json::json!({
                    "taskExecutionId": "task-3",
                    "result": {"value": 3}
                }),
                Utc::now(),
            ),
        ];

        let ctx: WorkflowContextImpl<CommandCollector> =
            WorkflowContextImpl::new_with_task_submitter(
                Uuid::new_v4(),
                Uuid::new_v4(),
                serde_json::json!({}),
                CommandCollector::new(),
                replay_events,
                1700000000000,
                Some(Arc::new(MockTaskSubmitter)),
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
        let ctx = create_test_context_with_task_submitter();

        // Create a future - sequence should be assigned immediately
        let future1 = ctx.schedule_async_raw("task-a", serde_json::json!({}));
        let future2 = ctx.schedule_async_raw("task-b", serde_json::json!({}));

        // Verify futures have different sequence numbers
        assert_eq!(future1.task_seq, 0);
        assert_eq!(future2.task_seq, 1);
    }

    #[test]
    fn test_schedule_async_multiple_tasks_get_sequential_ids() {
        let ctx = create_test_context_with_task_submitter();

        // Create multiple futures of the same type
        let futures: Vec<_> = (0..5)
            .map(|i| ctx.schedule_async_raw("same-task", serde_json::json!({"i": i})))
            .collect();

        // Each should have a unique, sequential sequence number
        for (i, f) in futures.iter().enumerate() {
            assert_eq!(f.task_seq, i as u32, "Future {} has wrong sequence", i);
        }
    }

    #[test]
    fn test_sleep_async_returns_timer_future() {
        let ctx = create_test_context();

        let timer = ctx.sleep_async(Duration::from_secs(60));

        // Verify it's a timer future with proper sequence
        assert_eq!(timer.timer_seq, 0);
        assert!(!timer.timer_id.is_empty());
    }

    #[test]
    fn test_async_methods_record_commands_immediately() {
        let ctx = create_test_context_with_task_submitter();

        // Commands should be empty initially
        assert!(ctx.get_commands().is_empty());

        // Create futures - commands should be recorded immediately
        let _task1 = ctx.schedule_async_raw("task-1", serde_json::json!({}));
        assert_eq!(ctx.get_commands().len(), 1);

        let _task2 = ctx.schedule_async_raw("task-2", serde_json::json!({}));
        assert_eq!(ctx.get_commands().len(), 2);

        let _timer = ctx.sleep_async(Duration::from_secs(30));
        assert_eq!(ctx.get_commands().len(), 3);

        // Verify command types
        let commands = ctx.get_commands();
        assert!(matches!(
            &commands[0],
            WorkflowCommand::ScheduleTask { task_type, .. } if task_type == "task-1"
        ));
        assert!(matches!(
            &commands[1],
            WorkflowCommand::ScheduleTask { task_type, .. } if task_type == "task-2"
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
                    "taskType": "task-a",
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
                    "taskType": "task-b",
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
                    "taskType": "task-a",
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

        let ctx = WorkflowContextImpl::new_with_task_submitter(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
            Some(Arc::new(MockTaskSubmitter)),
        );

        // Create futures in parallel (same order as replay)
        let future_a0 = ctx.schedule_async_raw("task-a", serde_json::json!({}));
        let future_b0 = ctx.schedule_async_raw("task-b", serde_json::json!({}));
        let future_a1 = ctx.schedule_async_raw("task-a", serde_json::json!({}));

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
                    "taskType": "my-task",
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
                    "taskType": "my-task",
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

        let ctx = WorkflowContextImpl::new_with_task_submitter(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
            Some(Arc::new(MockTaskSubmitter)),
        );

        // Create futures in parallel
        let task0 = ctx.schedule_async_raw("my-task", serde_json::json!({}));
        let timer0 = ctx.sleep_async(Duration::from_secs(1));
        let task1 = ctx.schedule_async_raw("my-task", serde_json::json!({}));

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
                    "taskType": "slow-task",
                    "taskExecutionId": slow_exec_id
                }),
                Utc::now(),
            ),
            // Task(1) scheduled second, completes first
            ReplayEvent::new(
                2,
                EventType::TaskScheduled,
                serde_json::json!({
                    "taskType": "fast-task",
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

        let ctx = WorkflowContextImpl::new_with_task_submitter(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
            Some(Arc::new(MockTaskSubmitter)),
        );

        // Create futures
        let slow_future = ctx.schedule_async_raw("slow-task", serde_json::json!({}));
        let fast_future = ctx.schedule_async_raw("fast-task", serde_json::json!({}));

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
                    "taskType": "fetch-data",
                    "taskExecutionId": fetch_exec_id
                }),
                Utc::now(),
            ),
            // Timer(0) - timer ID matches what sleep_async generates: "sleep-1"
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

        let ctx = WorkflowContextImpl::new_with_task_submitter(
            Uuid::new_v4(),
            Uuid::new_v4(),
            serde_json::json!({}),
            CommandCollector::new(),
            replay_events,
            1700000000000,
            Some(Arc::new(MockTaskSubmitter)),
        );

        // Create all futures in parallel
        let task = ctx.schedule_async_raw("fetch-data", serde_json::json!({}));
        let timer = ctx.sleep_async(Duration::from_secs(5));
        let op = ctx.run_async_raw("compute", serde_json::json!({"computed": true}));

        // Await in any order
        let op_result = op.await.unwrap();
        assert_eq!(op_result, serde_json::json!({"computed": true}));

        let task_result = task.await.unwrap();
        assert_eq!(task_result, serde_json::json!({"data": "fetched"}));

        let timer_result = timer.await;
        assert!(timer_result.is_ok());
    }
}
