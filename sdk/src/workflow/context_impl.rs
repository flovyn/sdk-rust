//! WorkflowContextImpl - Concrete implementation of WorkflowContext

use crate::error::{FlovynError, Result};
use crate::workflow::command::WorkflowCommand;
use crate::workflow::context::{DeterministicRandom, ScheduleTaskOptions, WorkflowContext};
use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::recorder::CommandRecorder;
use crate::workflow::task_submitter::TaskSubmitter;
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::Arc;
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

    /// Tracking for consumed task execution IDs during replay.
    /// This ensures multiple schedule() calls with the same taskType consume different events.
    consumed_task_execution_ids: RwLock<std::collections::HashSet<String>>,

    /// Task submitter for submitting tasks to the server.
    /// If None, scheduling tasks will fail with an error.
    task_submitter: Option<Arc<dyn TaskSubmitter>>,
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
            consumed_task_execution_ids: RwLock::new(std::collections::HashSet::new()),
            task_submitter,
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

    /// Find an event by sequence number and type
    fn find_event(&self, sequence: i32, event_type: EventType) -> Option<&ReplayEvent> {
        self.existing_events
            .iter()
            .find(|e| e.sequence_number() == sequence && e.event_type() == event_type)
    }

    /// Find an event by type and a field value (e.g., timer_id)
    fn find_event_by_field(
        &self,
        event_type: EventType,
        field_name: &str,
        field_value: &str,
    ) -> Option<&ReplayEvent> {
        self.existing_events.iter().find(|e| {
            e.event_type() == event_type
                && e.get_string(field_name)
                    .map(|v| v == field_value)
                    .unwrap_or(false)
        })
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
        let sequence = self.next_sequence();

        // Check if we have a cached result from replay
        {
            let cache = self.operation_cache.read();
            if let Some(cached) = cache.get(name) {
                // Record the command for validation
                self.record_command(WorkflowCommand::RecordOperation {
                    sequence_number: sequence,
                    operation_name: name.to_string(),
                    result: cached.clone(),
                })?;
                return Ok(cached.clone());
            }
        }

        // Check existing events for replay
        if let Some(event) = self.find_event(sequence, EventType::OperationCompleted) {
            if let Some(cached_result) = event.get("result") {
                // Cache it for future lookups
                {
                    let mut cache = self.operation_cache.write();
                    cache.insert(name.to_string(), cached_result.clone());
                }

                // Record the command
                self.record_command(WorkflowCommand::RecordOperation {
                    sequence_number: sequence,
                    operation_name: name.to_string(),
                    result: cached_result.clone(),
                })?;

                return Ok(cached_result.clone());
            }
        }

        // New operation - record and cache the result
        {
            let mut cache = self.operation_cache.write();
            cache.insert(name.to_string(), result.clone());
        }

        self.record_command(WorkflowCommand::RecordOperation {
            sequence_number: sequence,
            operation_name: name.to_string(),
            result: result.clone(),
        })?;

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
        // Find the FIRST UNCONSUMED TASK_SCHEDULED event for this taskType (by sequence order).
        // This is critical for handling multiple schedule() calls with the same taskType.
        // Each call must consume a different task event, not reuse the same one.
        let unconsumed_scheduled_event = {
            let consumed = self.consumed_task_execution_ids.read();
            self.existing_events
                .iter()
                .filter(|e| {
                    e.event_type() == EventType::TaskScheduled
                        && e.get_string("taskType")
                            .map(|t| t == task_type)
                            .unwrap_or(false)
                        && !consumed.contains(
                            &e.get_string("taskExecutionId")
                                .or_else(|| e.get_string("taskId"))
                                .unwrap_or_default()
                                .to_string(),
                        )
                })
                .min_by_key(|e| e.sequence_number())
                .cloned()
        };

        // If we found an unconsumed SCHEDULED event, look for its completion/failure
        if let Some(scheduled_event) = unconsumed_scheduled_event {
            let task_execution_id = scheduled_event
                .get_string("taskExecutionId")
                .or_else(|| scheduled_event.get_string("taskId"))
                .ok_or_else(|| {
                    FlovynError::Other("TASK_SCHEDULED event missing taskExecutionId".to_string())
                })?
                .to_string();

            // Find terminal event (COMPLETED or FAILED) for this specific task
            let terminal_event = self
                .existing_events
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
                .max_by_key(|e| e.sequence_number());

            match terminal_event {
                Some(event) if event.event_type() == EventType::TaskCompleted => {
                    // Task completed - mark as consumed and return result
                    {
                        let mut consumed = self.consumed_task_execution_ids.write();
                        consumed.insert(task_execution_id);
                    }

                    // Get result from "result" or "output" field
                    let result = event
                        .get("result")
                        .or_else(|| event.get("output"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    return Ok(result);
                }
                Some(event) if event.event_type() == EventType::TaskFailed => {
                    // Task failed - mark as consumed and return error
                    {
                        let mut consumed = self.consumed_task_execution_ids.write();
                        consumed.insert(task_execution_id);
                    }

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

        // No unconsumed task event found → submit new task
        // First, submit the task to the server to create the TaskExecution entity
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
        let sequence = self.next_sequence();

        // Record command
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
        let sequence = self.next_sequence();

        // Record command
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
        // Generate deterministic timer ID using dedicated counter.
        // This counter is separate from sequence_number to ensure timer IDs are
        // consistent across replays (sequence depends on existingEvents.size).
        let sleep_count = self.sleep_call_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let timer_id = format!("sleep-{}", sleep_count);
        let duration_ms = duration.as_millis() as i64;

        // Check for existing TIMER_FIRED event (during replay) by timer_id
        let fired_event = self.find_event_by_field(EventType::TimerFired, "timerId", &timer_id);
        if fired_event.is_some() {
            // Timer already fired during replay - return immediately (no new command)
            return Ok(());
        }

        // Check for existing TIMER_STARTED event (during replay)
        let started_event = self.find_event_by_field(EventType::TimerStarted, "timerId", &timer_id);

        if started_event.is_none() {
            // New timer - record START_TIMER command
            let sequence = self.next_sequence();
            self.record_command(WorkflowCommand::StartTimer {
                sequence_number: sequence,
                timer_id: timer_id.clone(),
                duration_ms,
            })?;
        }

        // Throw exception to suspend workflow execution (will be resumed when timer fires)
        // NOTE: The SuspendWorkflow command will be added by workflow_worker when it catches this error
        Err(FlovynError::Suspended {
            reason: format!("Waiting for timer: {}", timer_id),
        })
    }

    async fn promise_raw(&self, name: &str) -> Result<Value> {
        self.promise_with_timeout_raw(name, Duration::from_secs(u64::MAX))
            .await
    }

    async fn promise_with_timeout_raw(&self, name: &str, timeout: Duration) -> Result<Value> {
        let timeout_ms = if timeout.as_secs() == u64::MAX {
            None
        } else {
            Some(timeout.as_millis() as i64)
        };

        // Check for existing PROMISE_RESOLVED event (during replay) by promise name
        if let Some(event) =
            self.find_event_by_field(EventType::PromiseResolved, "promiseName", name)
        {
            if let Some(value) = event.get("value") {
                // Promise already resolved during replay - return immediately (no new command)
                return Ok(value.clone());
            }
        }

        // Check for existing PROMISE_REJECTED event (during replay) by promise name
        if let Some(event) =
            self.find_event_by_field(EventType::PromiseRejected, "promiseName", name)
        {
            let error = event
                .get_string("error")
                .unwrap_or("Promise rejected")
                .to_string();
            return Err(FlovynError::PromiseRejected {
                name: name.to_string(),
                error,
            });
        }

        // Check for timeout event (during replay) by promise name
        if self
            .find_event_by_field(EventType::PromiseTimeout, "promiseName", name)
            .is_some()
        {
            // Promise already timed out during replay - return error (no new command)
            return Err(FlovynError::PromiseTimeout {
                name: name.to_string(),
            });
        }

        // Check for existing PROMISE_CREATED event (during replay)
        let created_event = self.find_event_by_field(EventType::PromiseCreated, "promiseId", name);

        if created_event.is_none() {
            // New promise - record CREATE_PROMISE command
            let sequence = self.next_sequence();
            self.record_command(WorkflowCommand::CreatePromise {
                sequence_number: sequence,
                promise_id: name.to_string(),
                timeout_ms,
            })?;
        }

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
        // Check for existing CHILD_WORKFLOW_COMPLETED event (during replay) by childExecutionName
        if let Some(event) = self.find_event_by_field(
            EventType::ChildWorkflowCompleted,
            "childExecutionName",
            name,
        ) {
            if let Some(result) = event.get("output") {
                // Child workflow already completed during replay - return immediately (no new command)
                return Ok(result.clone());
            }
        }

        // Check for existing CHILD_WORKFLOW_FAILED event (during replay) by childExecutionName
        if let Some(event) =
            self.find_event_by_field(EventType::ChildWorkflowFailed, "childExecutionName", name)
        {
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

        // Check for existing CHILD_WORKFLOW_INITIATED event (during replay)
        let initiated_event = self.find_event_by_field(
            EventType::ChildWorkflowInitiated,
            "childExecutionName",
            name,
        );

        if initiated_event.is_none() {
            // New child workflow - record SCHEDULE_CHILD_WORKFLOW command
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
        }

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
    async fn test_run_raw_caches_result() {
        let ctx = create_test_context();

        // First call
        let result1 = ctx
            .run_raw("cached-op", serde_json::json!({"first": true}))
            .await
            .unwrap();

        // Second call with different value - should return cached
        let result2 = ctx
            .run_raw("cached-op", serde_json::json!({"second": true}))
            .await
            .unwrap();

        assert_eq!(result1, serde_json::json!({"first": true}));
        assert_eq!(result2, serde_json::json!({"first": true})); // Cached!
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
}
