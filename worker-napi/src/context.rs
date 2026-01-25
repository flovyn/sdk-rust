//! NAPI Context types for workflows and tasks.
//!
//! This module provides context types for NAPI bindings:
//!
//! ## NapiWorkflowContext
//!
//! A replay-aware workflow context that handles replay logic internally,
//! so language SDKs don't need to implement replay matching themselves.
//!
//! ## NapiTaskContext
//!
//! A task context that provides streaming and lifecycle APIs during task execution.

use flovyn_worker_core::workflow::execution::{DeterministicRandom, SeededRandom};
use flovyn_worker_core::workflow::ReplayEngine;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

use crate::command::NapiWorkflowCommand;
use crate::error::{napi_error, NapiErrorCode};

// ============================================================================
// Result Types - Returned by context methods (all use String for JS compat)
// ============================================================================

/// Result of scheduling a task.
#[napi(object)]
#[derive(Clone)]
pub struct TaskResult {
    /// Status: "completed", "failed", or "pending"
    pub status: String,
    /// Serialized output as JSON string (if completed).
    pub output: Option<String>,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Whether this is retryable (if failed).
    pub retryable: Option<bool>,
    /// The task execution ID (if pending or for tracking).
    pub task_execution_id: Option<String>,
}

/// Result of creating a promise.
#[napi(object)]
#[derive(Clone)]
pub struct PromiseResult {
    /// Status: "resolved", "rejected", "timed_out", or "pending"
    pub status: String,
    /// Serialized value as JSON string (if resolved).
    pub value: Option<String>,
    /// Error message (if rejected).
    pub error: Option<String>,
    /// The promise ID (for tracking).
    pub promise_id: String,
}

/// Result of starting a timer.
#[napi(object)]
#[derive(Clone)]
pub struct TimerResult {
    /// Status: "fired" or "pending"
    pub status: String,
    /// The timer ID (for tracking).
    pub timer_id: String,
}

/// Result of scheduling a child workflow.
#[napi(object)]
#[derive(Clone)]
pub struct ChildWorkflowResult {
    /// Status: "completed", "failed", or "pending"
    pub status: String,
    /// Serialized output as JSON string (if completed).
    pub output: Option<String>,
    /// Error message (if failed).
    pub error: Option<String>,
    /// The child execution ID (for tracking).
    pub child_execution_id: Option<String>,
}

/// Result of running a side effect operation.
#[napi(object)]
#[derive(Clone)]
pub struct OperationResult {
    /// Status: "cached" or "execute"
    pub status: String,
    /// Cached value as JSON string (if cached).
    pub value: Option<String>,
    /// Sequence number for this operation (if execute).
    pub operation_seq: Option<u32>,
}

// ============================================================================
// Internal Result Types
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
// Parsed Replay Event
// ============================================================================

#[derive(Debug, Clone)]
struct ParsedEvent {
    #[allow(dead_code)]
    sequence_number: i32,
    event_type: String,
    data: serde_json::Value,
    #[allow(dead_code)]
    timestamp_ms: i64,
}

impl ParsedEvent {
    fn from_json(json_str: &str) -> Option<Self> {
        let data: serde_json::Value = serde_json::from_str(json_str).ok()?;
        Some(Self {
            sequence_number: data.get("sequenceNumber")?.as_i64()? as i32,
            event_type: data.get("eventType")?.as_str()?.to_string(),
            data: data.get("data")?.clone(),
            timestamp_ms: data.get("timestampMs")?.as_i64().unwrap_or(0),
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

// ============================================================================
// NapiWorkflowContext
// ============================================================================

/// Replay-aware workflow context for NAPI bindings.
#[napi]
pub struct NapiWorkflowContext {
    workflow_execution_id: Uuid,
    #[allow(dead_code)]
    org_id: Uuid,
    replay_engine: ReplayEngine,
    commands: Mutex<Vec<NapiWorkflowCommand>>,
    completed_tasks: HashMap<String, TaskResultInternal>,
    resolved_promises: HashMap<String, PromiseResultInternal>,
    fired_timers: HashSet<String>,
    completed_child_workflows: HashMap<String, ChildWorkflowResultInternal>,
    operation_cache: HashMap<String, Vec<u8>>,
    current_time_ms: i64,
    uuid_counter: AtomicI64,
    random: SeededRandom,
    state: RwLock<HashMap<String, Vec<u8>>>,
    cancellation_requested: AtomicBool,
}

impl NapiWorkflowContext {
    /// Create a new NapiWorkflowContext from activation data.
    pub fn new_from_data(
        workflow_execution_id: &str,
        org_id: &str,
        timestamp_ms: i64,
        random_seed: &str,
        replay_events: &[String],
        state_entries: &[(String, String)],
        cancellation_requested: bool,
    ) -> Result<Self> {
        let workflow_id = Uuid::parse_str(workflow_execution_id).map_err(|e| {
            napi_error(
                NapiErrorCode::InvalidConfiguration,
                format!("Invalid workflow ID: {}", e),
            )
        })?;

        let org_uuid = Uuid::parse_str(org_id).map_err(|e| {
            napi_error(
                NapiErrorCode::InvalidConfiguration,
                format!("Invalid org ID: {}", e),
            )
        })?;

        // Parse replay events
        let all_events: Vec<ParsedEvent> = replay_events
            .iter()
            .filter_map(|s| ParsedEvent::from_json(s))
            .collect();

        // Convert to core replay events
        let core_events: Vec<flovyn_worker_core::workflow::ReplayEvent> = all_events
            .iter()
            .filter_map(|e| {
                let event_type = parse_event_type_str(&e.event_type)?;
                Some(flovyn_worker_core::workflow::ReplayEvent::new(
                    e.sequence_number,
                    event_type,
                    e.data.clone(),
                    chrono::Utc::now(),
                ))
            })
            .collect();

        let replay_engine = ReplayEngine::new(core_events);

        // Build result maps
        let completed_tasks = Self::build_task_results(&all_events);
        let resolved_promises = Self::build_promise_results(&all_events);
        let fired_timers = Self::build_fired_timers(&all_events);
        let completed_child_workflows = Self::build_child_workflow_results(&all_events);
        let operation_cache = Self::build_operation_cache(&all_events);

        // Parse state entries (base64 decode values)
        let state: HashMap<String, Vec<u8>> = state_entries
            .iter()
            .filter_map(|(k, v)| {
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD.decode(v).ok()?;
                Some((k.clone(), decoded))
            })
            .collect();

        // Parse random seed
        let seed: u64 = random_seed.parse().unwrap_or(workflow_id.as_u128() as u64);

        Ok(Self {
            workflow_execution_id: workflow_id,
            org_id: org_uuid,
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
            state: RwLock::new(state),
            cancellation_requested: AtomicBool::new(cancellation_requested),
        })
    }

    fn build_task_results(events: &[ParsedEvent]) -> HashMap<String, TaskResultInternal> {
        let mut results = HashMap::new();
        for event in events {
            match event.event_type.as_str() {
                "TASK_COMPLETED" | "TaskCompleted" => {
                    if let Some(task_id) = event.get_string("taskExecutionId") {
                        let output = event.get_bytes("output").unwrap_or_default();
                        results.insert(task_id, TaskResultInternal::Completed { output });
                    }
                }
                "TASK_FAILED" | "TaskFailed" => {
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
            match event.event_type.as_str() {
                "PROMISE_RESOLVED" | "PromiseResolved" => {
                    if let Some(promise_id) = event.get_string("promiseId") {
                        let value = event.get_bytes("value").unwrap_or_default();
                        results.insert(promise_id, PromiseResultInternal::Resolved { value });
                    }
                }
                "PROMISE_REJECTED" | "PromiseRejected" => {
                    if let Some(promise_id) = event.get_string("promiseId") {
                        let error = event.get_string("error").unwrap_or_default();
                        results.insert(promise_id, PromiseResultInternal::Rejected { error });
                    }
                }
                "PROMISE_TIMEOUT" | "PromiseTimeout" => {
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
            .filter(|e| e.event_type == "TIMER_FIRED" || e.event_type == "TimerFired")
            .filter_map(|e| e.get_string("timerId"))
            .collect()
    }

    fn build_child_workflow_results(
        events: &[ParsedEvent],
    ) -> HashMap<String, ChildWorkflowResultInternal> {
        let mut results = HashMap::new();
        for event in events {
            match event.event_type.as_str() {
                "CHILD_WORKFLOW_COMPLETED" | "ChildWorkflowCompleted" => {
                    if let Some(child_name) = event.get_string("childExecutionName") {
                        let output = event.get_bytes("output").unwrap_or_default();
                        results.insert(
                            child_name,
                            ChildWorkflowResultInternal::Completed { output },
                        );
                    }
                }
                "CHILD_WORKFLOW_FAILED" | "ChildWorkflowFailed" => {
                    if let Some(child_name) = event.get_string("childExecutionName") {
                        let error = event.get_string("error").unwrap_or_default();
                        results.insert(child_name, ChildWorkflowResultInternal::Failed { error });
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
            .filter(|e| {
                e.event_type == "OPERATION_COMPLETED" || e.event_type == "OperationCompleted"
            })
            .filter_map(|e| {
                let name = e
                    .get_string("operationName")
                    .or_else(|| e.get_string("name"))?;
                let result = e.get_bytes("result")?;
                Some((name, result))
            })
            .collect()
    }

    fn generate_uuid(&self) -> Uuid {
        let counter = self.uuid_counter.fetch_add(1, Ordering::SeqCst);
        let name = format!("{}:{}", self.workflow_execution_id, counter);
        Uuid::new_v5(&self.workflow_execution_id, name.as_bytes())
    }

    /// Get commands as JSON array string.
    pub fn get_commands_json(&self) -> String {
        let commands: Vec<serde_json::Value> =
            self.commands.lock().iter().map(|c| c.to_json()).collect();
        serde_json::to_string(&commands).unwrap_or_else(|_| "[]".to_string())
    }

    /// Clear commands after taking them.
    pub fn clear_commands(&self) {
        self.commands.lock().clear();
    }
}

#[napi]
impl NapiWorkflowContext {
    /// Create a new context from activation data.
    #[napi(constructor)]
    pub fn new(
        workflow_execution_id: String,
        org_id: String,
        timestamp_ms: i64,
        random_seed: String,
        replay_events: Vec<String>,
        state_entries: Vec<crate::activation::StateEntry>,
        cancellation_requested: bool,
    ) -> Result<Self> {
        let entries: Vec<(String, String)> = state_entries
            .into_iter()
            .map(|e| (e.key, e.value))
            .collect();
        Self::new_from_data(
            &workflow_execution_id,
            &org_id,
            timestamp_ms,
            &random_seed,
            &replay_events,
            &entries,
            cancellation_requested,
        )
    }

    /// Get the workflow execution ID.
    #[napi(getter)]
    pub fn workflow_execution_id(&self) -> String {
        self.workflow_execution_id.to_string()
    }

    /// Schedule a task for execution.
    #[napi]
    pub fn schedule_task(
        &self,
        kind: String,
        input: String,
        queue: Option<String>,
        timeout_ms: Option<i64>,
    ) -> Result<TaskResult> {
        let task_seq = self.replay_engine.next_task_seq();

        if let Some(event) = self.replay_engine.get_task_event(task_seq) {
            let event_kind = event.get_string("kind").unwrap_or_default();
            if event_kind != kind {
                return Err(napi_error(
                    NapiErrorCode::DeterminismViolation,
                    format!(
                        "Task kind mismatch at Task({}): expected '{}', got '{}'",
                        task_seq, kind, event_kind
                    ),
                ));
            }

            self.uuid_counter.fetch_add(1, Ordering::SeqCst);

            let task_execution_id = event.get_string("taskExecutionId").unwrap_or_default();

            if let Some(result) = self.completed_tasks.get(task_execution_id) {
                return Ok(match result {
                    TaskResultInternal::Completed { output } => TaskResult {
                        status: "completed".to_string(),
                        output: Some(String::from_utf8_lossy(output).to_string()),
                        error: None,
                        retryable: None,
                        task_execution_id: Some(task_execution_id.to_string()),
                    },
                    TaskResultInternal::Failed { error, retryable } => TaskResult {
                        status: "failed".to_string(),
                        output: None,
                        error: Some(error.clone()),
                        retryable: Some(*retryable),
                        task_execution_id: Some(task_execution_id.to_string()),
                    },
                });
            }

            Ok(TaskResult {
                status: "pending".to_string(),
                output: None,
                error: None,
                retryable: None,
                task_execution_id: Some(task_execution_id.to_string()),
            })
        } else {
            let task_execution_id = self.generate_uuid().to_string();

            self.commands
                .lock()
                .push(NapiWorkflowCommand::ScheduleTask {
                    task_execution_id: task_execution_id.clone(),
                    kind,
                    input: input.into_bytes(),
                    priority_seconds: None,
                    max_retries: None,
                    timeout_ms,
                    queue,
                    idempotency_key: None,
                    idempotency_key_ttl_seconds: None,
                });

            Ok(TaskResult {
                status: "pending".to_string(),
                output: None,
                error: None,
                retryable: None,
                task_execution_id: Some(task_execution_id),
            })
        }
    }

    /// Create a durable promise.
    #[napi]
    pub fn create_promise(&self, name: String, timeout_ms: Option<i64>) -> Result<PromiseResult> {
        let promise_seq = self.replay_engine.next_promise_seq();

        if let Some(event) = self.replay_engine.get_promise_event(promise_seq) {
            let event_name = event.get_string("promiseName").unwrap_or_default();
            if event_name != name {
                return Err(napi_error(
                    NapiErrorCode::DeterminismViolation,
                    format!(
                        "Promise name mismatch at Promise({}): expected '{}', got '{}'",
                        promise_seq, name, event_name
                    ),
                ));
            }

            let promise_id = event.get_string("promiseId").unwrap_or_default();

            if let Some(result) = self.resolved_promises.get(promise_id) {
                return Ok(match result {
                    PromiseResultInternal::Resolved { value } => PromiseResult {
                        status: "resolved".to_string(),
                        value: Some(String::from_utf8_lossy(value).to_string()),
                        error: None,
                        promise_id: name,
                    },
                    PromiseResultInternal::Rejected { error } => PromiseResult {
                        status: "rejected".to_string(),
                        value: None,
                        error: Some(error.clone()),
                        promise_id: name,
                    },
                    PromiseResultInternal::TimedOut => PromiseResult {
                        status: "timed_out".to_string(),
                        value: None,
                        error: None,
                        promise_id: name,
                    },
                });
            }

            Ok(PromiseResult {
                status: "pending".to_string(),
                value: None,
                error: None,
                promise_id: name,
            })
        } else {
            self.commands
                .lock()
                .push(NapiWorkflowCommand::CreatePromise {
                    promise_id: name.clone(),
                    timeout_ms,
                    idempotency_key: None,
                    idempotency_key_ttl_seconds: None,
                });

            Ok(PromiseResult {
                status: "pending".to_string(),
                value: None,
                error: None,
                promise_id: name,
            })
        }
    }

    /// Start a timer.
    #[napi]
    pub fn start_timer(&self, duration_ms: i64) -> Result<TimerResult> {
        let timer_seq = self.replay_engine.next_timer_seq();
        let timer_id = format!("timer-{}", timer_seq);

        if let Some(event) = self.replay_engine.get_timer_event(timer_seq) {
            let event_timer_id = event.get_string("timerId").unwrap_or_default();
            if event_timer_id != timer_id {
                return Err(napi_error(
                    NapiErrorCode::DeterminismViolation,
                    format!(
                        "Timer ID mismatch at Timer({}): expected '{}', got '{}'",
                        timer_seq, timer_id, event_timer_id
                    ),
                ));
            }

            if self.fired_timers.contains(&timer_id) {
                return Ok(TimerResult {
                    status: "fired".to_string(),
                    timer_id,
                });
            }

            Ok(TimerResult {
                status: "pending".to_string(),
                timer_id,
            })
        } else {
            self.commands.lock().push(NapiWorkflowCommand::StartTimer {
                timer_id: timer_id.clone(),
                duration_ms,
            });

            Ok(TimerResult {
                status: "pending".to_string(),
                timer_id,
            })
        }
    }

    /// Schedule a child workflow.
    #[napi]
    pub fn schedule_child_workflow(
        &self,
        name: String,
        kind: Option<String>,
        input: String,
        queue: Option<String>,
        priority_seconds: Option<i32>,
    ) -> Result<ChildWorkflowResult> {
        let child_seq = self.replay_engine.next_child_workflow_seq();

        if let Some(event) = self.replay_engine.get_child_workflow_event(child_seq) {
            let event_name = event.get_string("childExecutionName").unwrap_or_default();
            if event_name != name {
                return Err(napi_error(
                    NapiErrorCode::DeterminismViolation,
                    format!(
                        "Child workflow name mismatch at ChildWorkflow({}): expected '{}', got '{}'",
                        child_seq, name, event_name
                    ),
                ));
            }

            self.uuid_counter.fetch_add(1, Ordering::SeqCst);

            let child_execution_id = event
                .get_string("childExecutionId")
                .unwrap_or_default()
                .to_string();

            if let Some(result) = self.completed_child_workflows.get(&name) {
                return Ok(match result {
                    ChildWorkflowResultInternal::Completed { output } => ChildWorkflowResult {
                        status: "completed".to_string(),
                        output: Some(String::from_utf8_lossy(output).to_string()),
                        error: None,
                        child_execution_id: Some(child_execution_id.clone()),
                    },
                    ChildWorkflowResultInternal::Failed { error } => ChildWorkflowResult {
                        status: "failed".to_string(),
                        output: None,
                        error: Some(error.clone()),
                        child_execution_id: Some(child_execution_id.clone()),
                    },
                });
            }

            Ok(ChildWorkflowResult {
                status: "pending".to_string(),
                output: None,
                error: None,
                child_execution_id: Some(child_execution_id),
            })
        } else {
            let child_execution_id = self.generate_uuid().to_string();

            self.commands
                .lock()
                .push(NapiWorkflowCommand::ScheduleChildWorkflow {
                    name,
                    kind,
                    child_execution_id: child_execution_id.clone(),
                    input: input.into_bytes(),
                    queue: queue.unwrap_or_default(),
                    priority_seconds: priority_seconds.unwrap_or(0),
                });

            Ok(ChildWorkflowResult {
                status: "pending".to_string(),
                output: None,
                error: None,
                child_execution_id: Some(child_execution_id),
            })
        }
    }

    /// Run a side effect operation.
    #[napi]
    pub fn run_operation(&self, name: String) -> Result<OperationResult> {
        let operation_seq = self.replay_engine.next_operation_seq();

        if let Some(event) = self.replay_engine.get_operation_event(operation_seq) {
            let event_name = event
                .get_string("operationName")
                .or_else(|| event.get_string("name"))
                .unwrap_or_default();

            if event_name != name {
                return Err(napi_error(
                    NapiErrorCode::DeterminismViolation,
                    format!(
                        "Operation name mismatch at Operation({}): expected '{}', got '{}'",
                        operation_seq, name, event_name
                    ),
                ));
            }

            if let Some(value) = self.operation_cache.get(&name) {
                return Ok(OperationResult {
                    status: "cached".to_string(),
                    value: Some(String::from_utf8_lossy(value).to_string()),
                    operation_seq: None,
                });
            }

            Err(napi_error(
                NapiErrorCode::Other,
                format!("Operation '{}' has event but no cached result", name),
            ))
        } else {
            Ok(OperationResult {
                status: "execute".to_string(),
                value: None,
                operation_seq: Some(operation_seq),
            })
        }
    }

    /// Record the result of an operation.
    #[napi]
    pub fn record_operation_result(&self, name: String, result: String) {
        self.commands
            .lock()
            .push(NapiWorkflowCommand::RecordOperation {
                operation_name: name,
                result: result.into_bytes(),
            });
    }

    /// Get the current time in milliseconds.
    #[napi]
    pub fn current_time_millis(&self) -> i64 {
        self.current_time_ms
    }

    /// Generate a deterministic UUID.
    #[napi]
    pub fn random_uuid(&self) -> String {
        self.generate_uuid().to_string()
    }

    /// Generate a deterministic random number in [0, 1).
    #[napi]
    pub fn random(&self) -> f64 {
        self.random.next_double()
    }

    /// Set workflow state.
    #[napi]
    pub fn set_state(&self, key: String, value: String) -> Result<()> {
        let state_seq = self.replay_engine.next_state_seq() as usize;
        let value_bytes = value.into_bytes();

        if state_seq >= self.replay_engine.state_event_count() {
            self.commands.lock().push(NapiWorkflowCommand::SetState {
                key: key.clone(),
                value: value_bytes.clone(),
            });
        }

        self.state.write().insert(key, value_bytes);
        Ok(())
    }

    /// Get workflow state.
    #[napi]
    pub fn get_state(&self, key: String) -> Option<String> {
        self.state
            .read()
            .get(&key)
            .map(|v| String::from_utf8_lossy(v).to_string())
    }

    /// Clear workflow state.
    #[napi]
    pub fn clear_state(&self, key: String) {
        self.state.write().remove(&key);
        self.commands
            .lock()
            .push(NapiWorkflowCommand::ClearState { key });
    }

    /// Get all state keys.
    #[napi]
    pub fn state_keys(&self) -> Vec<String> {
        self.state.read().keys().cloned().collect()
    }

    /// Check if cancellation has been requested.
    #[napi(getter)]
    pub fn is_cancellation_requested(&self) -> bool {
        self.cancellation_requested.load(Ordering::SeqCst)
    }

    /// Request cancellation.
    #[napi]
    pub fn request_cancellation(&self) {
        self.cancellation_requested.store(true, Ordering::SeqCst);
    }

    /// Take all generated commands as JSON string.
    #[napi]
    pub fn take_commands(&self) -> String {
        let json = self.get_commands_json();
        self.clear_commands();
        json
    }

    /// Get the number of pending commands.
    #[napi]
    pub fn command_count(&self) -> u32 {
        self.commands.lock().len() as u32
    }
}

// ============================================================================
// NapiTaskContext
// ============================================================================

/// Task context for NAPI, providing streaming and lifecycle APIs.
#[napi]
pub struct NapiTaskContext {
    task_execution_id: Uuid,
    workflow_execution_id: Option<Uuid>,
    attempt: u32,
    channel: tonic::transport::Channel,
    worker_token: String,
    runtime: Arc<tokio::runtime::Runtime>,
    cancelled: AtomicBool,
}

impl NapiTaskContext {
    /// Create a new NapiTaskContext.
    pub fn new_internal(
        task_execution_id: Uuid,
        workflow_execution_id: Option<Uuid>,
        attempt: u32,
        channel: tonic::transport::Channel,
        worker_token: String,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Self {
        Self {
            task_execution_id,
            workflow_execution_id,
            attempt,
            channel,
            worker_token,
            runtime,
            cancelled: AtomicBool::new(false),
        }
    }

    /// Mark the task as cancelled.
    pub fn mark_cancelled(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

#[napi]
impl NapiTaskContext {
    /// Get the task execution ID.
    #[napi(getter)]
    pub fn task_execution_id(&self) -> String {
        self.task_execution_id.to_string()
    }

    /// Get the workflow execution ID.
    #[napi(getter)]
    pub fn workflow_execution_id(&self) -> Option<String> {
        self.workflow_execution_id.map(|id| id.to_string())
    }

    /// Get the current attempt number.
    #[napi(getter)]
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Check if the task has been cancelled.
    #[napi(getter)]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Stream a token.
    #[napi]
    pub async fn stream_token(&self, text: String) -> Result<bool> {
        use flovyn_worker_core::client::TaskExecutionClient;
        use flovyn_worker_core::task::streaming::StreamEvent;

        let event = StreamEvent::token(&text);
        let channel = self.channel.clone();
        let token = self.worker_token.clone();
        let task_id = self.task_execution_id;
        let workflow_id = self.workflow_execution_id;

        self.runtime
            .spawn(async move {
                let mut client = TaskExecutionClient::new(channel, &token);
                client.stream_task_data(task_id, workflow_id, &event).await
            })
            .await
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Join error: {}", e)))?
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Stream error: {}", e)))
    }

    /// Stream progress.
    #[napi]
    pub async fn stream_progress(&self, progress: f64, details: Option<String>) -> Result<bool> {
        use flovyn_worker_core::client::TaskExecutionClient;
        use flovyn_worker_core::task::streaming::StreamEvent;

        let event = StreamEvent::progress(progress, details.as_deref());
        let channel = self.channel.clone();
        let token = self.worker_token.clone();
        let task_id = self.task_execution_id;
        let workflow_id = self.workflow_execution_id;

        self.runtime
            .spawn(async move {
                let mut client = TaskExecutionClient::new(channel, &token);
                client.stream_task_data(task_id, workflow_id, &event).await
            })
            .await
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Join error: {}", e)))?
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Stream error: {}", e)))
    }

    /// Stream data.
    #[napi]
    pub async fn stream_data(&self, data: String) -> Result<bool> {
        use flovyn_worker_core::client::TaskExecutionClient;
        use flovyn_worker_core::task::streaming::StreamEvent;

        let value: serde_json::Value =
            serde_json::from_str(&data).unwrap_or(serde_json::Value::Null);
        let event = StreamEvent::data_value(value);
        let channel = self.channel.clone();
        let token = self.worker_token.clone();
        let task_id = self.task_execution_id;
        let workflow_id = self.workflow_execution_id;

        self.runtime
            .spawn(async move {
                let mut client = TaskExecutionClient::new(channel, &token);
                client.stream_task_data(task_id, workflow_id, &event).await
            })
            .await
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Join error: {}", e)))?
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Stream error: {}", e)))
    }

    /// Stream an error notification.
    #[napi]
    pub async fn stream_error(&self, message: String, code: Option<String>) -> Result<bool> {
        use flovyn_worker_core::client::TaskExecutionClient;
        use flovyn_worker_core::task::streaming::StreamEvent;

        let event = StreamEvent::error(&message, code.as_deref());
        let channel = self.channel.clone();
        let token = self.worker_token.clone();
        let task_id = self.task_execution_id;
        let workflow_id = self.workflow_execution_id;

        self.runtime
            .spawn(async move {
                let mut client = TaskExecutionClient::new(channel, &token);
                client.stream_task_data(task_id, workflow_id, &event).await
            })
            .await
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Join error: {}", e)))?
            .map_err(|e| napi_error(NapiErrorCode::Other, format!("Stream error: {}", e)))
    }
}

fn parse_event_type_str(s: &str) -> Option<flovyn_worker_core::EventType> {
    use flovyn_worker_core::EventType;
    match s {
        "WORKFLOW_STARTED" | "WorkflowStarted" => Some(EventType::WorkflowStarted),
        "WORKFLOW_COMPLETED" | "WorkflowCompleted" => Some(EventType::WorkflowCompleted),
        "WORKFLOW_EXECUTION_FAILED" | "WorkflowExecutionFailed" => {
            Some(EventType::WorkflowExecutionFailed)
        }
        "WORKFLOW_SUSPENDED" | "WorkflowSuspended" => Some(EventType::WorkflowSuspended),
        "CANCELLATION_REQUESTED" | "CancellationRequested" => {
            Some(EventType::CancellationRequested)
        }
        "OPERATION_COMPLETED" | "OperationCompleted" => Some(EventType::OperationCompleted),
        "STATE_SET" | "StateSet" => Some(EventType::StateSet),
        "STATE_CLEARED" | "StateCleared" => Some(EventType::StateCleared),
        "TASK_SCHEDULED" | "TaskScheduled" => Some(EventType::TaskScheduled),
        "TASK_COMPLETED" | "TaskCompleted" => Some(EventType::TaskCompleted),
        "TASK_FAILED" | "TaskFailed" => Some(EventType::TaskFailed),
        "TASK_CANCELLED" | "TaskCancelled" => Some(EventType::TaskCancelled),
        "PROMISE_CREATED" | "PromiseCreated" => Some(EventType::PromiseCreated),
        "PROMISE_RESOLVED" | "PromiseResolved" => Some(EventType::PromiseResolved),
        "PROMISE_REJECTED" | "PromiseRejected" => Some(EventType::PromiseRejected),
        "PROMISE_TIMEOUT" | "PromiseTimeout" => Some(EventType::PromiseTimeout),
        "CHILD_WORKFLOW_INITIATED" | "ChildWorkflowInitiated" => {
            Some(EventType::ChildWorkflowInitiated)
        }
        "CHILD_WORKFLOW_STARTED" | "ChildWorkflowStarted" => Some(EventType::ChildWorkflowStarted),
        "CHILD_WORKFLOW_COMPLETED" | "ChildWorkflowCompleted" => {
            Some(EventType::ChildWorkflowCompleted)
        }
        "CHILD_WORKFLOW_FAILED" | "ChildWorkflowFailed" => Some(EventType::ChildWorkflowFailed),
        "CHILD_WORKFLOW_CANCELLED" | "ChildWorkflowCancelled" => {
            Some(EventType::ChildWorkflowCancelled)
        }
        "TIMER_STARTED" | "TimerStarted" => Some(EventType::TimerStarted),
        "TIMER_FIRED" | "TimerFired" => Some(EventType::TimerFired),
        "TIMER_CANCELLED" | "TimerCancelled" => Some(EventType::TimerCancelled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_new() {
        let ctx = NapiWorkflowContext::new_from_data(
            "550e8400-e29b-41d4-a716-446655440000",
            "550e8400-e29b-41d4-a716-446655440001",
            1000,
            "12345",
            &[],
            &[],
            false,
        )
        .unwrap();

        assert_eq!(
            ctx.workflow_execution_id(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
        assert_eq!(ctx.current_time_millis(), 1000);
    }

    #[test]
    fn test_deterministic_uuid() {
        let ctx = NapiWorkflowContext::new_from_data(
            "550e8400-e29b-41d4-a716-446655440000",
            "550e8400-e29b-41d4-a716-446655440001",
            1000,
            "12345",
            &[],
            &[],
            false,
        )
        .unwrap();

        let uuid1 = ctx.random_uuid();
        let uuid2 = ctx.random_uuid();
        assert_ne!(uuid1, uuid2);
    }

    #[test]
    fn test_state_management() {
        let ctx = NapiWorkflowContext::new_from_data(
            "550e8400-e29b-41d4-a716-446655440000",
            "550e8400-e29b-41d4-a716-446655440001",
            1000,
            "12345",
            &[],
            &[],
            false,
        )
        .unwrap();

        assert!(ctx.get_state("key1".to_string()).is_none());

        ctx.set_state("key1".to_string(), "value1".to_string())
            .unwrap();
        assert_eq!(
            ctx.get_state("key1".to_string()),
            Some("value1".to_string())
        );

        ctx.clear_state("key1".to_string());
        assert!(ctx.get_state("key1".to_string()).is_none());
    }
}
