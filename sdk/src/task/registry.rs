//! TaskRegistry - Registry for task definitions

use crate::common::version::SemanticVersion;
use crate::error::{FlovynError, Result};
use crate::task::context::TaskContext;
use crate::task::definition::TaskDefinition;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Task metadata extracted from a task definition
#[derive(Debug, Clone)]
pub struct TaskMetadata {
    /// Unique task kind identifier
    pub kind: String,
    /// Human-readable name
    pub name: String,
    /// Description of the task
    pub description: Option<String>,
    /// Version of the task
    pub version: Option<SemanticVersion>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Timeout in seconds
    pub timeout_seconds: Option<u32>,
    /// Whether the task can be cancelled
    pub cancellable: bool,
    /// Heartbeat timeout in seconds
    pub heartbeat_timeout_seconds: Option<u32>,
}

/// Type alias for boxed task execution functions
pub type BoxedTaskFn = Box<
    dyn Fn(
            Arc<dyn TaskContext + Send + Sync>,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// A registered task with its metadata and execution function
pub struct RegisteredTask {
    /// Task metadata
    pub metadata: TaskMetadata,
    /// Boxed execution function
    execute_fn: BoxedTaskFn,
}

impl RegisteredTask {
    /// Create a new registered task
    pub fn new(metadata: TaskMetadata, execute_fn: BoxedTaskFn) -> Self {
        Self {
            metadata,
            execute_fn,
        }
    }

    /// Execute the task
    pub async fn execute(
        &self,
        ctx: Arc<dyn TaskContext + Send + Sync>,
        input: Value,
    ) -> Result<Value> {
        (self.execute_fn)(ctx, input).await
    }
}

impl std::fmt::Debug for RegisteredTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredTask")
            .field("metadata", &self.metadata)
            .field("execute_fn", &"<function>")
            .finish()
    }
}

/// Registry for code-first task definitions.
/// Workers register their task implementations here.
#[derive(Default)]
pub struct TaskRegistry {
    tasks: RwLock<HashMap<String, Arc<RegisteredTask>>>,
}

impl TaskRegistry {
    /// Create a new empty task registry
    pub fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
        }
    }

    /// Register a task with metadata and execution function
    pub fn register_raw(&self, task: RegisteredTask) -> Result<()> {
        let kind = task.metadata.kind.clone();
        let mut tasks = self.tasks.write();

        if tasks.contains_key(&kind) {
            return Err(FlovynError::InvalidConfiguration(format!(
                "Task '{}' is already registered. Each task kind must be unique within a worker.",
                kind
            )));
        }

        tasks.insert(kind, Arc::new(task));
        Ok(())
    }

    /// Register a task definition
    ///
    /// This is the primary way to register tasks. The task's metadata
    /// is extracted from the trait implementation.
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register(MyTask);
    /// ```
    pub fn register<T, I, O>(&self, task: T) -> Result<()>
    where
        T: TaskDefinition<Input = I, Output = O> + 'static,
        I: Serialize + DeserializeOwned + Send + 'static,
        O: Serialize + DeserializeOwned + Send + 'static,
    {
        let metadata = TaskMetadata {
            kind: task.kind().to_string(),
            name: task.name().to_string(),
            description: task.description().map(|s| s.to_string()),
            version: Some(task.version()),
            tags: task.tags(),
            timeout_seconds: task.timeout_seconds(),
            cancellable: task.cancellable(),
            heartbeat_timeout_seconds: task.heartbeat_timeout_seconds(),
        };

        let task = Arc::new(task);

        let execute_fn: BoxedTaskFn = Box::new(move |ctx, input| {
            let task = Arc::clone(&task);
            Box::pin(async move {
                let typed_input: I =
                    serde_json::from_value(input).map_err(FlovynError::Serialization)?;
                let output = task.execute(typed_input, ctx.as_ref()).await?;
                serde_json::to_value(output).map_err(FlovynError::Serialization)
            })
        });

        self.register_raw(RegisteredTask::new(metadata, execute_fn))
    }

    /// Register a simple task with just a kind and execution function
    pub fn register_simple<F, Fut>(&self, kind: &str, execute_fn: F) -> Result<()>
    where
        F: Fn(Arc<dyn TaskContext + Send + Sync>, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        let metadata = TaskMetadata {
            kind: kind.to_string(),
            name: kind.to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: None,
            cancellable: true,
            heartbeat_timeout_seconds: None,
        };

        let boxed_fn: BoxedTaskFn = Box::new(move |ctx, input| {
            let fut = execute_fn(ctx, input);
            Box::pin(fut)
        });

        self.register_raw(RegisteredTask::new(metadata, boxed_fn))
    }

    /// Get a registered task by kind
    pub fn get(&self, kind: &str) -> Option<Arc<RegisteredTask>> {
        self.tasks.read().get(kind).cloned()
    }

    /// Check if a task kind is registered
    pub fn has(&self, kind: &str) -> bool {
        self.tasks.read().contains_key(kind)
    }

    /// Get all registered task kinds
    pub fn get_registered_kinds(&self) -> Vec<String> {
        self.tasks.read().keys().cloned().collect()
    }

    /// Check if there are any registered tasks
    pub fn has_registrations(&self) -> bool {
        !self.tasks.read().is_empty()
    }

    /// Get all task metadata
    pub fn get_all_metadata(&self) -> Vec<TaskMetadata> {
        self.tasks
            .read()
            .values()
            .map(|t| t.metadata.clone())
            .collect()
    }

    /// Get the number of registered tasks
    pub fn len(&self) -> usize {
        self.tasks.read().len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.tasks.read().is_empty()
    }
}

impl std::fmt::Debug for TaskRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kinds: Vec<_> = self.get_registered_kinds();
        f.debug_struct("TaskRegistry")
            .field("tasks", &kinds)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::context_impl::TaskContextImpl;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn test_task_metadata() {
        let metadata = TaskMetadata {
            kind: "process-image".to_string(),
            name: "Process Image".to_string(),
            description: Some("Processes images".to_string()),
            version: Some(SemanticVersion::new(1, 0, 0)),
            tags: vec!["image".to_string(), "processing".to_string()],
            timeout_seconds: Some(300),
            cancellable: true,
            heartbeat_timeout_seconds: Some(60),
        };

        assert_eq!(metadata.kind, "process-image");
        assert_eq!(metadata.name, "Process Image");
        assert!(metadata.description.is_some());
        assert!(metadata.version.is_some());
        assert_eq!(metadata.tags.len(), 2);
        assert_eq!(metadata.timeout_seconds, Some(300));
        assert!(metadata.cancellable);
        assert_eq!(metadata.heartbeat_timeout_seconds, Some(60));
    }

    #[test]
    fn test_task_registry_new() {
        let registry = TaskRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(!registry.has_registrations());
    }

    #[test]
    fn test_task_registry_register_simple() {
        let registry = TaskRegistry::new();

        let result = registry.register_simple("test-task", |_ctx, input| async move {
            Ok(json!({"received": input}))
        });

        assert!(result.is_ok());
        assert!(registry.has("test-task"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_task_registry_duplicate_registration() {
        let registry = TaskRegistry::new();

        registry
            .register_simple("test-task", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let result =
            registry.register_simple("test-task", |_ctx, _input| async move { Ok(json!({})) });

        assert!(result.is_err());
        match result {
            Err(FlovynError::InvalidConfiguration(msg)) => {
                assert!(msg.contains("already registered"));
            }
            _ => panic!("Expected InvalidConfiguration error"),
        }
    }

    #[test]
    fn test_task_registry_get() {
        let registry = TaskRegistry::new();

        registry
            .register_simple("my-task", |_ctx, _input| async move {
                Ok(json!({"status": "ok"}))
            })
            .unwrap();

        let task = registry.get("my-task");
        assert!(task.is_some());
        assert_eq!(task.unwrap().metadata.kind, "my-task");

        let missing = registry.get("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_task_registry_has() {
        let registry = TaskRegistry::new();

        registry
            .register_simple("task-a", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        assert!(registry.has("task-a"));
        assert!(!registry.has("task-b"));
    }

    #[test]
    fn test_task_registry_get_registered_kinds() {
        let registry = TaskRegistry::new();

        registry
            .register_simple("task-1", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("task-2", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("task-3", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let kinds = registry.get_registered_kinds();
        assert_eq!(kinds.len(), 3);
        assert!(kinds.contains(&"task-1".to_string()));
        assert!(kinds.contains(&"task-2".to_string()));
        assert!(kinds.contains(&"task-3".to_string()));
    }

    #[test]
    fn test_task_registry_get_all_metadata() {
        let registry = TaskRegistry::new();

        registry
            .register_simple("task-a", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("task-b", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let metadata = registry.get_all_metadata();
        assert_eq!(metadata.len(), 2);

        let kinds: Vec<_> = metadata.iter().map(|m| m.kind.clone()).collect();
        assert!(kinds.contains(&"task-a".to_string()));
        assert!(kinds.contains(&"task-b".to_string()));
    }

    #[test]
    fn test_task_registry_register_with_metadata() {
        let registry = TaskRegistry::new();

        let metadata = TaskMetadata {
            kind: "complex-task".to_string(),
            name: "Complex Task".to_string(),
            description: Some("A complex task".to_string()),
            version: Some(SemanticVersion::new(2, 1, 0)),
            tags: vec!["complex".to_string()],
            timeout_seconds: Some(600),
            cancellable: false,
            heartbeat_timeout_seconds: Some(120),
        };

        let execute_fn: BoxedTaskFn =
            Box::new(|_ctx, input| Box::pin(async move { Ok(json!({"processed": input})) }));

        let result = registry.register_raw(RegisteredTask::new(metadata, execute_fn));
        assert!(result.is_ok());

        let task = registry.get("complex-task").unwrap();
        assert_eq!(task.metadata.name, "Complex Task");
        assert_eq!(
            task.metadata.description,
            Some("A complex task".to_string())
        );
        assert!(!task.metadata.cancellable);
        assert_eq!(task.metadata.timeout_seconds, Some(600));
    }

    #[tokio::test]
    async fn test_registered_task_execute() {
        let metadata = TaskMetadata {
            kind: "adder".to_string(),
            name: "Adder".to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: None,
            cancellable: true,
            heartbeat_timeout_seconds: None,
        };

        let execute_fn: BoxedTaskFn = Box::new(|_ctx, input| {
            Box::pin(async move {
                let a = input.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
                let b = input.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
                Ok(json!({"sum": a + b}))
            })
        });

        let registered = RegisteredTask::new(metadata, execute_fn);

        let ctx = TaskContextImpl::new(Uuid::new_v4(), 1);

        let result = registered
            .execute(Arc::new(ctx), json!({"a": 10, "b": 20}))
            .await
            .unwrap();

        assert_eq!(result, json!({"sum": 30}));
    }

    #[test]
    fn test_task_registry_debug() {
        let registry = TaskRegistry::new();

        registry
            .register_simple("task-1", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("task-2", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let debug_str = format!("{:?}", registry);
        assert!(debug_str.contains("TaskRegistry"));
    }

    #[test]
    fn test_registered_task_debug() {
        let metadata = TaskMetadata {
            kind: "test".to_string(),
            name: "Test".to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: None,
            cancellable: true,
            heartbeat_timeout_seconds: None,
        };

        let execute_fn: BoxedTaskFn = Box::new(|_ctx, _input| Box::pin(async { Ok(json!({})) }));

        let registered = RegisteredTask::new(metadata, execute_fn);
        let debug_str = format!("{:?}", registered);

        assert!(debug_str.contains("RegisteredTask"));
        assert!(debug_str.contains("test"));
    }
}
