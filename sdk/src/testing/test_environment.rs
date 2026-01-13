//! Test workflow environment for in-memory execution without a server.

use crate::error::{FlovynError, Result};
use crate::task::definition::TaskDefinition;
use crate::task::registry::TaskRegistry;
use crate::worker::registry::WorkflowRegistry;
use crate::workflow::definition::WorkflowDefinition;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use super::{MockTaskContext, MockWorkflowContext, TimeController};

/// In-memory workflow execution environment for testing.
///
/// This environment allows you to:
/// - Register workflows and tasks
/// - Execute workflows without a server
/// - Control time progression
/// - Inspect execution state
///
/// # Example
///
/// ```ignore
/// use flovyn_sdk::testing::TestWorkflowEnvironment;
/// use serde_json::json;
///
/// let env = TestWorkflowEnvironment::new();
/// env.register_workflow(MyWorkflow::new());
/// env.register_task(MyTask::new());
///
/// let result = env.execute_workflow("my-workflow", json!({"input": "data"})).await?;
/// ```
pub struct TestWorkflowEnvironment {
    inner: Arc<TestWorkflowEnvironmentInner>,
}

struct TestWorkflowEnvironmentInner {
    workflow_registry: RwLock<WorkflowRegistry>,
    task_registry: RwLock<TaskRegistry>,
    time_controller: TimeController,
    org_id: Uuid,
    task_results: RwLock<HashMap<String, Value>>,
    promise_results: RwLock<HashMap<String, Value>>,
    execution_history: RwLock<Vec<ExecutionRecord>>,
}

impl Clone for TestWorkflowEnvironment {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Record of an execution (workflow or task)
#[derive(Debug, Clone)]
pub struct ExecutionRecord {
    pub execution_type: ExecutionType,
    pub kind: String,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
}

/// Type of execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionType {
    Workflow,
    Task,
}

impl Default for TestWorkflowEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl TestWorkflowEnvironment {
    /// Create a new test environment.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(TestWorkflowEnvironmentInner {
                workflow_registry: RwLock::new(WorkflowRegistry::new()),
                task_registry: RwLock::new(TaskRegistry::new()),
                time_controller: TimeController::new(),
                org_id: Uuid::new_v4(),
                task_results: RwLock::new(HashMap::new()),
                promise_results: RwLock::new(HashMap::new()),
                execution_history: RwLock::new(Vec::new()),
            }),
        }
    }

    /// Create a new test environment with a specific initial time.
    pub fn with_initial_time(initial_time_millis: i64) -> Self {
        Self {
            inner: Arc::new(TestWorkflowEnvironmentInner {
                workflow_registry: RwLock::new(WorkflowRegistry::new()),
                task_registry: RwLock::new(TaskRegistry::new()),
                time_controller: TimeController::with_initial_time(initial_time_millis),
                org_id: Uuid::new_v4(),
                task_results: RwLock::new(HashMap::new()),
                promise_results: RwLock::new(HashMap::new()),
                execution_history: RwLock::new(Vec::new()),
            }),
        }
    }

    /// Get the time controller.
    pub fn time_controller(&self) -> &TimeController {
        &self.inner.time_controller
    }

    /// Get the tenant ID.
    pub fn org_id(&self) -> Uuid {
        self.inner.org_id
    }

    /// Register a workflow definition.
    pub fn register_workflow<W>(&self, workflow: W) -> Result<()>
    where
        W: WorkflowDefinition + 'static,
    {
        self.inner.workflow_registry.write().register(workflow)
    }

    /// Register a task definition.
    pub fn register_task<T>(&self, task: T) -> Result<()>
    where
        T: TaskDefinition + 'static,
    {
        self.inner.task_registry.write().register(task)
    }

    /// Set an expected task result (used when tasks are scheduled during workflow execution).
    pub fn set_task_result(&self, task_type: &str, result: Value) {
        self.inner
            .task_results
            .write()
            .insert(task_type.to_string(), result);
    }

    /// Set an expected promise result.
    pub fn set_promise_result(&self, name: &str, result: Value) {
        self.inner
            .promise_results
            .write()
            .insert(name.to_string(), result);
    }

    /// Execute a workflow by kind.
    pub async fn execute_workflow(&self, kind: &str, input: Value) -> Result<Value> {
        let started_at = self.inner.time_controller.current_time_millis();

        // Get the workflow from the registry
        let workflow = {
            let registry = self.inner.workflow_registry.read();
            registry
                .get(kind)
                .ok_or_else(|| FlovynError::Other(format!("Workflow not registered: {}", kind)))?
        };

        // Create mock context with task results
        let mut builder = MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::new_v4())
            .org_id(self.inner.org_id)
            .input(input.clone())
            .initial_time_millis(started_at);

        // Add task results to context
        for (task_type, result) in self.inner.task_results.read().iter() {
            builder = builder.task_result(task_type, result.clone());
        }

        // Add promise results to context
        for (name, result) in self.inner.promise_results.read().iter() {
            builder = builder.promise_result(name, result.clone());
        }

        let ctx = Arc::new(builder.build());

        // Execute the workflow
        let result = workflow.execute(ctx.clone(), input.clone()).await;

        let completed_at = self.inner.time_controller.current_time_millis();

        // Record execution
        let record = ExecutionRecord {
            execution_type: ExecutionType::Workflow,
            kind: kind.to_string(),
            input,
            output: result.as_ref().ok().cloned(),
            error: result.as_ref().err().map(|e| e.to_string()),
            started_at,
            completed_at: Some(completed_at),
        };
        self.inner.execution_history.write().push(record);

        result
    }

    /// Execute a task by kind.
    pub async fn execute_task(&self, kind: &str, input: Value) -> Result<Value> {
        let started_at = self.inner.time_controller.current_time_millis();

        // Get the task from the registry
        let task = {
            let registry = self.inner.task_registry.read();
            registry
                .get(kind)
                .ok_or_else(|| FlovynError::Other(format!("Task not registered: {}", kind)))?
        };

        // Create mock context
        let ctx = Arc::new(
            MockTaskContext::builder()
                .task_execution_id(Uuid::new_v4())
                .attempt(1)
                .build(),
        );

        // Execute the task
        let result = task.execute(ctx.clone(), input.clone()).await;

        let completed_at = self.inner.time_controller.current_time_millis();

        // Record execution
        let record = ExecutionRecord {
            execution_type: ExecutionType::Task,
            kind: kind.to_string(),
            input,
            output: result.as_ref().ok().cloned(),
            error: result.as_ref().err().map(|e| e.to_string()),
            started_at,
            completed_at: Some(completed_at),
        };
        self.inner.execution_history.write().push(record);

        result
    }

    /// Get all execution records.
    pub fn execution_history(&self) -> Vec<ExecutionRecord> {
        self.inner.execution_history.read().clone()
    }

    /// Get workflow execution records.
    pub fn workflow_executions(&self) -> Vec<ExecutionRecord> {
        self.inner
            .execution_history
            .read()
            .iter()
            .filter(|r| r.execution_type == ExecutionType::Workflow)
            .cloned()
            .collect()
    }

    /// Get task execution records.
    pub fn task_executions(&self) -> Vec<ExecutionRecord> {
        self.inner
            .execution_history
            .read()
            .iter()
            .filter(|r| r.execution_type == ExecutionType::Task)
            .cloned()
            .collect()
    }

    /// Clear execution history.
    pub fn clear_history(&self) {
        self.inner.execution_history.write().clear();
    }

    /// Advance time by the given duration.
    pub fn advance_time(&self, duration: Duration) -> Vec<String> {
        self.inner.time_controller.advance(duration)
    }

    /// Skip all pending timers.
    pub fn skip_all_timers(&self) -> Vec<String> {
        self.inner.time_controller.skip_all_timers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::context::TaskContext;
    use crate::task::definition::DynamicTask;
    use crate::workflow::context::WorkflowContext;
    use crate::workflow::definition::DynamicWorkflow;
    use async_trait::async_trait;
    use serde_json::json;

    // Test workflow
    struct TestWorkflow;

    #[async_trait]
    impl DynamicWorkflow for TestWorkflow {
        fn kind(&self) -> &str {
            "test-workflow"
        }

        async fn execute(
            &self,
            _ctx: &dyn WorkflowContext,
            input: serde_json::Map<String, Value>,
        ) -> Result<serde_json::Map<String, Value>> {
            let mut output = serde_json::Map::new();
            output.insert(
                "processed".to_string(),
                input.get("value").cloned().unwrap_or(json!(0)),
            );
            Ok(output)
        }
    }

    // Test task
    struct TestTask;

    #[async_trait]
    impl DynamicTask for TestTask {
        fn kind(&self) -> &str {
            "test-task"
        }

        async fn execute(
            &self,
            input: serde_json::Map<String, Value>,
            _ctx: &dyn TaskContext,
        ) -> Result<serde_json::Map<String, Value>> {
            let mut output = serde_json::Map::new();
            output.insert(
                "task_result".to_string(),
                input.get("data").cloned().unwrap_or(json!("none")),
            );
            Ok(output)
        }
    }

    #[test]
    fn test_environment_new() {
        let env = TestWorkflowEnvironment::new();
        assert!(!env.org_id().is_nil());
    }

    #[test]
    fn test_environment_with_initial_time() {
        let env = TestWorkflowEnvironment::with_initial_time(1000);
        assert_eq!(env.time_controller().current_time_millis(), 1000);
    }

    #[test]
    fn test_register_workflow() {
        let env = TestWorkflowEnvironment::new();
        assert!(env.register_workflow(TestWorkflow).is_ok());
    }

    #[test]
    fn test_register_task() {
        let env = TestWorkflowEnvironment::new();
        assert!(env.register_task(TestTask).is_ok());
    }

    #[tokio::test]
    async fn test_execute_workflow() {
        let env = TestWorkflowEnvironment::new();
        env.register_workflow(TestWorkflow).unwrap();

        let result = env
            .execute_workflow("test-workflow", json!({"value": 42}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"processed": 42}));
    }

    #[tokio::test]
    async fn test_execute_workflow_not_registered() {
        let env = TestWorkflowEnvironment::new();
        let result = env.execute_workflow("nonexistent", json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_task() {
        let env = TestWorkflowEnvironment::new();
        env.register_task(TestTask).unwrap();

        let result = env
            .execute_task("test-task", json!({"data": "hello"}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"task_result": "hello"}));
    }

    #[tokio::test]
    async fn test_execute_task_not_registered() {
        let env = TestWorkflowEnvironment::new();
        let result = env.execute_task("nonexistent", json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execution_history() {
        let env = TestWorkflowEnvironment::new();
        env.register_workflow(TestWorkflow).unwrap();
        env.register_task(TestTask).unwrap();

        env.execute_workflow("test-workflow", json!({}))
            .await
            .unwrap();
        env.execute_task("test-task", json!({})).await.unwrap();

        let history = env.execution_history();
        assert_eq!(history.len(), 2);

        let workflows = env.workflow_executions();
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].kind, "test-workflow");

        let tasks = env.task_executions();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].kind, "test-task");
    }

    #[tokio::test]
    async fn test_clear_history() {
        let env = TestWorkflowEnvironment::new();
        env.register_workflow(TestWorkflow).unwrap();

        env.execute_workflow("test-workflow", json!({}))
            .await
            .unwrap();
        assert_eq!(env.execution_history().len(), 1);

        env.clear_history();
        assert!(env.execution_history().is_empty());
    }

    #[test]
    fn test_advance_time() {
        let env = TestWorkflowEnvironment::with_initial_time(1000);

        env.advance_time(Duration::from_secs(5));
        assert_eq!(env.time_controller().current_time_millis(), 6000);
    }

    #[test]
    fn test_set_task_result() {
        let env = TestWorkflowEnvironment::new();
        env.set_task_result("external-task", json!({"result": "ok"}));
    }

    #[test]
    fn test_set_promise_result() {
        let env = TestWorkflowEnvironment::new();
        env.set_promise_result("approval", json!({"approved": true}));
    }

    #[test]
    fn test_clone() {
        let env1 = TestWorkflowEnvironment::with_initial_time(1000);
        let env2 = env1.clone();

        env1.advance_time(Duration::from_secs(5));
        assert_eq!(env2.time_controller().current_time_millis(), 6000);
    }
}
