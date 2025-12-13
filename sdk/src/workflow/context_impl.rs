//! WorkflowContextImpl - Concrete implementation of WorkflowContext

use crate::error::{FlovynError, Result};
use crate::workflow::command::WorkflowCommand;
use crate::workflow::context::{DeterministicRandom, ScheduleTaskOptions, WorkflowContext};
use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::recorder::CommandRecorder;
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
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

        Self {
            workflow_execution_id,
            tenant_id,
            input,
            recorder: RwLock::new(recorder),
            existing_events,
            sequence_number: AtomicI32::new(1),
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
        let sequence = self.next_sequence();
        let task_execution_id = self.random_uuid();

        // Check existing events for replay
        if let Some(event) = self.find_event(sequence, EventType::TaskCompleted) {
            if let Some(result) = event.get("result") {
                self.record_command(WorkflowCommand::ScheduleTask {
                    sequence_number: sequence,
                    task_type: task_type.to_string(),
                    task_execution_id,
                    input,
                    priority_seconds: options.priority_seconds,
                })?;
                return Ok(result.clone());
            }
        }

        // Record the command
        self.record_command(WorkflowCommand::ScheduleTask {
            sequence_number: sequence,
            task_type: task_type.to_string(),
            task_execution_id,
            input,
            priority_seconds: options.priority_seconds,
        })?;

        // Check if already resolved
        {
            let pending = self.pending_tasks.read();
            if let Some(result) = pending.get(task_type) {
                return Ok(result.clone());
            }
        }

        // Task not yet resolved - suspend
        Err(FlovynError::Suspended {
            reason: format!("Waiting for task: {}", task_type),
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
        let sequence = self.next_sequence();
        let timer_id = format!("timer-{}", sequence);
        let duration_ms = duration.as_millis() as i64;

        // Check existing events for replay
        if self.find_event(sequence, EventType::TimerFired).is_some() {
            self.record_command(WorkflowCommand::StartTimer {
                sequence_number: sequence,
                timer_id: timer_id.clone(),
                duration_ms,
            })?;
            return Ok(());
        }

        // Record command and suspend (timer needs to fire externally)
        self.record_command(WorkflowCommand::StartTimer {
            sequence_number: sequence,
            timer_id: timer_id.clone(),
            duration_ms,
        })?;

        // Register pending timer and suspend
        {
            let mut pending = self.pending_timers.write();
            pending.insert(timer_id.clone(), duration_ms);
        }

        Err(FlovynError::Suspended {
            reason: format!("Waiting for timer: {}", timer_id),
        })
    }

    async fn promise_raw(&self, name: &str) -> Result<Value> {
        self.promise_with_timeout_raw(name, Duration::from_secs(u64::MAX))
            .await
    }

    async fn promise_with_timeout_raw(&self, name: &str, timeout: Duration) -> Result<Value> {
        let sequence = self.next_sequence();
        let timeout_ms = if timeout.as_secs() == u64::MAX {
            None
        } else {
            Some(timeout.as_millis() as i64)
        };

        // Check existing events for replay
        if let Some(event) = self.find_event(sequence, EventType::PromiseResolved) {
            if let Some(value) = event.get("value") {
                self.record_command(WorkflowCommand::CreatePromise {
                    sequence_number: sequence,
                    promise_id: name.to_string(),
                    timeout_ms,
                })?;
                return Ok(value.clone());
            }
        }

        // Check for timeout
        if let Some(_event) = self.find_event(sequence, EventType::PromiseTimeout) {
            self.record_command(WorkflowCommand::CreatePromise {
                sequence_number: sequence,
                promise_id: name.to_string(),
                timeout_ms,
            })?;
            return Err(FlovynError::PromiseTimeout {
                name: name.to_string(),
            });
        }

        // Record command
        self.record_command(WorkflowCommand::CreatePromise {
            sequence_number: sequence,
            promise_id: name.to_string(),
            timeout_ms,
        })?;

        // Check if already resolved
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
        let sequence = self.next_sequence();
        let child_execution_id = self.random_uuid();

        // Check existing events for replay
        if let Some(event) = self.find_event(sequence, EventType::ChildWorkflowCompleted) {
            if let Some(result) = event.get("result") {
                self.record_command(WorkflowCommand::ScheduleChildWorkflow {
                    sequence_number: sequence,
                    name: name.to_string(),
                    kind: Some(kind.to_string()),
                    definition_id: None,
                    child_execution_id,
                    input,
                    task_queue: "default".to_string(),
                    priority_seconds: 0,
                })?;
                return Ok(result.clone());
            }
        }

        // Record command
        self.record_command(WorkflowCommand::ScheduleChildWorkflow {
            sequence_number: sequence,
            name: name.to_string(),
            kind: Some(kind.to_string()),
            definition_id: None,
            child_execution_id,
            input,
            task_queue: "default".to_string(),
            priority_seconds: 0,
        })?;

        // Check if already resolved
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
        let ctx = create_test_context();

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
    async fn test_schedule_raw_returns_resolved() {
        let ctx = create_test_context();

        // Pre-resolve the task
        ctx.resolve_task("my-task", serde_json::json!({"result": 42}));

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
}
