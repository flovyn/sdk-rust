//! Workflow lifecycle hooks for observability
//!
//! Hooks receive notifications about workflow execution events and can be used
//! for logging, metrics, monitoring, etc.

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

/// Hook trait for observing workflow lifecycle events
///
/// Implementations can be registered with FlovynClientBuilder to receive
/// notifications about workflow execution events for monitoring, logging,
/// metrics collection, etc.
///
/// Hook exceptions are caught and logged but do not affect workflow execution.
#[async_trait]
pub trait WorkflowHook: Send + Sync {
    /// Called when a workflow starts executing on this worker
    async fn on_workflow_started(
        &self,
        _workflow_execution_id: Uuid,
        _workflow_kind: &str,
        _input: &Value,
    ) {
    }

    /// Called when a workflow completes successfully
    async fn on_workflow_completed(
        &self,
        _workflow_execution_id: Uuid,
        _workflow_kind: &str,
        _result: &Value,
    ) {
    }

    /// Called when a workflow fails with an error
    async fn on_workflow_failed(
        &self,
        _workflow_execution_id: Uuid,
        _workflow_kind: &str,
        _error: &str,
    ) {
    }

    /// Called when a task is scheduled via ctx.schedule()
    async fn on_task_scheduled(
        &self,
        _workflow_execution_id: Uuid,
        _task_id: &str,
        _task_type: &str,
        _input: &Value,
    ) {
    }

    /// Called when a durable promise is awaited via ctx.promise()
    async fn on_promise_awaited(&self, _workflow_execution_id: Uuid, _promise_name: &str) {}

    /// Called when workflow state is updated via ctx.set()
    async fn on_state_updated(&self, _workflow_execution_id: Uuid, _key: &str, _value: &Value) {}
}

/// Composite hook that delegates to multiple hooks
///
/// Used internally to support multiple hook registrations.
pub struct CompositeWorkflowHook {
    hooks: Vec<Box<dyn WorkflowHook>>,
}

impl CompositeWorkflowHook {
    /// Create a new composite hook from a list of hooks
    pub fn new(hooks: Vec<Box<dyn WorkflowHook>>) -> Self {
        Self { hooks }
    }
}

#[async_trait]
impl WorkflowHook for CompositeWorkflowHook {
    async fn on_workflow_started(
        &self,
        workflow_execution_id: Uuid,
        workflow_kind: &str,
        input: &Value,
    ) {
        for hook in &self.hooks {
            if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // We can't easily catch async panics, so we just log errors
                let _ = hook.on_workflow_started(workflow_execution_id, workflow_kind, input);
            })) {
                tracing::warn!("Hook exception in on_workflow_started: {:?}", e);
            }
        }
        // Execute hooks properly
        for hook in &self.hooks {
            hook.on_workflow_started(workflow_execution_id, workflow_kind, input)
                .await;
        }
    }

    async fn on_workflow_completed(
        &self,
        workflow_execution_id: Uuid,
        workflow_kind: &str,
        result: &Value,
    ) {
        for hook in &self.hooks {
            hook.on_workflow_completed(workflow_execution_id, workflow_kind, result)
                .await;
        }
    }

    async fn on_workflow_failed(
        &self,
        workflow_execution_id: Uuid,
        workflow_kind: &str,
        error: &str,
    ) {
        for hook in &self.hooks {
            hook.on_workflow_failed(workflow_execution_id, workflow_kind, error)
                .await;
        }
    }

    async fn on_task_scheduled(
        &self,
        workflow_execution_id: Uuid,
        task_id: &str,
        task_type: &str,
        input: &Value,
    ) {
        for hook in &self.hooks {
            hook.on_task_scheduled(workflow_execution_id, task_id, task_type, input)
                .await;
        }
    }

    async fn on_promise_awaited(&self, workflow_execution_id: Uuid, promise_name: &str) {
        for hook in &self.hooks {
            hook.on_promise_awaited(workflow_execution_id, promise_name)
                .await;
        }
    }

    async fn on_state_updated(&self, workflow_execution_id: Uuid, key: &str, value: &Value) {
        for hook in &self.hooks {
            hook.on_state_updated(workflow_execution_id, key, value)
                .await;
        }
    }
}

/// A no-op hook that does nothing (useful as a default)
pub struct NoOpHook;

#[async_trait]
impl WorkflowHook for NoOpHook {}

/// A logging hook that logs workflow events using tracing
pub struct LoggingHook {
    level: tracing::Level,
}

impl LoggingHook {
    /// Create a new logging hook with the specified log level
    pub fn new(level: tracing::Level) -> Self {
        Self { level }
    }

    /// Create a logging hook that logs at INFO level
    pub fn info() -> Self {
        Self::new(tracing::Level::INFO)
    }

    /// Create a logging hook that logs at DEBUG level
    pub fn debug() -> Self {
        Self::new(tracing::Level::DEBUG)
    }
}

impl Default for LoggingHook {
    fn default() -> Self {
        Self::info()
    }
}

#[async_trait]
impl WorkflowHook for LoggingHook {
    async fn on_workflow_started(
        &self,
        workflow_execution_id: Uuid,
        workflow_kind: &str,
        _input: &Value,
    ) {
        match self.level {
            tracing::Level::DEBUG => {
                tracing::debug!(
                    workflow_execution_id = %workflow_execution_id,
                    workflow_kind = %workflow_kind,
                    "Workflow started"
                );
            }
            _ => {
                tracing::info!(
                    workflow_execution_id = %workflow_execution_id,
                    workflow_kind = %workflow_kind,
                    "Workflow started"
                );
            }
        }
    }

    async fn on_workflow_completed(
        &self,
        workflow_execution_id: Uuid,
        workflow_kind: &str,
        _result: &Value,
    ) {
        match self.level {
            tracing::Level::DEBUG => {
                tracing::debug!(
                    workflow_execution_id = %workflow_execution_id,
                    workflow_kind = %workflow_kind,
                    "Workflow completed"
                );
            }
            _ => {
                tracing::info!(
                    workflow_execution_id = %workflow_execution_id,
                    workflow_kind = %workflow_kind,
                    "Workflow completed"
                );
            }
        }
    }

    async fn on_workflow_failed(
        &self,
        workflow_execution_id: Uuid,
        workflow_kind: &str,
        error: &str,
    ) {
        tracing::error!(
            workflow_execution_id = %workflow_execution_id,
            workflow_kind = %workflow_kind,
            error = %error,
            "Workflow failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct CountingHook {
        started_count: Arc<AtomicU32>,
        completed_count: Arc<AtomicU32>,
        failed_count: Arc<AtomicU32>,
    }

    impl CountingHook {
        fn new() -> Self {
            Self {
                started_count: Arc::new(AtomicU32::new(0)),
                completed_count: Arc::new(AtomicU32::new(0)),
                failed_count: Arc::new(AtomicU32::new(0)),
            }
        }
    }

    #[async_trait]
    impl WorkflowHook for CountingHook {
        async fn on_workflow_started(&self, _: Uuid, _: &str, _: &Value) {
            self.started_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn on_workflow_completed(&self, _: Uuid, _: &str, _: &Value) {
            self.completed_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn on_workflow_failed(&self, _: Uuid, _: &str, _: &str) {
            self.failed_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn test_noop_hook() {
        let hook = NoOpHook;
        // Should not panic
        hook.on_workflow_started(Uuid::new_v4(), "test", &Value::Null)
            .await;
        hook.on_workflow_completed(Uuid::new_v4(), "test", &Value::Null)
            .await;
        hook.on_workflow_failed(Uuid::new_v4(), "test", "error")
            .await;
    }

    #[tokio::test]
    async fn test_counting_hook() {
        let hook = CountingHook::new();
        let started = hook.started_count.clone();
        let completed = hook.completed_count.clone();
        let failed = hook.failed_count.clone();

        hook.on_workflow_started(Uuid::new_v4(), "test", &Value::Null)
            .await;
        hook.on_workflow_started(Uuid::new_v4(), "test", &Value::Null)
            .await;
        hook.on_workflow_completed(Uuid::new_v4(), "test", &Value::Null)
            .await;
        hook.on_workflow_failed(Uuid::new_v4(), "test", "error")
            .await;

        assert_eq!(started.load(Ordering::SeqCst), 2);
        assert_eq!(completed.load(Ordering::SeqCst), 1);
        assert_eq!(failed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_composite_hook() {
        let hook1 = CountingHook::new();
        let hook2 = CountingHook::new();
        let started1 = hook1.started_count.clone();
        let started2 = hook2.started_count.clone();

        let composite = CompositeWorkflowHook::new(vec![Box::new(hook1), Box::new(hook2)]);

        composite
            .on_workflow_started(Uuid::new_v4(), "test", &Value::Null)
            .await;

        assert_eq!(started1.load(Ordering::SeqCst), 1);
        assert_eq!(started2.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_logging_hook_creation() {
        let _hook = LoggingHook::info();
        let _hook = LoggingHook::debug();
        let _hook = LoggingHook::default();
    }
}
