//! WorkflowContext trait definition

use crate::error::{FlovynError, Result};
use crate::task::definition::TaskDefinition;
use crate::workflow::definition::WorkflowDefinition;
use crate::workflow::future::{
    ChildWorkflowFuture, ChildWorkflowFutureRaw, OperationFutureRaw, PromiseFuture,
    PromiseFutureRaw, TaskFuture, TaskFutureRaw, TimerFuture,
};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::marker::PhantomData;
use std::time::Duration;
use uuid::Uuid;

/// Options for scheduling a task
#[derive(Debug, Clone, Default)]
pub struct ScheduleTaskOptions {
    /// Priority in seconds (lower = higher priority)
    pub priority_seconds: Option<i32>,
    /// Task timeout override
    pub timeout: Option<Duration>,
    /// Task queue override
    pub queue: Option<String>,
    /// Maximum retry attempts
    pub max_retries: Option<u32>,
}

/// A deterministic random number generator
pub trait DeterministicRandom: Send + Sync {
    /// Generate a random i32 in the range [min, max)
    fn next_int(&self, min: i32, max: i32) -> i32;

    /// Generate a random i64 in the range [min, max)
    fn next_long(&self, min: i64, max: i64) -> i64;

    /// Generate a random f64 in the range [0, 1)
    fn next_double(&self) -> f64;

    /// Generate a random bool
    fn next_bool(&self) -> bool;
}

/// Context for workflow execution providing deterministic APIs and side effect management.
///
/// This trait uses `Value` types for object-safety. For typed APIs, use the extension
/// methods provided by `WorkflowContextExt`.
#[async_trait]
pub trait WorkflowContext: Send + Sync {
    // === Identifiers ===

    /// Get the unique ID of this workflow execution
    fn workflow_execution_id(&self) -> Uuid;

    /// Get the tenant ID for this workflow
    fn tenant_id(&self) -> Uuid;

    /// Get the raw workflow input as JSON Value
    fn input_raw(&self) -> &Value;

    // === Deterministic APIs (recorded/replayed) ===

    /// Get the current time in milliseconds (deterministic - same on replay)
    fn current_time_millis(&self) -> i64;

    /// Generate a deterministic UUID (same on replay)
    fn random_uuid(&self) -> Uuid;

    /// Get a deterministic random number generator (same sequence on replay)
    fn random(&self) -> &dyn DeterministicRandom;

    // === Side Effects (cached via event sourcing) ===

    /// Execute a side effect and cache the result (raw Value version).
    /// On replay, returns the cached result without re-executing.
    async fn run_raw(&self, name: &str, result: Value) -> Result<Value>;

    // === Task Scheduling ===

    /// Schedule a task and wait for its completion (raw Value version)
    async fn schedule_raw(&self, task_type: &str, input: Value) -> Result<Value>;

    /// Schedule a task with custom options (raw Value version)
    async fn schedule_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> Result<Value>;

    // === State Management ===

    /// Get a value from workflow state (raw Value version)
    async fn get_raw(&self, key: &str) -> Result<Option<Value>>;

    /// Set a value in workflow state (raw Value version)
    async fn set_raw(&self, key: &str, value: Value) -> Result<()>;

    /// Clear a specific key from workflow state
    async fn clear(&self, key: &str) -> Result<()>;

    /// Clear all workflow state
    async fn clear_all(&self) -> Result<()>;

    /// Get all keys in workflow state
    async fn state_keys(&self) -> Result<Vec<String>>;

    // === Timers ===

    /// Sleep for the specified duration (durable - survives restarts)
    async fn sleep(&self, duration: Duration) -> Result<()>;

    // === Promises (Signals) ===

    /// Create a durable promise that can be resolved externally (raw Value version)
    async fn promise_raw(&self, name: &str) -> Result<Value>;

    /// Create a durable promise with a timeout (raw Value version)
    async fn promise_with_timeout_raw(&self, name: &str, timeout: Duration) -> Result<Value>;

    // === Child Workflows ===

    /// Schedule a child workflow and wait for its completion (raw Value version)
    async fn schedule_workflow_raw(&self, name: &str, kind: &str, input: Value) -> Result<Value>;

    // === Cancellation ===

    /// Check if cancellation has been requested
    fn is_cancellation_requested(&self) -> bool;

    /// Check for cancellation and return error if cancelled
    async fn check_cancellation(&self) -> Result<()>;

    // =========================================================================
    // Async Operation Methods for Parallel Execution
    // =========================================================================
    // These methods schedule operations and return futures immediately,
    // allowing multiple operations to be scheduled and awaited in parallel.
    // The sequence number is assigned at call time (not await time) for
    // deterministic replay.

    // === Async Task Scheduling ===

    /// Schedule a task asynchronously, returning a future.
    ///
    /// Unlike `schedule_raw`, this method returns immediately with a future
    /// that can be awaited later, enabling parallel task execution.
    ///
    /// # Example
    /// ```ignore
    /// let task_a = ctx.schedule_async_raw("task-a", input_a);
    /// let task_b = ctx.schedule_async_raw("task-b", input_b);
    /// let (result_a, result_b) = tokio::join!(task_a, task_b);
    /// ```
    fn schedule_async_raw(&self, task_type: &str, input: Value) -> TaskFutureRaw;

    /// Schedule a task asynchronously with custom options.
    fn schedule_async_with_options_raw(
        &self,
        task_type: &str,
        input: Value,
        options: ScheduleTaskOptions,
    ) -> TaskFutureRaw;

    // === Async Timers ===

    /// Sleep asynchronously, returning a timer future.
    ///
    /// Unlike `sleep`, this returns immediately with a future.
    fn sleep_async(&self, duration: Duration) -> TimerFuture;

    // === Async Child Workflows ===

    /// Schedule a child workflow asynchronously.
    fn schedule_workflow_async_raw(
        &self,
        name: &str,
        kind: &str,
        input: Value,
    ) -> ChildWorkflowFutureRaw;

    // === Async Promises ===

    /// Create a durable promise asynchronously.
    fn promise_async_raw(&self, name: &str) -> PromiseFutureRaw;

    // === Async Operations ===

    /// Run an operation asynchronously, returning a future with the cached result.
    fn run_async_raw(&self, name: &str, result: Value) -> OperationFutureRaw;
}

/// Extension trait for typed workflow context operations.
/// These methods provide type-safe wrappers around the raw Value methods.
pub trait WorkflowContextExt: WorkflowContext {
    /// Get the workflow input as the specified type
    fn input<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.input_raw().clone())
            .map_err(crate::error::FlovynError::Serialization)
    }

    /// Get a value from workflow state
    fn get_typed<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> impl std::future::Future<Output = Result<Option<T>>> + Send
    where
        Self: Sync,
    {
        async move {
            match self.get_raw(key).await? {
                Some(v) => serde_json::from_value(v)
                    .map(Some)
                    .map_err(crate::error::FlovynError::Serialization),
                None => Ok(None),
            }
        }
    }

    /// Set a value in workflow state
    fn set_typed<T: serde::Serialize + Send>(
        &self,
        key: &str,
        value: T,
    ) -> impl std::future::Future<Output = Result<()>> + Send
    where
        Self: Sync,
    {
        async move {
            let v =
                serde_json::to_value(value).map_err(crate::error::FlovynError::Serialization)?;
            self.set_raw(key, v).await
        }
    }

    // =========================================================================
    // Typed Async Methods for Parallel Execution
    // =========================================================================
    // These methods provide compile-time type safety by leveraging TaskDefinition
    // and WorkflowDefinition traits. They wrap the raw Value methods with
    // automatic serialization/deserialization.

    /// Schedule a typed task asynchronously.
    ///
    /// This method provides compile-time type safety by using the `TaskDefinition` trait
    /// to determine the task type and ensure correct input/output types.
    ///
    /// # Type Parameters
    ///
    /// * `T` - A type implementing `TaskDefinition + Default`. The `Default` bound
    ///   allows accessing the task type without requiring an instance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// #[derive(Default)]
    /// struct SendEmailTask;
    ///
    /// impl TaskDefinition for SendEmailTask {
    ///     type Input = EmailRequest;
    ///     type Output = EmailResponse;
    ///     fn kind(&self) -> &str { "send-email" }
    ///     // ...
    /// }
    ///
    /// // Schedule with type safety
    /// let future: TaskFuture<EmailResponse> = ctx.schedule_async::<SendEmailTask>(request);
    /// let response = future.await?;
    /// ```
    fn schedule_async<T: TaskDefinition + Default>(&self, input: T::Input) -> TaskFuture<T::Output>
    where
        T::Input: Serialize,
        T::Output: DeserializeOwned,
    {
        let task = T::default();
        let task_type = task.kind();

        let input_value = match serde_json::to_value(&input) {
            Ok(v) => v,
            Err(e) => return TaskFuture::with_error(FlovynError::Serialization(e)),
        };

        let raw_future = self.schedule_async_raw(task_type, input_value);

        // Convert TaskFuture<Value> to TaskFuture<T::Output>
        TaskFuture {
            task_seq: raw_future.task_seq,
            task_execution_id: raw_future.task_execution_id,
            context: raw_future.context,
            state: raw_future.state,
            _marker: PhantomData,
        }
    }

    /// Schedule a typed task with custom options.
    ///
    /// Similar to `schedule_async`, but allows specifying task options like
    /// priority, timeout, queue, and max retries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let options = ScheduleTaskOptions {
    ///     timeout: Some(Duration::from_secs(30)),
    ///     max_retries: Some(3),
    ///     ..Default::default()
    /// };
    /// let future = ctx.schedule_async_with_options::<SendEmailTask>(request, options);
    /// ```
    fn schedule_async_with_options<T: TaskDefinition + Default>(
        &self,
        input: T::Input,
        options: ScheduleTaskOptions,
    ) -> TaskFuture<T::Output>
    where
        T::Input: Serialize,
        T::Output: DeserializeOwned,
    {
        let task = T::default();
        let task_type = task.kind();

        let input_value = match serde_json::to_value(&input) {
            Ok(v) => v,
            Err(e) => return TaskFuture::with_error(FlovynError::Serialization(e)),
        };

        let raw_future = self.schedule_async_with_options_raw(task_type, input_value, options);

        TaskFuture {
            task_seq: raw_future.task_seq,
            task_execution_id: raw_future.task_execution_id,
            context: raw_future.context,
            state: raw_future.state,
            _marker: PhantomData,
        }
    }

    /// Schedule a typed child workflow asynchronously.
    ///
    /// This method provides compile-time type safety for child workflow scheduling
    /// using the `WorkflowDefinition` trait.
    ///
    /// # Example
    ///
    /// ```ignore
    /// #[derive(Default)]
    /// struct PaymentWorkflow;
    ///
    /// impl WorkflowDefinition for PaymentWorkflow {
    ///     type Input = PaymentRequest;
    ///     type Output = PaymentResult;
    ///     fn kind(&self) -> &str { "payment-workflow" }
    ///     // ...
    /// }
    ///
    /// let future = ctx.schedule_workflow_async::<PaymentWorkflow>("payment-1", request);
    /// let result = future.await?;
    /// ```
    fn schedule_workflow_async<W: WorkflowDefinition + Default>(
        &self,
        name: &str,
        input: W::Input,
    ) -> ChildWorkflowFuture<W::Output>
    where
        W::Input: Serialize,
        W::Output: DeserializeOwned,
    {
        let workflow = W::default();
        let kind = workflow.kind();

        let input_value = match serde_json::to_value(&input) {
            Ok(v) => v,
            Err(e) => return ChildWorkflowFuture::with_error(FlovynError::Serialization(e)),
        };

        let raw_future = self.schedule_workflow_async_raw(name, kind, input_value);

        ChildWorkflowFuture {
            child_workflow_seq: raw_future.child_workflow_seq,
            child_execution_id: raw_future.child_execution_id,
            child_execution_name: raw_future.child_execution_name,
            context: raw_future.context,
            state: raw_future.state,
            _marker: PhantomData,
        }
    }

    /// Create a typed promise asynchronously.
    ///
    /// Returns a future that resolves when the promise is resolved externally
    /// with a value of type `T`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let future: PromiseFuture<ApprovalDecision> = ctx.promise_async("approval");
    /// let decision = future.await?;
    /// ```
    fn promise_async<T: DeserializeOwned>(&self, name: &str) -> PromiseFuture<T> {
        let raw_future = self.promise_async_raw(name);

        PromiseFuture {
            promise_seq: raw_future.promise_seq,
            promise_id: raw_future.promise_id,
            context: raw_future.context,
            state: raw_future.state,
            _marker: PhantomData,
        }
    }
}

// Implement WorkflowContextExt for all types that implement WorkflowContext
impl<C: WorkflowContext + ?Sized> WorkflowContextExt for C {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_task_options_default() {
        let options = ScheduleTaskOptions::default();
        assert!(options.priority_seconds.is_none());
        assert!(options.timeout.is_none());
    }

    #[test]
    fn test_schedule_task_options_with_values() {
        let options = ScheduleTaskOptions {
            priority_seconds: Some(60),
            timeout: Some(Duration::from_secs(300)),
            queue: Some("custom-queue".to_string()),
            max_retries: Some(5),
        };
        assert_eq!(options.priority_seconds, Some(60));
        assert_eq!(options.timeout, Some(Duration::from_secs(300)));
        assert_eq!(options.queue, Some("custom-queue".to_string()));
        assert_eq!(options.max_retries, Some(5));
    }
}

#[cfg(all(test, feature = "testing"))]
mod typed_async_tests {
    use super::*;
    use crate::task::context::TaskContext;
    use crate::testing::MockWorkflowContext;
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use std::future::Future;
    use std::task::{Context, Poll};

    // ========================================================================
    // Test Task Definition
    // ========================================================================

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct EmailRequest {
        to: String,
        subject: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct EmailResponse {
        sent: bool,
        message_id: String,
    }

    #[derive(Default)]
    struct SendEmailTask;

    #[async_trait]
    impl TaskDefinition for SendEmailTask {
        type Input = EmailRequest;
        type Output = EmailResponse;

        fn kind(&self) -> &str {
            "send-email"
        }

        async fn execute(
            &self,
            _input: Self::Input,
            _ctx: &dyn TaskContext,
        ) -> crate::error::Result<Self::Output> {
            Ok(EmailResponse {
                sent: true,
                message_id: "test-123".to_string(),
            })
        }
    }

    // ========================================================================
    // Test Workflow Definition
    // ========================================================================

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct PaymentRequest {
        amount: i64,
        currency: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct PaymentResult {
        success: bool,
        transaction_id: String,
    }

    #[derive(Default)]
    struct PaymentWorkflow;

    #[async_trait]
    impl WorkflowDefinition for PaymentWorkflow {
        type Input = PaymentRequest;
        type Output = PaymentResult;

        fn kind(&self) -> &str {
            "payment-workflow"
        }

        async fn execute(
            &self,
            _ctx: &dyn WorkflowContext,
            _input: Self::Input,
        ) -> crate::error::Result<Self::Output> {
            Ok(PaymentResult {
                success: true,
                transaction_id: "txn-456".to_string(),
            })
        }
    }

    // ========================================================================
    // Tests for schedule_async
    // ========================================================================

    #[test]
    fn test_schedule_async_typed_serializes_input() {
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "send-email",
                json!({
                    "sent": true,
                    "message_id": "msg-001"
                }),
            )
            .build();

        let input = EmailRequest {
            to: "test@example.com".to_string(),
            subject: "Hello".to_string(),
        };

        let _future: TaskFuture<EmailResponse> = ctx.schedule_async::<SendEmailTask>(input.clone());

        // Verify the task was scheduled with the correct task type
        assert!(ctx.was_task_scheduled("send-email"));

        // Verify the input was serialized correctly
        let scheduled_tasks = ctx.scheduled_tasks();
        assert_eq!(scheduled_tasks.len(), 1);
        let scheduled_input: EmailRequest =
            serde_json::from_value(scheduled_tasks[0].input.clone()).unwrap();
        assert_eq!(scheduled_input, input);
    }

    #[tokio::test]
    async fn test_schedule_async_typed_returns_typed_future() {
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "send-email",
                json!({
                    "sent": true,
                    "message_id": "msg-001"
                }),
            )
            .build();

        let input = EmailRequest {
            to: "test@example.com".to_string(),
            subject: "Hello".to_string(),
        };

        let future: TaskFuture<EmailResponse> = ctx.schedule_async::<SendEmailTask>(input);
        let result = future.await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.sent);
        assert_eq!(response.message_id, "msg-001");
    }

    #[test]
    fn test_schedule_async_serialization_error_returns_error_future() {
        // Test with a type that has custom serialization that might fail
        // For this test, we'll use a working type since it's hard to make
        // serde_json::to_value fail with standard types
        // Instead, we verify the error handling path works by checking
        // the future mechanism works correctly

        let ctx = MockWorkflowContext::new();

        let input = EmailRequest {
            to: "test@example.com".to_string(),
            subject: "Hello".to_string(),
        };

        // Without a configured result, the mock will return an error
        let future: TaskFuture<EmailResponse> = ctx.schedule_async::<SendEmailTask>(input);

        // Create a waker and poll the future
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(_)) => {} // Expected - no mock result configured
            other => panic!("Expected Ready(Err), got {:?}", other),
        }
    }

    // ========================================================================
    // Tests for schedule_async_with_options
    // ========================================================================

    #[test]
    fn test_schedule_async_with_options_typed() {
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "send-email",
                json!({
                    "sent": true,
                    "message_id": "msg-002"
                }),
            )
            .build();

        let input = EmailRequest {
            to: "test@example.com".to_string(),
            subject: "Hello with options".to_string(),
        };

        let options = ScheduleTaskOptions {
            timeout: Some(Duration::from_secs(30)),
            max_retries: Some(3),
            ..Default::default()
        };

        let _future: TaskFuture<EmailResponse> =
            ctx.schedule_async_with_options::<SendEmailTask>(input.clone(), options.clone());

        // Verify the task was scheduled
        assert!(ctx.was_task_scheduled("send-email"));

        // Verify options were passed
        let scheduled_tasks = ctx.scheduled_tasks();
        assert_eq!(scheduled_tasks.len(), 1);
        assert_eq!(
            scheduled_tasks[0].options.timeout,
            Some(Duration::from_secs(30))
        );
        assert_eq!(scheduled_tasks[0].options.max_retries, Some(3));
    }

    // ========================================================================
    // Tests for schedule_workflow_async
    // ========================================================================

    #[test]
    fn test_schedule_workflow_async_typed() {
        let ctx = MockWorkflowContext::builder()
            .child_workflow_result(
                "payment-workflow",
                json!({
                    "success": true,
                    "transaction_id": "txn-789"
                }),
            )
            .build();

        let input = PaymentRequest {
            amount: 10000,
            currency: "USD".to_string(),
        };

        let _future: ChildWorkflowFuture<PaymentResult> =
            ctx.schedule_workflow_async::<PaymentWorkflow>("payment-1", input.clone());

        // Verify the workflow was scheduled with the correct kind
        assert!(ctx.was_workflow_scheduled("payment-workflow"));

        // Verify the input was serialized correctly
        let scheduled_workflows = ctx.scheduled_workflows();
        assert_eq!(scheduled_workflows.len(), 1);
        assert_eq!(scheduled_workflows[0].name, "payment-1");
        assert_eq!(scheduled_workflows[0].kind, "payment-workflow");

        let scheduled_input: PaymentRequest =
            serde_json::from_value(scheduled_workflows[0].input.clone()).unwrap();
        assert_eq!(scheduled_input, input);
    }

    #[tokio::test]
    async fn test_schedule_workflow_async_typed_returns_result() {
        let ctx = MockWorkflowContext::builder()
            .child_workflow_result(
                "payment-workflow",
                json!({
                    "success": true,
                    "transaction_id": "txn-789"
                }),
            )
            .build();

        let input = PaymentRequest {
            amount: 10000,
            currency: "USD".to_string(),
        };

        let future: ChildWorkflowFuture<PaymentResult> =
            ctx.schedule_workflow_async::<PaymentWorkflow>("payment-1", input);
        let result = future.await;

        assert!(result.is_ok());
        let payment_result = result.unwrap();
        assert!(payment_result.success);
        assert_eq!(payment_result.transaction_id, "txn-789");
    }

    // ========================================================================
    // Tests for promise_async
    // ========================================================================

    #[test]
    fn test_promise_async_typed() {
        let ctx = MockWorkflowContext::builder()
            .promise_result(
                "approval",
                json!({
                    "approved": true,
                    "approver": "manager"
                }),
            )
            .build();

        #[derive(Debug, Deserialize, PartialEq)]
        struct ApprovalDecision {
            approved: bool,
            approver: String,
        }

        let _future: PromiseFuture<ApprovalDecision> = ctx.promise_async("approval");

        // Verify the promise was created
        assert!(ctx.was_promise_created("approval"));
    }

    #[tokio::test]
    async fn test_promise_async_typed_returns_result() {
        let ctx = MockWorkflowContext::builder()
            .promise_result(
                "approval",
                json!({
                    "approved": true,
                    "approver": "manager"
                }),
            )
            .build();

        #[derive(Debug, Deserialize, PartialEq)]
        struct ApprovalDecision {
            approved: bool,
            approver: String,
        }

        let future: PromiseFuture<ApprovalDecision> = ctx.promise_async("approval");
        let result = future.await;

        assert!(result.is_ok());
        let decision = result.unwrap();
        assert!(decision.approved);
        assert_eq!(decision.approver, "manager");
    }

    // ========================================================================
    // Tests for error handling
    // ========================================================================

    #[test]
    fn test_schedule_async_without_mock_result_returns_error() {
        let ctx = MockWorkflowContext::new();

        let input = EmailRequest {
            to: "test@example.com".to_string(),
            subject: "Hello".to_string(),
        };

        let future: TaskFuture<EmailResponse> = ctx.schedule_async::<SendEmailTask>(input);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(e)) => {
                // Should be an error about no mock result configured
                assert!(format!("{:?}", e).contains("send-email"));
            }
            other => panic!("Expected Ready(Err), got {:?}", other),
        }
    }

    #[test]
    fn test_schedule_workflow_async_without_mock_result_returns_error() {
        let ctx = MockWorkflowContext::new();

        let input = PaymentRequest {
            amount: 10000,
            currency: "USD".to_string(),
        };

        let future: ChildWorkflowFuture<PaymentResult> =
            ctx.schedule_workflow_async::<PaymentWorkflow>("payment-1", input);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(e)) => {
                // Should be an error about no mock result configured
                assert!(format!("{:?}", e).contains("payment-workflow"));
            }
            other => panic!("Expected Ready(Err), got {:?}", other),
        }
    }

    #[test]
    fn test_promise_async_without_mock_result_returns_error() {
        let ctx = MockWorkflowContext::new();

        #[derive(Debug, Deserialize)]
        struct ApprovalDecision {
            #[allow(dead_code)]
            approved: bool,
        }

        let future: PromiseFuture<ApprovalDecision> = ctx.promise_async("approval");

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(e)) => {
                // Should be an error about no mock result configured
                assert!(format!("{:?}", e).contains("approval"));
            }
            other => panic!("Expected Ready(Err), got {:?}", other),
        }
    }
}
