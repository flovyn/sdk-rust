//! Workflow future types for parallel execution
//!
//! This module provides future types that enable parallel execution of workflow operations.
//! Each operation type (task, timer, child workflow, promise, operation) has a corresponding
//! future type that can be scheduled and awaited independently.

use crate::error::{FlovynError, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use uuid::Uuid;

/// Trait for workflow futures that can be cancelled.
///
/// Unlike standard Rust futures, workflow futures represent operations that may
/// execute externally (tasks, child workflows) or are managed by the workflow
/// runtime (timers). This trait provides a consistent interface for cancellation.
pub trait CancellableFuture: Future {
    /// Cancel this operation.
    ///
    /// For tasks and child workflows, this sends a cancellation request to the server.
    /// The operation may still complete successfully before the cancellation is processed.
    ///
    /// For timers, cancellation is synchronous - the future immediately resolves
    /// with a `Cancelled` result.
    ///
    /// Promises and operations cannot be cancelled.
    fn cancel(&self);

    /// Check if this future has been cancelled.
    fn is_cancelled(&self) -> bool;
}

/// State shared between the future and the context for completion tracking
#[derive(Debug)]
pub(crate) struct FutureState {
    /// Whether the future has been cancelled
    pub cancelled: AtomicBool,
    /// Pre-computed error (e.g., determinism violation detected at creation)
    pub error: parking_lot::Mutex<Option<FlovynError>>,
    /// Pre-computed result (for replay cases where result is already known)
    pub result: parking_lot::Mutex<Option<Result<Value>>>,
}

impl FutureState {
    pub fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            error: parking_lot::Mutex::new(None),
            result: parking_lot::Mutex::new(None),
        }
    }

    pub fn with_error(error: FlovynError) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            error: parking_lot::Mutex::new(Some(error)),
            result: parking_lot::Mutex::new(None),
        }
    }

    pub fn with_result(result: Result<Value>) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            error: parking_lot::Mutex::new(None),
            result: parking_lot::Mutex::new(Some(result)),
        }
    }
}

impl Default for FutureState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// TaskFuture
// ============================================================================

/// Future for a scheduled task.
///
/// Created by `WorkflowContext::schedule()` and related methods.
/// Resolves when the task completes or fails.
///
/// # Cancellation
///
/// Calling `cancel()` sends a `RequestCancelTask` command to the server.
/// The task may still complete successfully if it finishes before the
/// cancellation is processed.
#[allow(dead_code)]
pub struct TaskFuture<T> {
    /// Per-type sequence number for this task
    pub(crate) task_seq: u32,
    /// Unique task execution ID
    pub(crate) task_execution_id: Uuid,
    /// Weak reference to the context
    pub(crate) context: Weak<dyn TaskFutureContext + Send + Sync>,
    /// Shared state
    pub(crate) state: Arc<FutureState>,
    /// Phantom marker for output type
    pub(crate) _marker: PhantomData<T>,
}

/// Trait for context operations needed by TaskFuture
pub(crate) trait TaskFutureContext {
    fn find_task_result(&self, task_execution_id: &Uuid) -> Option<Result<Value>>;
    fn record_cancel_task(&self, task_execution_id: &Uuid);
}

impl<T: DeserializeOwned> TaskFuture<T> {
    /// Create a new TaskFuture
    pub(crate) fn new(
        task_seq: u32,
        task_execution_id: Uuid,
        context: Weak<dyn TaskFutureContext + Send + Sync>,
    ) -> Self {
        Self {
            task_seq,
            task_execution_id,
            context,
            state: Arc::new(FutureState::new()),
            _marker: PhantomData,
        }
    }

    /// Create a TaskFuture for replay with result already known
    pub(crate) fn from_replay(
        task_seq: u32,
        task_execution_id: Uuid,
        context: Weak<dyn TaskFutureContext + Send + Sync>,
        result: Result<Value>,
    ) -> Self {
        Self {
            task_seq,
            task_execution_id,
            context,
            state: Arc::new(FutureState::with_result(result)),
            _marker: PhantomData,
        }
    }

    /// Create a TaskFuture with an error (e.g., determinism violation)
    pub(crate) fn with_error(error: FlovynError) -> Self {
        Self {
            task_seq: 0,
            task_execution_id: Uuid::nil(),
            context: Weak::<DummyTaskFutureContext>::new(),
            state: Arc::new(FutureState::with_error(error)),
            _marker: PhantomData,
        }
    }
}

struct DummyTaskFutureContext;
impl TaskFutureContext for DummyTaskFutureContext {
    fn find_task_result(&self, _: &Uuid) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_task(&self, _: &Uuid) {}
}

impl<T: DeserializeOwned> Future for TaskFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check for pre-computed error
        if let Some(error) = self.state.error.lock().take() {
            return Poll::Ready(Err(error));
        }

        // Check for pre-computed result (replay case)
        if let Some(result) = self.state.result.lock().take() {
            return Poll::Ready(match result {
                Ok(value) => serde_json::from_value(value).map_err(FlovynError::Serialization),
                Err(e) => Err(e),
            });
        }

        // Check if cancelled
        if self.state.cancelled.load(Ordering::SeqCst) {
            return Poll::Ready(Err(FlovynError::TaskCancelled));
        }

        // Check context for completion
        if let Some(ctx) = self.context.upgrade() {
            if let Some(result) = ctx.find_task_result(&self.task_execution_id) {
                return Poll::Ready(match result {
                    Ok(value) => serde_json::from_value(value).map_err(FlovynError::Serialization),
                    Err(e) => Err(e),
                });
            }
        }

        // Not ready yet - signal workflow suspension
        Poll::Ready(Err(FlovynError::Suspended {
            reason: format!("Waiting for task {} to complete", self.task_execution_id),
        }))
    }
}

impl<T> CancellableFuture for TaskFuture<T>
where
    T: DeserializeOwned,
{
    fn cancel(&self) {
        if self.state.cancelled.swap(true, Ordering::SeqCst) {
            return; // Already cancelled
        }

        if let Some(ctx) = self.context.upgrade() {
            ctx.record_cancel_task(&self.task_execution_id);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }
}

// ============================================================================
// TimerFuture
// ============================================================================

/// Future for a timer/sleep operation.
///
/// Created by `WorkflowContext::sleep()`.
/// Resolves when the timer fires or is cancelled.
///
/// # Cancellation
///
/// Timer cancellation is synchronous - calling `cancel()` immediately
/// resolves the future with an error result. A `CancelTimer` command
/// is also recorded.
#[allow(dead_code)]
pub struct TimerFuture {
    /// Per-type sequence number for this timer
    pub(crate) timer_seq: u32,
    /// Timer ID
    pub(crate) timer_id: String,
    /// Weak reference to the context
    pub(crate) context: Weak<dyn TimerFutureContext + Send + Sync>,
    /// Shared state
    pub(crate) state: Arc<FutureState>,
}

/// Trait for context operations needed by TimerFuture
pub(crate) trait TimerFutureContext {
    fn find_timer_result(&self, timer_id: &str) -> Option<Result<()>>;
    fn record_cancel_timer(&self, timer_id: &str);
}

impl TimerFuture {
    /// Create a new TimerFuture
    pub(crate) fn new(
        timer_seq: u32,
        timer_id: String,
        context: Weak<dyn TimerFutureContext + Send + Sync>,
    ) -> Self {
        Self {
            timer_seq,
            timer_id,
            context,
            state: Arc::new(FutureState::new()),
        }
    }

    /// Create a TimerFuture for replay with result already known
    pub(crate) fn from_replay(
        timer_seq: u32,
        timer_id: String,
        context: Weak<dyn TimerFutureContext + Send + Sync>,
        fired: bool,
    ) -> Self {
        let result = if fired {
            Ok(Value::Null)
        } else {
            Err(FlovynError::TimerError("Timer cancelled".to_string()))
        };
        Self {
            timer_seq,
            timer_id,
            context,
            state: Arc::new(FutureState::with_result(result)),
        }
    }

    /// Create a TimerFuture with an error
    pub(crate) fn with_error(error: FlovynError) -> Self {
        Self {
            timer_seq: 0,
            timer_id: String::new(),
            context: Weak::<DummyTimerFutureContext>::new(),
            state: Arc::new(FutureState::with_error(error)),
        }
    }
}

struct DummyTimerFutureContext;
impl TimerFutureContext for DummyTimerFutureContext {
    fn find_timer_result(&self, _: &str) -> Option<Result<()>> {
        None
    }
    fn record_cancel_timer(&self, _: &str) {}
}

impl Future for TimerFuture {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check for pre-computed error
        if let Some(error) = self.state.error.lock().take() {
            return Poll::Ready(Err(error));
        }

        // Check for pre-computed result (replay or cancelled)
        if let Some(result) = self.state.result.lock().take() {
            return Poll::Ready(result.map(|_| ()));
        }

        // Check if cancelled (synchronous cancellation)
        if self.state.cancelled.load(Ordering::SeqCst) {
            return Poll::Ready(Err(FlovynError::TimerError("Timer cancelled".to_string())));
        }

        // Check context for completion
        if let Some(ctx) = self.context.upgrade() {
            if let Some(result) = ctx.find_timer_result(&self.timer_id) {
                return Poll::Ready(result);
            }
        }

        // Not ready yet - signal workflow suspension
        Poll::Ready(Err(FlovynError::Suspended {
            reason: format!("Waiting for timer {} to fire", self.timer_id),
        }))
    }
}

impl CancellableFuture for TimerFuture {
    fn cancel(&self) {
        if self.state.cancelled.swap(true, Ordering::SeqCst) {
            return; // Already cancelled
        }

        // Record cancellation command
        if let Some(ctx) = self.context.upgrade() {
            ctx.record_cancel_timer(&self.timer_id);
        }

        // Set result to cancelled for immediate unblocking
        *self.state.result.lock() =
            Some(Err(FlovynError::TimerError("Timer cancelled".to_string())));
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }
}

// ============================================================================
// ChildWorkflowFuture
// ============================================================================

/// Future for a child workflow.
///
/// Created by `WorkflowContext::schedule_workflow()`.
/// Resolves when the child workflow completes or fails.
///
/// # Cancellation
///
/// Calling `cancel()` sends a `RequestCancelChildWorkflow` command.
/// The child workflow may still complete if it finishes before
/// cancellation is processed.
#[allow(dead_code)]
pub struct ChildWorkflowFuture<T> {
    /// Per-type sequence number
    pub(crate) child_workflow_seq: u32,
    /// Child workflow execution ID
    pub(crate) child_execution_id: Uuid,
    /// Child workflow name (for lookup)
    pub(crate) child_execution_name: String,
    /// Weak reference to the context
    pub(crate) context: Weak<dyn ChildWorkflowFutureContext + Send + Sync>,
    /// Shared state
    pub(crate) state: Arc<FutureState>,
    /// Phantom marker for output type
    pub(crate) _marker: PhantomData<T>,
}

/// Trait for context operations needed by ChildWorkflowFuture
pub(crate) trait ChildWorkflowFutureContext {
    fn find_child_workflow_result(&self, name: &str) -> Option<Result<Value>>;
    fn record_cancel_child_workflow(&self, child_execution_id: &Uuid);
}

impl<T: DeserializeOwned> ChildWorkflowFuture<T> {
    /// Create a new ChildWorkflowFuture
    pub(crate) fn new(
        child_workflow_seq: u32,
        child_execution_id: Uuid,
        child_execution_name: String,
        context: Weak<dyn ChildWorkflowFutureContext + Send + Sync>,
    ) -> Self {
        Self {
            child_workflow_seq,
            child_execution_id,
            child_execution_name,
            context,
            state: Arc::new(FutureState::new()),
            _marker: PhantomData,
        }
    }

    /// Create for replay with result already known
    pub(crate) fn from_replay(
        child_workflow_seq: u32,
        child_execution_id: Uuid,
        child_execution_name: String,
        context: Weak<dyn ChildWorkflowFutureContext + Send + Sync>,
        result: Result<Value>,
    ) -> Self {
        Self {
            child_workflow_seq,
            child_execution_id,
            child_execution_name,
            context,
            state: Arc::new(FutureState::with_result(result)),
            _marker: PhantomData,
        }
    }

    /// Create with an error
    pub(crate) fn with_error(error: FlovynError) -> Self {
        Self {
            child_workflow_seq: 0,
            child_execution_id: Uuid::nil(),
            child_execution_name: String::new(),
            context: Weak::<DummyChildWorkflowFutureContext>::new(),
            state: Arc::new(FutureState::with_error(error)),
            _marker: PhantomData,
        }
    }
}

struct DummyChildWorkflowFutureContext;
impl ChildWorkflowFutureContext for DummyChildWorkflowFutureContext {
    fn find_child_workflow_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
    fn record_cancel_child_workflow(&self, _: &Uuid) {}
}

impl<T: DeserializeOwned> Future for ChildWorkflowFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check for pre-computed error
        if let Some(error) = self.state.error.lock().take() {
            return Poll::Ready(Err(error));
        }

        // Check for pre-computed result
        if let Some(result) = self.state.result.lock().take() {
            return Poll::Ready(match result {
                Ok(value) => serde_json::from_value(value).map_err(FlovynError::Serialization),
                Err(e) => Err(e),
            });
        }

        // Check if cancelled
        if self.state.cancelled.load(Ordering::SeqCst) {
            return Poll::Ready(Err(FlovynError::WorkflowCancelled(
                "Child workflow cancelled".to_string(),
            )));
        }

        // Check context for completion
        if let Some(ctx) = self.context.upgrade() {
            if let Some(result) = ctx.find_child_workflow_result(&self.child_execution_name) {
                return Poll::Ready(match result {
                    Ok(value) => serde_json::from_value(value).map_err(FlovynError::Serialization),
                    Err(e) => Err(e),
                });
            }
        }

        // Not ready yet - signal workflow suspension
        Poll::Ready(Err(FlovynError::Suspended {
            reason: format!(
                "Waiting for child workflow {} to complete",
                self.child_execution_name
            ),
        }))
    }
}

impl<T: DeserializeOwned> CancellableFuture for ChildWorkflowFuture<T> {
    fn cancel(&self) {
        if self.state.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }

        if let Some(ctx) = self.context.upgrade() {
            ctx.record_cancel_child_workflow(&self.child_execution_id);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }
}

// ============================================================================
// PromiseFuture
// ============================================================================

/// Future for a durable promise.
///
/// Created by `WorkflowContext::promise()`.
/// Resolves when the promise is resolved externally via a signal.
///
/// # Cancellation
///
/// Promises cannot be cancelled as they require external resolution.
/// Calling `cancel()` has no effect.
#[allow(dead_code)]
pub struct PromiseFuture<T> {
    /// Per-type sequence number
    pub(crate) promise_seq: u32,
    /// Promise ID/name
    pub(crate) promise_id: String,
    /// Weak reference to the context
    pub(crate) context: Weak<dyn PromiseFutureContext + Send + Sync>,
    /// Shared state
    pub(crate) state: Arc<FutureState>,
    /// Phantom marker for output type
    pub(crate) _marker: PhantomData<T>,
}

/// Trait for context operations needed by PromiseFuture
pub(crate) trait PromiseFutureContext {
    fn find_promise_result(&self, promise_id: &str) -> Option<Result<Value>>;
}

impl<T: DeserializeOwned> PromiseFuture<T> {
    /// Create a new PromiseFuture
    pub(crate) fn new(
        promise_seq: u32,
        promise_id: String,
        context: Weak<dyn PromiseFutureContext + Send + Sync>,
    ) -> Self {
        Self {
            promise_seq,
            promise_id,
            context,
            state: Arc::new(FutureState::new()),
            _marker: PhantomData,
        }
    }

    /// Create for replay with result already known
    pub(crate) fn from_replay(
        promise_seq: u32,
        promise_id: String,
        context: Weak<dyn PromiseFutureContext + Send + Sync>,
        result: Result<Value>,
    ) -> Self {
        Self {
            promise_seq,
            promise_id,
            context,
            state: Arc::new(FutureState::with_result(result)),
            _marker: PhantomData,
        }
    }

    /// Create with an error
    pub(crate) fn with_error(error: FlovynError) -> Self {
        Self {
            promise_seq: 0,
            promise_id: String::new(),
            context: Weak::<DummyPromiseFutureContext>::new(),
            state: Arc::new(FutureState::with_error(error)),
            _marker: PhantomData,
        }
    }
}

struct DummyPromiseFutureContext;
impl PromiseFutureContext for DummyPromiseFutureContext {
    fn find_promise_result(&self, _: &str) -> Option<Result<Value>> {
        None
    }
}

impl<T: DeserializeOwned> Future for PromiseFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check for pre-computed error
        if let Some(error) = self.state.error.lock().take() {
            return Poll::Ready(Err(error));
        }

        // Check for pre-computed result
        if let Some(result) = self.state.result.lock().take() {
            return Poll::Ready(match result {
                Ok(value) => serde_json::from_value(value).map_err(FlovynError::Serialization),
                Err(e) => Err(e),
            });
        }

        // Check context for completion
        if let Some(ctx) = self.context.upgrade() {
            if let Some(result) = ctx.find_promise_result(&self.promise_id) {
                return Poll::Ready(match result {
                    Ok(value) => serde_json::from_value(value).map_err(FlovynError::Serialization),
                    Err(e) => Err(e),
                });
            }
        }

        // Not ready yet - signal workflow suspension
        Poll::Ready(Err(FlovynError::Suspended {
            reason: format!("Waiting for promise {} to be resolved", self.promise_id),
        }))
    }
}

impl<T: DeserializeOwned> CancellableFuture for PromiseFuture<T> {
    fn cancel(&self) {
        // Promises cannot be cancelled - no-op
    }

    fn is_cancelled(&self) -> bool {
        false // Never cancelled
    }
}

// ============================================================================
// OperationFuture
// ============================================================================

/// Future for a run() operation.
///
/// Created by `WorkflowContext::run()`.
/// Operations are synchronous and complete immediately, so this future
/// is always ready with its result.
///
/// # Cancellation
///
/// Operations cannot be cancelled as the result is already computed
/// or cached. Calling `cancel()` has no effect.
#[allow(dead_code)]
pub struct OperationFuture<T> {
    /// Per-type sequence number
    pub(crate) operation_seq: u32,
    /// Operation name
    pub(crate) operation_name: String,
    /// Shared state with result
    pub(crate) state: Arc<FutureState>,
    /// Phantom marker for output type
    pub(crate) _marker: PhantomData<T>,
}

impl<T: DeserializeOwned> OperationFuture<T> {
    /// Create a new OperationFuture with the result
    pub(crate) fn new(operation_seq: u32, operation_name: String, result: Result<Value>) -> Self {
        Self {
            operation_seq,
            operation_name,
            state: Arc::new(FutureState::with_result(result)),
            _marker: PhantomData,
        }
    }

    /// Create with an error
    pub(crate) fn with_error(error: FlovynError) -> Self {
        Self {
            operation_seq: 0,
            operation_name: String::new(),
            state: Arc::new(FutureState::with_error(error)),
            _marker: PhantomData,
        }
    }
}

impl<T: DeserializeOwned> Future for OperationFuture<T> {
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Check for pre-computed error
        if let Some(error) = self.state.error.lock().take() {
            return Poll::Ready(Err(error));
        }

        // Operations always have their result available immediately
        if let Some(result) = self.state.result.lock().take() {
            return Poll::Ready(match result {
                Ok(value) => serde_json::from_value(value).map_err(FlovynError::Serialization),
                Err(e) => Err(e),
            });
        }

        // Should not happen for operations - they're always ready
        Poll::Ready(Err(FlovynError::Other(
            "OperationFuture polled without result".to_string(),
        )))
    }
}

impl<T: DeserializeOwned> CancellableFuture for OperationFuture<T> {
    fn cancel(&self) {
        // Operations cannot be cancelled - no-op
    }

    fn is_cancelled(&self) -> bool {
        false // Never cancelled
    }
}

// ============================================================================
// Type aliases for Value-typed futures (raw versions)
// ============================================================================

/// Raw task future returning JSON Value
pub type TaskFutureRaw = TaskFuture<Value>;

/// Raw child workflow future returning JSON Value
pub type ChildWorkflowFutureRaw = ChildWorkflowFuture<Value>;

/// Raw promise future returning JSON Value
pub type PromiseFutureRaw = PromiseFuture<Value>;

/// Raw operation future returning JSON Value
pub type OperationFutureRaw = OperationFuture<Value>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    // ========================================================================
    // FutureState tests
    // ========================================================================

    #[test]
    fn test_future_state_new() {
        let state = FutureState::new();
        assert!(!state.cancelled.load(Ordering::SeqCst));
        assert!(state.error.lock().is_none());
        assert!(state.result.lock().is_none());
    }

    #[test]
    fn test_future_state_with_error() {
        let state = FutureState::with_error(FlovynError::TaskCancelled);
        assert!(state.error.lock().is_some());
    }

    #[test]
    fn test_future_state_with_result() {
        let state = FutureState::with_result(Ok(serde_json::json!(42)));
        let result = state.result.lock().take();
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());
    }

    #[test]
    fn test_future_state_default() {
        let state = FutureState::default();
        assert!(!state.cancelled.load(Ordering::SeqCst));
        assert!(state.error.lock().is_none());
        assert!(state.result.lock().is_none());
    }

    // ========================================================================
    // TaskFuture tests
    // ========================================================================

    #[test]
    fn test_task_future_returns_suspended_when_not_completed() {
        // Create a TaskFuture with no result - it should return Suspended
        let future: TaskFuture<i32> =
            TaskFuture::new(0, Uuid::new_v4(), Weak::<DummyTaskFutureContext>::new());
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Suspended { .. })) => {} // Expected
            other => panic!("Expected Ready(Err(Suspended)), got {:?}", other),
        }
    }

    #[test]
    fn test_task_future_returns_ready_when_completed() {
        // Create a TaskFuture from replay with result already known
        let future: TaskFuture<i32> = TaskFuture::from_replay(
            0,
            Uuid::new_v4(),
            Weak::<DummyTaskFutureContext>::new(),
            Ok(serde_json::json!(42)),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(42)) => {} // Expected
            other => panic!("Expected Ready(Ok(42)), got {:?}", other),
        }
    }

    #[test]
    fn test_task_future_returns_error_when_failed() {
        // Create a TaskFuture from replay with error result
        let future: TaskFuture<i32> = TaskFuture::from_replay(
            0,
            Uuid::new_v4(),
            Weak::<DummyTaskFutureContext>::new(),
            Err(FlovynError::TaskFailed("task failed".to_string())),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::TaskFailed(_))) => {} // Expected
            other => panic!("Expected Ready(Err(TaskFailed)), got {:?}", other),
        }
    }

    #[test]
    fn test_task_future_with_error_polls_ready() {
        let future: TaskFuture<i32> =
            TaskFuture::with_error(FlovynError::Other("test".to_string()));
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(_)) => {} // Expected
            other => panic!("Expected Ready(Err), got {:?}", other),
        }
    }

    /// Mock context that tracks cancel calls
    struct MockTaskFutureContext {
        cancel_count: AtomicUsize,
    }

    impl MockTaskFutureContext {
        fn new() -> Self {
            Self {
                cancel_count: AtomicUsize::new(0),
            }
        }
    }

    impl TaskFutureContext for MockTaskFutureContext {
        fn find_task_result(&self, _: &Uuid) -> Option<Result<Value>> {
            None
        }

        fn record_cancel_task(&self, _: &Uuid) {
            self.cancel_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_task_future_cancel_records_command() {
        let ctx = Arc::new(MockTaskFutureContext::new());
        let future: TaskFuture<i32> = TaskFuture::new(
            0,
            Uuid::new_v4(),
            Arc::downgrade(&ctx) as Weak<dyn TaskFutureContext + Send + Sync>,
        );

        assert_eq!(ctx.cancel_count.load(Ordering::SeqCst), 0);
        future.cancel();
        assert_eq!(ctx.cancel_count.load(Ordering::SeqCst), 1);

        // Calling cancel again should not record another command
        future.cancel();
        assert_eq!(ctx.cancel_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_task_future_is_cancelled_returns_true_after_cancel() {
        let future: TaskFuture<i32> =
            TaskFuture::new(0, Uuid::new_v4(), Weak::<DummyTaskFutureContext>::new());

        assert!(!future.is_cancelled());
        future.cancel();
        assert!(future.is_cancelled());
    }

    #[test]
    fn test_task_future_returns_cancelled_error_when_cancelled() {
        let future: TaskFuture<i32> =
            TaskFuture::new(0, Uuid::new_v4(), Weak::<DummyTaskFutureContext>::new());
        future.cancel();

        let mut future = std::pin::pin!(future);
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::TaskCancelled)) => {} // Expected
            other => panic!("Expected Ready(Err(TaskCancelled)), got {:?}", other),
        }
    }

    // ========================================================================
    // TimerFuture tests
    // ========================================================================

    #[test]
    fn test_timer_future_returns_suspended_when_not_fired() {
        let future = TimerFuture::new(
            0,
            "timer-1".to_string(),
            Weak::<DummyTimerFutureContext>::new(),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Suspended { .. })) => {} // Expected
            other => panic!("Expected Ready(Err(Suspended)), got {:?}", other),
        }
    }

    #[test]
    fn test_timer_future_returns_ready_when_fired() {
        let future = TimerFuture::from_replay(
            0,
            "timer-1".to_string(),
            Weak::<DummyTimerFutureContext>::new(),
            true, // fired
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(())) => {} // Expected
            other => panic!("Expected Ready(Ok(())), got {:?}", other),
        }
    }

    #[test]
    fn test_timer_future_cancellation_is_synchronous() {
        let future = TimerFuture::new(
            0,
            "timer-1".to_string(),
            Weak::<DummyTimerFutureContext>::new(),
        );

        assert!(!future.is_cancelled());
        future.cancel();
        assert!(future.is_cancelled());

        // The result should be set to cancelled
        let result = future.state.result.lock().take();
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn test_timer_future_cancel_returns_error_on_poll() {
        let future = TimerFuture::new(
            0,
            "timer-1".to_string(),
            Weak::<DummyTimerFutureContext>::new(),
        );
        future.cancel();

        let mut future = std::pin::pin!(future);
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::TimerError(_))) => {} // Expected
            other => panic!("Expected Ready(Err(TimerError)), got {:?}", other),
        }
    }

    #[test]
    fn test_timer_future_with_error() {
        let future = TimerFuture::with_error(FlovynError::Other("test error".to_string()));
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(msg))) if msg == "test error" => {} // Expected
            other => panic!("Expected Ready(Err(Other)), got {:?}", other),
        }
    }

    // ========================================================================
    // ChildWorkflowFuture tests
    // ========================================================================

    #[test]
    fn test_child_workflow_future_returns_suspended_when_running() {
        let future: ChildWorkflowFuture<i32> = ChildWorkflowFuture::new(
            0,
            Uuid::new_v4(),
            "child-1".to_string(),
            Weak::<DummyChildWorkflowFutureContext>::new(),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Suspended { .. })) => {} // Expected
            other => panic!("Expected Ready(Err(Suspended)), got {:?}", other),
        }
    }

    #[test]
    fn test_child_workflow_future_returns_ready_when_completed() {
        let future: ChildWorkflowFuture<i32> = ChildWorkflowFuture::from_replay(
            0,
            Uuid::new_v4(),
            "child-1".to_string(),
            Weak::<DummyChildWorkflowFutureContext>::new(),
            Ok(serde_json::json!(123)),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(123)) => {} // Expected
            other => panic!("Expected Ready(Ok(123)), got {:?}", other),
        }
    }

    #[test]
    fn test_child_workflow_future_returns_error_when_failed() {
        let future: ChildWorkflowFuture<i32> = ChildWorkflowFuture::from_replay(
            0,
            Uuid::new_v4(),
            "child-1".to_string(),
            Weak::<DummyChildWorkflowFutureContext>::new(),
            Err(FlovynError::WorkflowFailed("failed".to_string())),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::WorkflowFailed(_))) => {} // Expected
            other => panic!("Expected Ready(Err(WorkflowFailed)), got {:?}", other),
        }
    }

    #[test]
    fn test_child_workflow_future_with_error() {
        let future: ChildWorkflowFuture<i32> =
            ChildWorkflowFuture::with_error(FlovynError::Other("test error".to_string()));
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(_))) => {} // Expected
            other => panic!("Expected Ready(Err(Other)), got {:?}", other),
        }
    }

    /// Mock context that tracks cancel calls for child workflows
    struct MockChildWorkflowFutureContext {
        cancel_count: AtomicUsize,
    }

    impl MockChildWorkflowFutureContext {
        fn new() -> Self {
            Self {
                cancel_count: AtomicUsize::new(0),
            }
        }
    }

    impl ChildWorkflowFutureContext for MockChildWorkflowFutureContext {
        fn find_child_workflow_result(&self, _: &str) -> Option<Result<Value>> {
            None
        }

        fn record_cancel_child_workflow(&self, _: &Uuid) {
            self.cancel_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_child_workflow_future_cancel_records_command() {
        let ctx = Arc::new(MockChildWorkflowFutureContext::new());
        let future: ChildWorkflowFuture<i32> = ChildWorkflowFuture::new(
            0,
            Uuid::new_v4(),
            "child-1".to_string(),
            Arc::downgrade(&ctx) as Weak<dyn ChildWorkflowFutureContext + Send + Sync>,
        );

        assert_eq!(ctx.cancel_count.load(Ordering::SeqCst), 0);
        future.cancel();
        assert_eq!(ctx.cancel_count.load(Ordering::SeqCst), 1);

        // Calling cancel again should not record another command
        future.cancel();
        assert_eq!(ctx.cancel_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_child_workflow_future_returns_cancelled_error_when_cancelled() {
        let future: ChildWorkflowFuture<i32> = ChildWorkflowFuture::new(
            0,
            Uuid::new_v4(),
            "child-1".to_string(),
            Weak::<DummyChildWorkflowFutureContext>::new(),
        );
        future.cancel();

        let mut future = std::pin::pin!(future);
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::WorkflowCancelled(_))) => {} // Expected
            other => panic!("Expected Ready(Err(WorkflowCancelled)), got {:?}", other),
        }
    }

    // ========================================================================
    // PromiseFuture tests
    // ========================================================================

    #[test]
    fn test_promise_future_returns_suspended_when_not_resolved() {
        let future: PromiseFuture<i32> = PromiseFuture::new(
            0,
            "promise-1".to_string(),
            Weak::<DummyPromiseFutureContext>::new(),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Suspended { .. })) => {} // Expected
            other => panic!("Expected Ready(Err(Suspended)), got {:?}", other),
        }
    }

    #[test]
    fn test_promise_future_returns_ready_when_resolved() {
        let future: PromiseFuture<String> = PromiseFuture::from_replay(
            0,
            "promise-1".to_string(),
            Weak::<DummyPromiseFutureContext>::new(),
            Ok(serde_json::json!("resolved")),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(s)) if s == "resolved" => {} // Expected
            other => panic!("Expected Ready(Ok(\"resolved\")), got {:?}", other),
        }
    }

    #[test]
    fn test_promise_future_cannot_be_cancelled() {
        let future: PromiseFuture<i32> = PromiseFuture::new(
            0,
            "promise-1".to_string(),
            Weak::<DummyPromiseFutureContext>::new(),
        );

        future.cancel();
        assert!(!future.is_cancelled()); // Should still return false
    }

    #[test]
    fn test_promise_future_with_error() {
        let future: PromiseFuture<i32> =
            PromiseFuture::with_error(FlovynError::Other("test".to_string()));
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(_))) => {} // Expected
            other => panic!("Expected Ready(Err(Other)), got {:?}", other),
        }
    }

    #[test]
    fn test_promise_future_returns_error_when_rejected() {
        let future: PromiseFuture<i32> = PromiseFuture::from_replay(
            0,
            "promise-1".to_string(),
            Weak::<DummyPromiseFutureContext>::new(),
            Err(FlovynError::Other("promise rejected".to_string())),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(msg))) if msg.contains("rejected") => {} // Expected
            other => panic!("Expected Ready(Err(Other)) with rejected, got {:?}", other),
        }
    }

    // ========================================================================
    // OperationFuture tests
    // ========================================================================

    #[test]
    fn test_operation_future_cannot_be_cancelled() {
        let future: OperationFuture<i32> =
            OperationFuture::new(0, "op-1".to_string(), Ok(serde_json::json!(42)));

        future.cancel();
        assert!(!future.is_cancelled());
    }

    #[test]
    fn test_operation_future_ready_immediately() {
        let future: OperationFuture<i32> =
            OperationFuture::new(0, "op-1".to_string(), Ok(serde_json::json!(42)));
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(42)) => {} // Expected
            other => panic!("Expected Ready(Ok(42)), got {:?}", other),
        }
    }

    #[test]
    fn test_operation_future_returns_error_when_failed() {
        let future: OperationFuture<i32> = OperationFuture::new(
            0,
            "op-1".to_string(),
            Err(FlovynError::Other("op failed".to_string())),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(_))) => {} // Expected
            other => panic!("Expected Ready(Err(Other)), got {:?}", other),
        }
    }

    #[test]
    fn test_operation_future_with_error() {
        let future: OperationFuture<i32> =
            OperationFuture::with_error(FlovynError::Other("test".to_string()));
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(_))) => {} // Expected
            other => panic!("Expected Ready(Err(Other)), got {:?}", other),
        }
    }

    #[test]
    fn test_operation_future_deserializes_complex_type() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Complex {
            name: String,
            value: i32,
        }

        let future: OperationFuture<Complex> = OperationFuture::new(
            0,
            "op-1".to_string(),
            Ok(serde_json::json!({"name": "test", "value": 42})),
        );
        let mut future = std::pin::pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(c)) => {
                assert_eq!(c.name, "test");
                assert_eq!(c.value, 42);
            }
            other => panic!("Expected Ready(Ok(Complex)), got {:?}", other),
        }
    }
}
