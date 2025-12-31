//! FfiWorkflowContext - Replay-aware workflow context for FFI.
//!
//! This module provides a workflow context that handles replay logic internally,
//! so language SDKs don't need to implement replay matching themselves.
//!
//! ## How it works
//!
//! 1. Context is created with replay events from the server
//! 2. Events are pre-filtered by type for O(1) lookup
//! 3. Terminal events (TaskCompleted, PromiseResolved, etc.) are parsed into result maps
//! 4. When SDK calls a context method (e.g., `schedule_task`):
//!    - If replaying: validate determinism and return cached result
//!    - If new: generate command and return Pending
//! 5. Commands are extracted at completion time

use flovyn_sdk_core::workflow::execution::{DeterministicRandom, SeededRandom};
use flovyn_sdk_core::workflow::ReplayEngine;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

use crate::command::FfiWorkflowCommand;
use crate::error::FfiError;
use crate::types::{FfiEventType, FfiReplayEvent};

// ============================================================================
// Result Enums - Returned by context methods
// ============================================================================

/// Result of scheduling a task.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiTaskResult {
    /// Task completed during replay - return cached result.
    Completed {
        /// Serialized output as JSON bytes.
        output: Vec<u8>,
    },
    /// Task failed during replay - return cached error.
    Failed {
        /// Error message.
        error: String,
        /// Whether this is retryable.
        retryable: bool,
    },
    /// Task is pending - workflow should suspend.
    Pending {
        /// The task execution ID for tracking.
        task_execution_id: String,
    },
}

/// Result of creating a promise.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiPromiseResult {
    /// Promise resolved during replay.
    Resolved {
        /// Serialized value as JSON bytes.
        value: Vec<u8>,
    },
    /// Promise rejected during replay.
    Rejected {
        /// Error message.
        error: String,
    },
    /// Promise timed out during replay.
    TimedOut,
    /// Promise is pending - workflow should suspend.
    Pending {
        /// The promise ID for tracking.
        promise_id: String,
    },
}

/// Result of starting a timer.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiTimerResult {
    /// Timer fired during replay.
    Fired,
    /// Timer is pending - workflow should suspend.
    Pending {
        /// The timer ID for tracking.
        timer_id: String,
    },
}

/// Result of scheduling a child workflow.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiChildWorkflowResult {
    /// Child workflow completed during replay.
    Completed {
        /// Serialized output as JSON bytes.
        output: Vec<u8>,
    },
    /// Child workflow failed during replay.
    Failed {
        /// Error message.
        error: String,
    },
    /// Child workflow is pending - workflow should suspend.
    Pending {
        /// The child execution ID for tracking.
        child_execution_id: String,
    },
}

/// Result of running a side effect operation.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum FfiOperationResult {
    /// Operation completed during replay - return cached value.
    Cached {
        /// Serialized value as JSON bytes.
        value: Vec<u8>,
    },
    /// Operation is new - SDK should execute and record.
    Execute {
        /// Sequence number for this operation.
        operation_seq: u32,
    },
}

// ============================================================================
// Internal Result Types - For storing parsed terminal events
// ============================================================================

#[derive(Debug, Clone)]
enum TaskResultInternal {
    Completed { output: Vec<u8> },
    Failed { error: String, retryable: bool },
}

#[derive(Debug, Clone)]
enum PromiseResultInternal {
    Resolved { value: Vec<u8> },
    Rejected { error: String },
    TimedOut,
}

#[derive(Debug, Clone)]
enum ChildWorkflowResultInternal {
    Completed { output: Vec<u8> },
    Failed { error: String },
}

// ============================================================================
// Parsed Replay Event - Internal representation
// ============================================================================

#[derive(Debug, Clone)]
struct ParsedEvent {
    #[allow(dead_code)]
    sequence_number: i32,
    event_type: FfiEventType,
    data: serde_json::Value,
    #[allow(dead_code)]
    timestamp_ms: i64,
}

impl ParsedEvent {
    fn from_ffi(event: &FfiReplayEvent) -> Option<Self> {
        let data: serde_json::Value = serde_json::from_slice(&event.data).ok()?;
        Some(Self {
            sequence_number: event.sequence_number,
            event_type: event.event_type,
            data,
            timestamp_ms: event.timestamp_ms,
        })
    }

    fn get_string(&self, key: &str) -> Option<String> {
        self.data
            .get(key)
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    fn get_bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.data
            .get(key)
            .map(|v| serde_json::to_vec(v).unwrap_or_default())
    }

    fn get_bool(&self, key: &str) -> Option<bool> {
        self.data.get(key).and_then(|v| v.as_bool())
    }
}

// SeededRandom is imported from flovyn_sdk_core::workflow::execution

// ============================================================================
// FfiWorkflowContext - Main context object
// ============================================================================

/// Replay-aware workflow context for FFI.
///
/// This context handles all replay logic internally:
/// - Pre-filters events by type for O(1) lookup
/// - Validates determinism during replay
/// - Returns cached results for replayed operations
/// - Only generates commands for NEW operations
#[derive(uniffi::Object)]
pub struct FfiWorkflowContext {
    // Identifiers
    workflow_execution_id: Uuid,
    #[allow(dead_code)]
    tenant_id: Uuid,

    // Replay engine for event filtering, sequence management, and terminal event lookup
    replay_engine: ReplayEngine,

    // Generated commands (only NEW ones)
    commands: Mutex<Vec<FfiWorkflowCommand>>,

    // Resolved values from replay events (FFI-specific: stores bytes for efficiency)
    completed_tasks: HashMap<String, TaskResultInternal>,
    resolved_promises: HashMap<String, PromiseResultInternal>,
    fired_timers: HashSet<String>,
    completed_child_workflows: HashMap<String, ChildWorkflowResultInternal>,
    operation_cache: HashMap<String, Vec<u8>>,

    // Deterministic generation
    current_time_ms: i64,
    uuid_counter: AtomicI64,
    random: SeededRandom,

    // Workflow state (FFI-specific: stores bytes)
    ffi_state: RwLock<HashMap<String, Vec<u8>>>,

    // Cancellation
    cancellation_requested: AtomicBool,
}

impl FfiWorkflowContext {
    /// Create a new FfiWorkflowContext.
    ///
    /// This parses all events and builds lookup tables for replay.
    pub fn new(
        workflow_execution_id: Uuid,
        tenant_id: Uuid,
        timestamp_ms: i64,
        random_seed: u64,
        events: Vec<FfiReplayEvent>,
        state_entries: Vec<(String, Vec<u8>)>,
        cancellation_requested: bool,
    ) -> Arc<Self> {
        // Convert FFI events to core ReplayEvents for ReplayEngine
        let replay_events: Vec<flovyn_sdk_core::workflow::ReplayEvent> =
            events.iter().filter_map(|e| e.to_replay_event()).collect();

        // Create the replay engine
        let replay_engine = ReplayEngine::new(replay_events);

        // Parse events for FFI-specific result maps (stores bytes for efficiency)
        let all_events: Vec<ParsedEvent> =
            events.iter().filter_map(ParsedEvent::from_ffi).collect();

        // Build terminal event lookup maps (FFI-specific: stores bytes)
        let completed_tasks = Self::build_task_results(&all_events);
        let resolved_promises = Self::build_promise_results(&all_events);
        let fired_timers = Self::build_fired_timers(&all_events);
        let completed_child_workflows = Self::build_child_workflow_results(&all_events);
        let operation_cache = Self::build_operation_cache(&all_events);

        // Initialize FFI state from state entries
        let ffi_state: HashMap<String, Vec<u8>> = state_entries.into_iter().collect();

        // Create seed from workflow execution ID if not provided
        let seed = if random_seed == 0 {
            workflow_execution_id.as_u128() as u64
        } else {
            random_seed
        };

        Arc::new(Self {
            workflow_execution_id,
            tenant_id,
            replay_engine,
            commands: Mutex::new(Vec::new()),
            completed_tasks,
            resolved_promises,
            fired_timers,
            completed_child_workflows,
            operation_cache,
            current_time_ms: timestamp_ms,
            uuid_counter: AtomicI64::new(0),
            random: SeededRandom::new(seed),
            ffi_state: RwLock::new(ffi_state),
            cancellation_requested: AtomicBool::new(cancellation_requested),
        })
    }

    fn build_task_results(events: &[ParsedEvent]) -> HashMap<String, TaskResultInternal> {
        let mut results = HashMap::new();

        for event in events {
            match event.event_type {
                FfiEventType::TaskCompleted => {
                    if let Some(task_id) = event.get_string("taskExecutionId") {
                        let output = event.get_bytes("output").unwrap_or_default();
                        results.insert(task_id, TaskResultInternal::Completed { output });
                    }
                }
                FfiEventType::TaskFailed => {
                    if let Some(task_id) = event.get_string("taskExecutionId") {
                        let error = event.get_string("error").unwrap_or_default();
                        let retryable = event.get_bool("retryable").unwrap_or(true);
                        results.insert(task_id, TaskResultInternal::Failed { error, retryable });
                    }
                }
                _ => {}
            }
        }

        results
    }

    fn build_promise_results(events: &[ParsedEvent]) -> HashMap<String, PromiseResultInternal> {
        let mut results = HashMap::new();

        for event in events {
            match event.event_type {
                FfiEventType::PromiseResolved => {
                    if let Some(promise_id) = event.get_string("promiseId") {
                        let value = event.get_bytes("value").unwrap_or_default();
                        results.insert(promise_id, PromiseResultInternal::Resolved { value });
                    }
                }
                FfiEventType::PromiseRejected => {
                    if let Some(promise_id) = event.get_string("promiseId") {
                        let error = event.get_string("error").unwrap_or_default();
                        results.insert(promise_id, PromiseResultInternal::Rejected { error });
                    }
                }
                FfiEventType::PromiseTimeout => {
                    if let Some(promise_id) = event.get_string("promiseId") {
                        results.insert(promise_id, PromiseResultInternal::TimedOut);
                    }
                }
                _ => {}
            }
        }

        results
    }

    fn build_fired_timers(events: &[ParsedEvent]) -> HashSet<String> {
        events
            .iter()
            .filter(|e| e.event_type == FfiEventType::TimerFired)
            .filter_map(|e| e.get_string("timerId"))
            .collect()
    }

    fn build_child_workflow_results(
        events: &[ParsedEvent],
    ) -> HashMap<String, ChildWorkflowResultInternal> {
        let mut results = HashMap::new();

        for event in events {
            match event.event_type {
                FfiEventType::ChildWorkflowCompleted => {
                    if let Some(child_id) = event.get_string("childWorkflowExecutionId") {
                        let output = event.get_bytes("output").unwrap_or_default();
                        results.insert(child_id, ChildWorkflowResultInternal::Completed { output });
                    }
                }
                FfiEventType::ChildWorkflowFailed => {
                    if let Some(child_id) = event.get_string("childWorkflowExecutionId") {
                        let error = event.get_string("error").unwrap_or_default();
                        results.insert(child_id, ChildWorkflowResultInternal::Failed { error });
                    }
                }
                _ => {}
            }
        }

        results
    }

    fn build_operation_cache(events: &[ParsedEvent]) -> HashMap<String, Vec<u8>> {
        events
            .iter()
            .filter(|e| e.event_type == FfiEventType::OperationCompleted)
            .filter_map(|e| {
                let name = e.get_string("operationName")?;
                let result = e.get_bytes("result")?;
                Some((name, result))
            })
            .collect()
    }

    /// Generate a deterministic UUID based on workflow ID and counter.
    fn generate_uuid(&self) -> Uuid {
        let counter = self.uuid_counter.fetch_add(1, Ordering::SeqCst);
        let base = self.workflow_execution_id.as_u128();
        let combined = base.wrapping_add(counter as u128);
        Uuid::from_u128(combined)
    }
}

// ============================================================================
// FFI-exported methods
// ============================================================================

#[uniffi::export]
impl FfiWorkflowContext {
    /// Get the workflow execution ID.
    pub fn workflow_execution_id(&self) -> String {
        self.workflow_execution_id.to_string()
    }

    /// Schedule a task for execution.
    ///
    /// Returns:
    /// - `Completed` if task already completed during replay
    /// - `Failed` if task already failed during replay
    /// - `Pending` if task is new or not yet completed
    pub fn schedule_task(
        &self,
        kind: String,
        input: Vec<u8>,
        queue: Option<String>,
        timeout_ms: Option<i64>,
    ) -> Result<FfiTaskResult, FfiError> {
        let task_seq = self.replay_engine.next_task_seq();

        if let Some(event) = self.replay_engine.get_task_event(task_seq) {
            // Replay: validate task kind matches
            let event_kind = event.get_string("kind").unwrap_or_default();

            if event_kind != kind {
                return Err(FfiError::DeterminismViolation {
                    msg: format!(
                        "Task kind mismatch at Task({}): expected '{}', got '{}'",
                        task_seq, kind, event_kind
                    ),
                });
            }

            let task_execution_id = event
                .get_string("taskExecutionId")
                .unwrap_or_default()
                .to_string();

            // Look up terminal result from FFI-specific cache
            if let Some(result) = self.completed_tasks.get(&task_execution_id) {
                return Ok(match result {
                    TaskResultInternal::Completed { output } => FfiTaskResult::Completed {
                        output: output.clone(),
                    },
                    TaskResultInternal::Failed { error, retryable } => FfiTaskResult::Failed {
                        error: error.clone(),
                        retryable: *retryable,
                    },
                });
            }

            // Task scheduled but not yet completed
            Ok(FfiTaskResult::Pending { task_execution_id })
        } else {
            // New command
            let task_execution_id = self.generate_uuid().to_string();

            self.commands.lock().push(FfiWorkflowCommand::ScheduleTask {
                task_execution_id: task_execution_id.clone(),
                kind: kind,
                input,
                priority_seconds: None,
                max_retries: None,
                timeout_ms,
                queue,
            });

            Ok(FfiTaskResult::Pending { task_execution_id })
        }
    }

    /// Create a durable promise.
    ///
    /// Returns:
    /// - `Resolved` if promise already resolved during replay
    /// - `Rejected` if promise already rejected during replay
    /// - `TimedOut` if promise already timed out during replay
    /// - `Pending` if promise is new or not yet resolved
    pub fn create_promise(
        &self,
        name: String,
        timeout_ms: Option<i64>,
    ) -> Result<FfiPromiseResult, FfiError> {
        let promise_seq = self.replay_engine.next_promise_seq();

        if let Some(event) = self.replay_engine.get_promise_event(promise_seq) {
            // Replay: validate promise name matches
            let event_name = event.get_string("promiseId").unwrap_or_default();

            if event_name != name {
                return Err(FfiError::DeterminismViolation {
                    msg: format!(
                        "Promise name mismatch at Promise({}): expected '{}', got '{}'",
                        promise_seq, name, event_name
                    ),
                });
            }

            // Look up terminal result from FFI-specific cache
            if let Some(result) = self.resolved_promises.get(&name) {
                return Ok(match result {
                    PromiseResultInternal::Resolved { value } => FfiPromiseResult::Resolved {
                        value: value.clone(),
                    },
                    PromiseResultInternal::Rejected { error } => FfiPromiseResult::Rejected {
                        error: error.clone(),
                    },
                    PromiseResultInternal::TimedOut => FfiPromiseResult::TimedOut,
                });
            }

            // Promise created but not yet resolved
            Ok(FfiPromiseResult::Pending { promise_id: name })
        } else {
            // New command
            self.commands
                .lock()
                .push(FfiWorkflowCommand::CreatePromise {
                    promise_id: name.clone(),
                    timeout_ms,
                });

            Ok(FfiPromiseResult::Pending { promise_id: name })
        }
    }

    /// Start a timer.
    ///
    /// Returns:
    /// - `Fired` if timer already fired during replay
    /// - `Pending` if timer is new or not yet fired
    pub fn start_timer(&self, duration_ms: i64) -> Result<FfiTimerResult, FfiError> {
        let timer_seq = self.replay_engine.next_timer_seq();
        let timer_id = format!("timer-{}", timer_seq);

        if let Some(event) = self.replay_engine.get_timer_event(timer_seq) {
            // Replay: validate timer ID matches
            let event_timer_id = event.get_string("timerId").unwrap_or_default();

            if event_timer_id != timer_id {
                return Err(FfiError::DeterminismViolation {
                    msg: format!(
                        "Timer ID mismatch at Timer({}): expected '{}', got '{}'",
                        timer_seq, timer_id, event_timer_id
                    ),
                });
            }

            // Check if timer fired from FFI-specific cache
            if self.fired_timers.contains(&timer_id) {
                return Ok(FfiTimerResult::Fired);
            }

            // Timer started but not yet fired
            Ok(FfiTimerResult::Pending { timer_id })
        } else {
            // New command
            self.commands.lock().push(FfiWorkflowCommand::StartTimer {
                timer_id: timer_id.clone(),
                duration_ms,
            });

            Ok(FfiTimerResult::Pending { timer_id })
        }
    }

    /// Schedule a child workflow.
    ///
    /// Returns:
    /// - `Completed` if child already completed during replay
    /// - `Failed` if child already failed during replay
    /// - `Pending` if child is new or not yet completed
    pub fn schedule_child_workflow(
        &self,
        name: String,
        kind: Option<String>,
        input: Vec<u8>,
        queue: Option<String>,
        priority_seconds: Option<i32>,
    ) -> Result<FfiChildWorkflowResult, FfiError> {
        let child_seq = self.replay_engine.next_child_workflow_seq();

        if let Some(event) = self.replay_engine.get_child_workflow_event(child_seq) {
            // Replay: validate name matches
            let event_name = event
                .get_string("childWorkflowExecutionName")
                .unwrap_or_default();

            if event_name != name {
                return Err(FfiError::DeterminismViolation {
                    msg: format!(
                        "Child workflow name mismatch at ChildWorkflow({}): expected '{}', got '{}'",
                        child_seq, name, event_name
                    ),
                });
            }

            // Also validate kind if provided
            if let Some(ref expected_kind) = kind {
                let event_kind = event.get_string("workflowKind").unwrap_or_default();
                if event_kind != expected_kind {
                    return Err(FfiError::DeterminismViolation {
                        msg: format!(
                            "Child workflow kind mismatch at ChildWorkflow({}): expected '{}', got '{}'",
                            child_seq, expected_kind, event_kind
                        ),
                    });
                }
            }

            let child_execution_id = event
                .get_string("childWorkflowExecutionId")
                .unwrap_or_default()
                .to_string();

            // Look up terminal result from FFI-specific cache
            if let Some(result) = self.completed_child_workflows.get(&child_execution_id) {
                return Ok(match result {
                    ChildWorkflowResultInternal::Completed { output } => {
                        FfiChildWorkflowResult::Completed {
                            output: output.clone(),
                        }
                    }
                    ChildWorkflowResultInternal::Failed { error } => {
                        FfiChildWorkflowResult::Failed {
                            error: error.clone(),
                        }
                    }
                });
            }

            // Child started but not yet completed
            Ok(FfiChildWorkflowResult::Pending { child_execution_id })
        } else {
            // New command
            let child_execution_id = self.generate_uuid().to_string();

            self.commands
                .lock()
                .push(FfiWorkflowCommand::ScheduleChildWorkflow {
                    name,
                    kind,
                    child_execution_id: child_execution_id.clone(),
                    input,
                    queue: queue.unwrap_or_else(|| "default".to_string()),
                    priority_seconds: priority_seconds.unwrap_or(0),
                });

            Ok(FfiChildWorkflowResult::Pending { child_execution_id })
        }
    }

    /// Run a side effect operation.
    ///
    /// Returns:
    /// - `Cached` if operation already executed during replay
    /// - `Execute` if operation is new and should be executed
    pub fn run_operation(&self, name: String) -> Result<FfiOperationResult, FfiError> {
        let operation_seq = self.replay_engine.next_operation_seq();

        if let Some(event) = self.replay_engine.get_operation_event(operation_seq) {
            // Replay: validate operation name matches
            let event_name = event.get_string("operationName").unwrap_or_default();

            if event_name != name {
                return Err(FfiError::DeterminismViolation {
                    msg: format!(
                        "Operation name mismatch at Operation({}): expected '{}', got '{}'",
                        operation_seq, name, event_name
                    ),
                });
            }

            // Return cached result from FFI-specific cache
            if let Some(value) = self.operation_cache.get(&name) {
                return Ok(FfiOperationResult::Cached {
                    value: value.clone(),
                });
            }

            // Should not happen - operation event without cached result
            Err(FfiError::Other {
                msg: format!("Operation '{}' has event but no cached result", name),
            })
        } else {
            // New operation - SDK should execute
            Ok(FfiOperationResult::Execute { operation_seq })
        }
    }

    /// Record the result of an operation that was executed.
    pub fn record_operation_result(&self, name: String, result: Vec<u8>) {
        self.commands
            .lock()
            .push(FfiWorkflowCommand::RecordOperation {
                operation_name: name,
                result,
            });
    }

    /// Get the current time in milliseconds since Unix epoch.
    ///
    /// This returns a deterministic value that is the same across replays.
    pub fn current_time_millis(&self) -> i64 {
        self.current_time_ms
    }

    /// Generate a deterministic UUID.
    ///
    /// Each call returns a new UUID that is the same across replays.
    pub fn random_uuid(&self) -> String {
        self.generate_uuid().to_string()
    }

    /// Generate a deterministic random number in [0, 1).
    pub fn random(&self) -> f64 {
        self.random.next_double()
    }

    /// Set workflow state.
    pub fn set_state(&self, key: String, value: Vec<u8>) -> Result<(), FfiError> {
        let state_seq = self.replay_engine.next_state_seq() as usize;

        // During replay, we still validate but don't generate commands for replayed state
        if state_seq >= self.replay_engine.state_event_count() {
            // New command
            self.commands.lock().push(FfiWorkflowCommand::SetState {
                key: key.clone(),
                value: value.clone(),
            });
        }

        // Always update local FFI state
        self.ffi_state.write().insert(key, value);
        Ok(())
    }

    /// Get workflow state.
    pub fn get_state(&self, key: String) -> Option<Vec<u8>> {
        self.ffi_state.read().get(&key).cloned()
    }

    /// Clear workflow state.
    pub fn clear_state(&self, key: String) {
        self.ffi_state.write().remove(&key);
        self.commands
            .lock()
            .push(FfiWorkflowCommand::ClearState { key });
    }

    /// Get all state keys.
    pub fn state_keys(&self) -> Vec<String> {
        self.ffi_state.read().keys().cloned().collect()
    }

    /// Check if cancellation has been requested.
    pub fn is_cancellation_requested(&self) -> bool {
        self.cancellation_requested.load(Ordering::SeqCst)
    }

    /// Request cancellation (called when CancelWorkflow job is received).
    pub fn request_cancellation(&self) {
        self.cancellation_requested.store(true, Ordering::SeqCst);
    }

    /// Take all generated commands (only new ones, not replayed).
    ///
    /// This drains the command buffer and returns the commands.
    pub fn take_commands(&self) -> Vec<FfiWorkflowCommand> {
        std::mem::take(&mut *self.commands.lock())
    }

    /// Get the number of pending commands.
    pub fn command_count(&self) -> u32 {
        self.commands.lock().len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context(events: Vec<FfiReplayEvent>) -> Arc<FfiWorkflowContext> {
        FfiWorkflowContext::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1000,
            12345,
            events,
            vec![],
            false,
        )
    }

    #[test]
    fn test_schedule_task_new() {
        let ctx = create_test_context(vec![]);

        let result = ctx
            .schedule_task("my-task".to_string(), b"{}".to_vec(), None, None)
            .unwrap();

        assert!(matches!(result, FfiTaskResult::Pending { .. }));
        assert_eq!(ctx.command_count(), 1);
    }

    #[test]
    fn test_schedule_task_replay_completed() {
        // Create events: TaskScheduled + TaskCompleted
        let task_id = Uuid::new_v4().to_string();
        let events = vec![
            FfiReplayEvent {
                sequence_number: 1,
                event_type: FfiEventType::TaskScheduled,
                data: serde_json::to_vec(&serde_json::json!({
                    "kind": "my-task",
                    "taskExecutionId": task_id
                }))
                .unwrap(),
                timestamp_ms: 1000,
            },
            FfiReplayEvent {
                sequence_number: 2,
                event_type: FfiEventType::TaskCompleted,
                data: serde_json::to_vec(&serde_json::json!({
                    "taskExecutionId": task_id,
                    "output": {"result": 42}
                }))
                .unwrap(),
                timestamp_ms: 2000,
            },
        ];

        let ctx = create_test_context(events);

        let result = ctx
            .schedule_task("my-task".to_string(), b"{}".to_vec(), None, None)
            .unwrap();

        assert!(matches!(result, FfiTaskResult::Completed { .. }));
        // No new commands should be generated during replay
        assert_eq!(ctx.command_count(), 0);
    }

    #[test]
    fn test_schedule_task_determinism_violation() {
        let task_id = Uuid::new_v4().to_string();
        let events = vec![FfiReplayEvent {
            sequence_number: 1,
            event_type: FfiEventType::TaskScheduled,
            data: serde_json::to_vec(&serde_json::json!({
                "kind": "original-task",
                "taskExecutionId": task_id
            }))
            .unwrap(),
            timestamp_ms: 1000,
        }];

        let ctx = create_test_context(events);

        // Try to schedule a different task type
        let result = ctx.schedule_task("different-task".to_string(), b"{}".to_vec(), None, None);

        assert!(matches!(result, Err(FfiError::DeterminismViolation { .. })));
    }

    #[test]
    fn test_start_timer_fired() {
        let events = vec![
            FfiReplayEvent {
                sequence_number: 1,
                event_type: FfiEventType::TimerStarted,
                data: serde_json::to_vec(&serde_json::json!({
                    "timerId": "timer-0",
                    "durationMs": 1000
                }))
                .unwrap(),
                timestamp_ms: 1000,
            },
            FfiReplayEvent {
                sequence_number: 2,
                event_type: FfiEventType::TimerFired,
                data: serde_json::to_vec(&serde_json::json!({
                    "timerId": "timer-0"
                }))
                .unwrap(),
                timestamp_ms: 2000,
            },
        ];

        let ctx = create_test_context(events);

        let result = ctx.start_timer(1000).unwrap();

        assert!(matches!(result, FfiTimerResult::Fired));
        assert_eq!(ctx.command_count(), 0);
    }

    #[test]
    fn test_create_promise_resolved() {
        let events = vec![
            FfiReplayEvent {
                sequence_number: 1,
                event_type: FfiEventType::PromiseCreated,
                data: serde_json::to_vec(&serde_json::json!({
                    "promiseId": "my-promise"
                }))
                .unwrap(),
                timestamp_ms: 1000,
            },
            FfiReplayEvent {
                sequence_number: 2,
                event_type: FfiEventType::PromiseResolved,
                data: serde_json::to_vec(&serde_json::json!({
                    "promiseId": "my-promise",
                    "value": {"approved": true}
                }))
                .unwrap(),
                timestamp_ms: 2000,
            },
        ];

        let ctx = create_test_context(events);

        let result = ctx.create_promise("my-promise".to_string(), None).unwrap();

        assert!(matches!(result, FfiPromiseResult::Resolved { .. }));
        assert_eq!(ctx.command_count(), 0);
    }

    #[test]
    fn test_deterministic_uuid() {
        let ctx = create_test_context(vec![]);

        let uuid1 = ctx.random_uuid();
        let uuid2 = ctx.random_uuid();

        assert_ne!(uuid1, uuid2);

        // Create another context with same workflow ID and seed
        let ctx2 = FfiWorkflowContext::new(
            Uuid::parse_str(&ctx.workflow_execution_id()).unwrap(),
            Uuid::new_v4(),
            1000,
            12345,
            vec![],
            vec![],
            false,
        );

        let uuid1_replay = ctx2.random_uuid();
        let uuid2_replay = ctx2.random_uuid();

        assert_eq!(uuid1, uuid1_replay);
        assert_eq!(uuid2, uuid2_replay);
    }

    #[test]
    fn test_state_management() {
        let ctx = create_test_context(vec![]);

        assert!(ctx.get_state("key1".to_string()).is_none());

        ctx.set_state("key1".to_string(), b"value1".to_vec())
            .unwrap();
        assert_eq!(ctx.get_state("key1".to_string()), Some(b"value1".to_vec()));

        ctx.clear_state("key1".to_string());
        assert!(ctx.get_state("key1".to_string()).is_none());
    }

    #[test]
    fn test_take_commands() {
        let ctx = create_test_context(vec![]);

        ctx.schedule_task("task1".to_string(), b"{}".to_vec(), None, None)
            .unwrap();
        ctx.schedule_task("task2".to_string(), b"{}".to_vec(), None, None)
            .unwrap();

        assert_eq!(ctx.command_count(), 2);

        let commands = ctx.take_commands();
        assert_eq!(commands.len(), 2);
        assert_eq!(ctx.command_count(), 0);
    }
}
