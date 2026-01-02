//! Mock workflow context for unit testing workflows in isolation.

use crate::error::{FlovynError, Result};
use crate::workflow::context::{
    DeterministicRandom, PromiseOptions, ScheduleTaskOptions, WorkflowContext,
};
use crate::workflow::future::{
    ChildWorkflowFuture, ChildWorkflowFutureRaw, OperationFuture, OperationFutureRaw,
    PromiseFuture, PromiseFutureRaw, TaskFuture, TaskFutureRaw, TimerFuture,
};
use async_trait::async_trait;
use parking_lot::RwLock;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use uuid::Uuid;

use super::TimeController;

/// Mock implementation of WorkflowContext for testing workflows in isolation.
///
/// This mock allows you to:
/// - Set up expected task results
/// - Set up expected promise resolutions
/// - Control time progression
/// - Inspect recorded operations
///
/// # Example
///
/// ```ignore
/// use flovyn_sdk::testing::MockWorkflowContext;
/// use serde_json::json;
///
/// let ctx = MockWorkflowContext::builder()
///     .workflow_execution_id(Uuid::new_v4())
///     .input(json!({"amount": 100}))
///     .task_result("send-email", json!({"sent": true}))
///     .build();
///
/// // Execute your workflow with the mock context
/// let result = my_workflow(ctx.clone()).await;
///
/// // Verify expectations
/// assert!(ctx.was_task_scheduled("send-email"));
/// ```
pub struct MockWorkflowContext {
    inner: Arc<MockWorkflowContextInner>,
}

struct MockWorkflowContextInner {
    workflow_execution_id: Uuid,
    tenant_id: Uuid,
    input: Value,
    time_controller: TimeController,
    state: RwLock<HashMap<String, Value>>,
    task_results: RwLock<HashMap<String, Value>>,
    promise_results: RwLock<HashMap<String, Value>>,
    child_workflow_results: RwLock<HashMap<String, Value>>,
    recorded_operations: RwLock<Vec<RecordedOperation>>,
    scheduled_tasks: RwLock<Vec<ScheduledTask>>,
    scheduled_workflows: RwLock<Vec<ScheduledWorkflow>>,
    created_promises: RwLock<Vec<String>>,
    cancellation_requested: AtomicBool,
    uuid_counter: AtomicU64,
    rng_seed: u64,
    // Async method sequence counters
    next_task_seq: AtomicU32,
    next_timer_seq: AtomicU32,
    next_child_workflow_seq: AtomicU32,
    next_promise_seq: AtomicU32,
    next_operation_seq: AtomicU32,
}

impl Clone for MockWorkflowContext {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// A recorded side effect operation
#[derive(Debug, Clone)]
pub struct RecordedOperation {
    pub name: String,
    pub result: Value,
}

/// A scheduled task
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub task_type: String,
    pub input: Value,
    pub options: ScheduleTaskOptions,
}

/// A scheduled child workflow
#[derive(Debug, Clone)]
pub struct ScheduledWorkflow {
    pub name: String,
    pub kind: String,
    pub input: Value,
}

impl MockWorkflowContext {
    /// Create a new builder for MockWorkflowContext.
    pub fn builder() -> MockWorkflowContextBuilder {
        MockWorkflowContextBuilder::default()
    }

    /// Create a simple mock context with default values.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Get the time controller for this context.
    pub fn time_controller(&self) -> &TimeController {
        &self.inner.time_controller
    }

    /// Get all recorded operations.
    pub fn recorded_operations(&self) -> Vec<RecordedOperation> {
        self.inner.recorded_operations.read().clone()
    }

    /// Check if a specific operation was recorded.
    pub fn was_operation_recorded(&self, name: &str) -> bool {
        self.inner
            .recorded_operations
            .read()
            .iter()
            .any(|op| op.name == name)
    }

    /// Get all scheduled tasks.
    pub fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        self.inner.scheduled_tasks.read().clone()
    }

    /// Check if a specific task type was scheduled.
    pub fn was_task_scheduled(&self, task_type: &str) -> bool {
        self.inner
            .scheduled_tasks
            .read()
            .iter()
            .any(|t| t.task_type == task_type)
    }

    /// Get all scheduled child workflows.
    pub fn scheduled_workflows(&self) -> Vec<ScheduledWorkflow> {
        self.inner.scheduled_workflows.read().clone()
    }

    /// Check if a specific child workflow kind was scheduled.
    pub fn was_workflow_scheduled(&self, kind: &str) -> bool {
        self.inner
            .scheduled_workflows
            .read()
            .iter()
            .any(|w| w.kind == kind)
    }

    /// Get all created promises.
    pub fn created_promises(&self) -> Vec<String> {
        self.inner.created_promises.read().clone()
    }

    /// Check if a specific promise was created.
    pub fn was_promise_created(&self, name: &str) -> bool {
        self.inner
            .created_promises
            .read()
            .contains(&name.to_string())
    }

    /// Request cancellation.
    pub fn request_cancellation(&self) {
        self.inner
            .cancellation_requested
            .store(true, Ordering::SeqCst);
    }

    /// Set a task result for testing.
    pub fn set_task_result(&self, task_type: &str, result: Value) {
        self.inner
            .task_results
            .write()
            .insert(task_type.to_string(), result);
    }

    /// Set a promise result for testing.
    pub fn set_promise_result(&self, name: &str, result: Value) {
        self.inner
            .promise_results
            .write()
            .insert(name.to_string(), result);
    }

    /// Set a child workflow result for testing.
    pub fn set_child_workflow_result(&self, kind: &str, result: Value) {
        self.inner
            .child_workflow_results
            .write()
            .insert(kind.to_string(), result);
    }

    /// Get the current state snapshot.
    pub fn state_snapshot(&self) -> HashMap<String, Value> {
        self.inner.state.read().clone()
    }
}

impl Default for MockWorkflowContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for MockWorkflowContext.
#[derive(Default)]
pub struct MockWorkflowContextBuilder {
    workflow_execution_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    input: Option<Value>,
    initial_time_millis: Option<i64>,
    initial_state: HashMap<String, Value>,
    task_results: HashMap<String, Value>,
    promise_results: HashMap<String, Value>,
    child_workflow_results: HashMap<String, Value>,
    rng_seed: Option<u64>,
}

impl MockWorkflowContextBuilder {
    /// Set the workflow execution ID.
    pub fn workflow_execution_id(mut self, id: Uuid) -> Self {
        self.workflow_execution_id = Some(id);
        self
    }

    /// Set the tenant ID.
    pub fn tenant_id(mut self, id: Uuid) -> Self {
        self.tenant_id = Some(id);
        self
    }

    /// Set the workflow input.
    pub fn input(mut self, input: Value) -> Self {
        self.input = Some(input);
        self
    }

    /// Set the initial time in milliseconds.
    pub fn initial_time_millis(mut self, time: i64) -> Self {
        self.initial_time_millis = Some(time);
        self
    }

    /// Set an initial state value.
    pub fn state(mut self, key: &str, value: Value) -> Self {
        self.initial_state.insert(key.to_string(), value);
        self
    }

    /// Set an expected task result.
    pub fn task_result(mut self, task_type: &str, result: Value) -> Self {
        self.task_results.insert(task_type.to_string(), result);
        self
    }

    /// Set an expected promise result.
    pub fn promise_result(mut self, name: &str, result: Value) -> Self {
        self.promise_results.insert(name.to_string(), result);
        self
    }

    /// Set an expected child workflow result.
    pub fn child_workflow_result(mut self, kind: &str, result: Value) -> Self {
        self.child_workflow_results.insert(kind.to_string(), result);
        self
    }

    /// Set the RNG seed for deterministic random generation.
    pub fn rng_seed(mut self, seed: u64) -> Self {
        self.rng_seed = Some(seed);
        self
    }

    /// Build the MockWorkflowContext.
    pub fn build(self) -> MockWorkflowContext {
        let time_controller = match self.initial_time_millis {
            Some(time) => TimeController::with_initial_time(time),
            None => TimeController::new(),
        };

        MockWorkflowContext {
            inner: Arc::new(MockWorkflowContextInner {
                workflow_execution_id: self.workflow_execution_id.unwrap_or_else(Uuid::new_v4),
                tenant_id: self.tenant_id.unwrap_or_else(Uuid::new_v4),
                input: self.input.unwrap_or(Value::Null),
                time_controller,
                state: RwLock::new(self.initial_state),
                task_results: RwLock::new(self.task_results),
                promise_results: RwLock::new(self.promise_results),
                child_workflow_results: RwLock::new(self.child_workflow_results),
                recorded_operations: RwLock::new(Vec::new()),
                scheduled_tasks: RwLock::new(Vec::new()),
                scheduled_workflows: RwLock::new(Vec::new()),
                created_promises: RwLock::new(Vec::new()),
                cancellation_requested: AtomicBool::new(false),
                uuid_counter: AtomicU64::new(0),
                rng_seed: self.rng_seed.unwrap_or(12345),
                next_task_seq: AtomicU32::new(0),
                next_timer_seq: AtomicU32::new(0),
                next_child_workflow_seq: AtomicU32::new(0),
                next_promise_seq: AtomicU32::new(0),
                next_operation_seq: AtomicU32::new(0),
            }),
        }
    }
}

/// Mock deterministic random implementation
struct MockDeterministicRandom {
    rng: RwLock<ChaCha8Rng>,
}

impl MockDeterministicRandom {
    fn new(seed: u64) -> Self {
        Self {
            rng: RwLock::new(ChaCha8Rng::seed_from_u64(seed)),
        }
    }
}

impl DeterministicRandom for MockDeterministicRandom {
    fn next_int(&self, min: i32, max: i32) -> i32 {
        self.rng.write().gen_range(min..max)
    }

    fn next_long(&self, min: i64, max: i64) -> i64 {
        self.rng.write().gen_range(min..max)
    }

    fn next_double(&self) -> f64 {
        self.rng.write().gen()
    }

    fn next_bool(&self) -> bool {
        self.rng.write().gen()
    }
}

#[async_trait]
impl WorkflowContext for MockWorkflowContext {
    fn workflow_execution_id(&self) -> Uuid {
        self.inner.workflow_execution_id
    }

    fn tenant_id(&self) -> Uuid {
        self.inner.tenant_id
    }

    fn input_raw(&self) -> &Value {
        &self.inner.input
    }

    fn current_time_millis(&self) -> i64 {
        self.inner.time_controller.current_time_millis()
    }

    fn random_uuid(&self) -> Uuid {
        let counter = self.inner.uuid_counter.fetch_add(1, Ordering::SeqCst);
        let namespace = Uuid::from_bytes([
            0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4,
            0x30, 0xc8,
        ]);
        Uuid::new_v5(&namespace, &counter.to_le_bytes())
    }

    fn random(&self) -> &dyn DeterministicRandom {
        // This is a simplified implementation - in real tests you might want
        // to store the random instance in the context
        // For now, we'll use a leaked box (acceptable for testing)
        let rng = Box::new(MockDeterministicRandom::new(self.inner.rng_seed));
        Box::leak(rng)
    }

    async fn get_raw(&self, key: &str) -> Result<Option<Value>> {
        Ok(self.inner.state.read().get(key).cloned())
    }

    async fn set_raw(&self, key: &str, value: Value) -> Result<()> {
        self.inner.state.write().insert(key.to_string(), value);
        Ok(())
    }

    async fn clear(&self, key: &str) -> Result<()> {
        self.inner.state.write().remove(key);
        Ok(())
    }

    async fn clear_all(&self) -> Result<()> {
        self.inner.state.write().clear();
        Ok(())
    }

    async fn state_keys(&self) -> Result<Vec<String>> {
        Ok(self.inner.state.read().keys().cloned().collect())
    }

    fn is_cancellation_requested(&self) -> bool {
        self.inner.cancellation_requested.load(Ordering::SeqCst)
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
        let task_seq = self.inner.next_task_seq.fetch_add(1, Ordering::SeqCst);
        let task_execution_id = self.random_uuid();

        // Record the scheduled task
        self.inner.scheduled_tasks.write().push(ScheduledTask {
            task_type: task_type.to_string(),
            input: input.clone(),
            options,
        });

        // Look up the mock result
        match self.inner.task_results.read().get(task_type) {
            Some(result) => TaskFuture::from_replay(
                task_seq,
                task_execution_id,
                Weak::<DummyMockTaskContext>::new(),
                Ok(result.clone()),
            ),
            None => TaskFuture::with_error(FlovynError::Other(format!(
                "No mock result configured for task type: {}",
                task_type
            ))),
        }
    }

    fn sleep(&self, duration: Duration) -> TimerFuture {
        let timer_seq = self.inner.next_timer_seq.fetch_add(1, Ordering::SeqCst);
        let timer_id = format!(
            "sleep-{}",
            self.inner.uuid_counter.fetch_add(1, Ordering::SeqCst)
        );

        // Register the timer
        self.inner
            .time_controller
            .register_timer_after(&timer_id, duration);

        // For mock context, timers complete immediately (mock behavior)
        TimerFuture::from_replay(
            timer_seq,
            timer_id,
            Weak::<DummyMockTimerContext>::new(),
            true,
        )
    }

    fn schedule_workflow_raw(
        &self,
        name: &str,
        kind: &str,
        input: Value,
    ) -> ChildWorkflowFutureRaw {
        let cw_seq = self
            .inner
            .next_child_workflow_seq
            .fetch_add(1, Ordering::SeqCst);
        let child_execution_id = self.random_uuid();

        // Record the scheduled workflow
        self.inner
            .scheduled_workflows
            .write()
            .push(ScheduledWorkflow {
                name: name.to_string(),
                kind: kind.to_string(),
                input: input.clone(),
            });

        // Look up the mock result
        match self.inner.child_workflow_results.read().get(kind) {
            Some(result) => ChildWorkflowFuture::from_replay(
                cw_seq,
                child_execution_id,
                name.to_string(),
                Weak::<DummyMockChildWorkflowContext>::new(),
                Ok(result.clone()),
            ),
            None => ChildWorkflowFuture::with_error(FlovynError::Other(format!(
                "No mock result configured for child workflow kind: {}",
                kind
            ))),
        }
    }

    fn promise_raw(&self, name: &str) -> PromiseFutureRaw {
        let promise_seq = self.inner.next_promise_seq.fetch_add(1, Ordering::SeqCst);

        // Record the promise creation
        self.inner.created_promises.write().push(name.to_string());

        // Look up the mock result
        match self.inner.promise_results.read().get(name) {
            Some(result) => PromiseFuture::from_replay(
                promise_seq,
                name.to_string(),
                Weak::<DummyMockPromiseFutureContext>::new(),
                Ok(result.clone()),
            ),
            None => PromiseFuture::with_error(FlovynError::Other(format!(
                "No mock result configured for promise: {}",
                name
            ))),
        }
    }

    fn promise_with_timeout_raw(&self, name: &str, _timeout: Duration) -> PromiseFutureRaw {
        self.promise_raw(name)
    }

    fn promise_with_options_raw(&self, name: &str, _options: PromiseOptions) -> PromiseFutureRaw {
        // Mock context ignores options - just delegate to promise_raw
        self.promise_raw(name)
    }

    fn run_raw(&self, name: &str, result: Value) -> OperationFutureRaw {
        let op_seq = self.inner.next_operation_seq.fetch_add(1, Ordering::SeqCst);

        // Record the operation
        self.inner
            .recorded_operations
            .write()
            .push(RecordedOperation {
                name: name.to_string(),
                result: result.clone(),
            });

        // Operations complete immediately with their result
        OperationFuture::new(op_seq, name.to_string(), Ok(result))
    }
}

// Dummy context types for mock futures
use crate::workflow::future::{
    ChildWorkflowFutureContext, PromiseFutureContext, SuspensionContext, TaskFutureContext,
    TimerFutureContext,
};

struct DummyMockTaskContext;
impl SuspensionContext for DummyMockTaskContext {
    fn signal_suspension(&self, _reason: String) {}
}
impl TaskFutureContext for DummyMockTaskContext {
    fn find_task_result(&self, _: &Uuid) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_task(&self, _: &Uuid) {}
}

struct DummyMockTimerContext;
impl SuspensionContext for DummyMockTimerContext {
    fn signal_suspension(&self, _reason: String) {}
}
impl TimerFutureContext for DummyMockTimerContext {
    fn find_timer_result(&self, _: &str) -> Option<Result<()>> {
        None
    }
    fn record_cancel_timer(&self, _: &str) {}
}

struct DummyMockChildWorkflowContext;
impl SuspensionContext for DummyMockChildWorkflowContext {
    fn signal_suspension(&self, _reason: String) {}
}
impl ChildWorkflowFutureContext for DummyMockChildWorkflowContext {
    fn find_child_workflow_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_child_workflow(&self, _: &Uuid) {}
}

struct DummyMockPromiseFutureContext;
impl SuspensionContext for DummyMockPromiseFutureContext {
    fn signal_suspension(&self, _reason: String) {}
}
impl PromiseFutureContext for DummyMockPromiseFutureContext {
    fn find_promise_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_mock_workflow_context_new() {
        let ctx = MockWorkflowContext::new();
        assert!(!ctx.workflow_execution_id().is_nil());
        assert!(!ctx.tenant_id().is_nil());
    }

    #[test]
    fn test_mock_workflow_context_builder() {
        let id = Uuid::new_v4();
        let tenant = Uuid::new_v4();
        let ctx = MockWorkflowContext::builder()
            .workflow_execution_id(id)
            .tenant_id(tenant)
            .input(json!({"key": "value"}))
            .initial_time_millis(1000)
            .build();

        assert_eq!(ctx.workflow_execution_id(), id);
        assert_eq!(ctx.tenant_id(), tenant);
        assert_eq!(ctx.input_raw(), &json!({"key": "value"}));
        assert_eq!(ctx.current_time_millis(), 1000);
    }

    #[tokio::test]
    async fn test_mock_workflow_context_state() {
        let ctx = MockWorkflowContext::builder()
            .state("initial", json!("value"))
            .build();

        assert_eq!(ctx.get_raw("initial").await.unwrap(), Some(json!("value")));
        assert_eq!(ctx.get_raw("missing").await.unwrap(), None);

        ctx.set_raw("new_key", json!(42)).await.unwrap();
        assert_eq!(ctx.get_raw("new_key").await.unwrap(), Some(json!(42)));

        ctx.clear("new_key").await.unwrap();
        assert_eq!(ctx.get_raw("new_key").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_mock_workflow_context_task_scheduling() {
        let ctx = MockWorkflowContext::builder()
            .task_result("send-email", json!({"sent": true}))
            .build();

        let result = ctx
            .schedule_raw("send-email", json!({"to": "test@example.com"}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"sent": true}));
        assert!(ctx.was_task_scheduled("send-email"));
    }

    #[tokio::test]
    async fn test_mock_workflow_context_task_not_configured() {
        let ctx = MockWorkflowContext::new();
        let result = ctx.schedule_raw("unknown-task", json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_workflow_context_promise() {
        let ctx = MockWorkflowContext::builder()
            .promise_result("approval", json!({"approved": true}))
            .build();

        let result = ctx.promise_raw("approval").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"approved": true}));
        assert!(ctx.was_promise_created("approval"));
    }

    #[tokio::test]
    async fn test_mock_workflow_context_child_workflow() {
        let ctx = MockWorkflowContext::builder()
            .child_workflow_result("payment-workflow", json!({"success": true}))
            .build();

        let result = ctx
            .schedule_workflow_raw("payment", "payment-workflow", json!({"amount": 100}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"success": true}));
        assert!(ctx.was_workflow_scheduled("payment-workflow"));
    }

    #[tokio::test]
    async fn test_mock_workflow_context_run_raw() {
        let ctx = MockWorkflowContext::new();
        let result = ctx.run_raw("operation1", json!({"result": "data"})).await;
        assert!(result.is_ok());
        assert!(ctx.was_operation_recorded("operation1"));
    }

    #[test]
    fn test_mock_workflow_context_cancellation() {
        let ctx = MockWorkflowContext::new();
        assert!(!ctx.is_cancellation_requested());
        ctx.request_cancellation();
        assert!(ctx.is_cancellation_requested());
    }

    #[tokio::test]
    async fn test_mock_workflow_context_check_cancellation() {
        let ctx = MockWorkflowContext::new();
        assert!(ctx.check_cancellation().await.is_ok());

        ctx.request_cancellation();
        assert!(ctx.check_cancellation().await.is_err());
    }

    #[test]
    fn test_mock_workflow_context_random_uuid() {
        let ctx = MockWorkflowContext::new();
        let uuid1 = ctx.random_uuid();
        let uuid2 = ctx.random_uuid();
        assert_ne!(uuid1, uuid2);
    }

    #[test]
    fn test_mock_workflow_context_time_controller() {
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1000)
            .build();

        assert_eq!(ctx.current_time_millis(), 1000);
        ctx.time_controller().advance(Duration::from_secs(5));
        assert_eq!(ctx.current_time_millis(), 6000);
    }

    #[tokio::test]
    async fn test_mock_workflow_context_sleep_registers_timer() {
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1000)
            .build();

        ctx.sleep(Duration::from_secs(10)).await.unwrap();
        assert_eq!(ctx.time_controller().pending_timer_ids().len(), 1);
    }

    #[tokio::test]
    async fn test_mock_workflow_context_state_keys() {
        let ctx = MockWorkflowContext::builder()
            .state("key1", json!("value1"))
            .state("key2", json!("value2"))
            .build();

        let keys = ctx.state_keys().await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
    }

    #[tokio::test]
    async fn test_mock_workflow_context_clear_all() {
        let ctx = MockWorkflowContext::builder()
            .state("key1", json!("value1"))
            .state("key2", json!("value2"))
            .build();

        ctx.clear_all().await.unwrap();
        assert!(ctx.state_keys().await.unwrap().is_empty());
    }

    #[test]
    fn test_mock_workflow_context_set_task_result_after_build() {
        let ctx = MockWorkflowContext::new();
        ctx.set_task_result("new-task", json!({"result": "ok"}));
    }
}
