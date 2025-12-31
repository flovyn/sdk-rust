//! WorkflowRegistry - Registry for workflow definitions

use crate::common::version::SemanticVersion;
use crate::error::{FlovynError, Result};
use crate::workflow::context::WorkflowContext;
use crate::workflow::definition::WorkflowDefinition;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Workflow metadata extracted from a workflow definition
#[derive(Debug, Clone)]
pub struct WorkflowMetadata {
    /// Unique workflow kind identifier
    pub kind: String,
    /// Human-readable name
    pub name: String,
    /// Description of the workflow
    pub description: Option<String>,
    /// Version of the workflow
    pub version: Option<SemanticVersion>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Whether the workflow can be cancelled
    pub cancellable: bool,
    /// Timeout in seconds
    pub timeout_seconds: Option<u64>,
    /// SHA-256 hash of workflow content for version validation
    pub content_hash: Option<String>,
    /// JSON Schema for input validation (auto-generated or manual)
    pub input_schema: Option<Value>,
    /// JSON Schema for output validation (auto-generated or manual)
    pub output_schema: Option<Value>,
}

impl From<WorkflowMetadata> for flovyn_sdk_core::WorkflowMetadata {
    fn from(m: WorkflowMetadata) -> Self {
        flovyn_sdk_core::WorkflowMetadata {
            kind: m.kind,
            name: m.name,
            description: m.description,
            version: m.version.map(|v| v.to_string()),
            tags: m.tags,
            cancellable: m.cancellable,
            timeout_seconds: m.timeout_seconds,
            content_hash: m.content_hash,
            input_schema: m.input_schema,
            output_schema: m.output_schema,
        }
    }
}

/// Type alias for boxed workflow execution functions
pub type BoxedWorkflowFn = Box<
    dyn Fn(
            Arc<dyn WorkflowContext + Send + Sync>,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// A registered workflow with its metadata and execution function
pub struct RegisteredWorkflow {
    /// Workflow metadata
    pub metadata: WorkflowMetadata,
    /// Boxed execution function
    execute_fn: BoxedWorkflowFn,
}

impl RegisteredWorkflow {
    /// Create a new registered workflow
    pub fn new(metadata: WorkflowMetadata, execute_fn: BoxedWorkflowFn) -> Self {
        Self {
            metadata,
            execute_fn,
        }
    }

    /// Execute the workflow
    pub async fn execute(
        &self,
        ctx: Arc<dyn WorkflowContext + Send + Sync>,
        input: Value,
    ) -> Result<Value> {
        (self.execute_fn)(ctx, input).await
    }
}

impl std::fmt::Debug for RegisteredWorkflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredWorkflow")
            .field("metadata", &self.metadata)
            .field("execute_fn", &"<function>")
            .finish()
    }
}

/// Registry for code-first workflow definitions.
/// Workers register their workflow implementations here.
#[derive(Default)]
pub struct WorkflowRegistry {
    workflows: RwLock<HashMap<String, Arc<RegisteredWorkflow>>>,
}

impl WorkflowRegistry {
    /// Create a new empty workflow registry
    pub fn new() -> Self {
        Self {
            workflows: RwLock::new(HashMap::new()),
        }
    }

    /// Register a workflow with metadata and execution function
    pub fn register_raw(&self, workflow: RegisteredWorkflow) -> Result<()> {
        let kind = workflow.metadata.kind.clone();
        let mut workflows = self.workflows.write();

        if workflows.contains_key(&kind) {
            return Err(FlovynError::InvalidConfiguration(format!(
                "Workflow '{}' is already registered. Each workflow kind must be unique within a worker.",
                kind
            )));
        }

        workflows.insert(kind, Arc::new(workflow));
        Ok(())
    }

    /// Register a workflow definition
    ///
    /// This is the primary way to register workflows. The workflow's metadata
    /// is extracted from the trait implementation.
    ///
    /// Schemas are auto-generated from Input/Output types using schemars (like Kotlin's JsonSchemaGenerator).
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register(MyWorkflow);
    /// ```
    pub fn register<W, I, O>(&self, workflow: W) -> Result<()>
    where
        W: WorkflowDefinition<Input = I, Output = O> + 'static,
        I: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
        O: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
    {
        // Get schemas from the workflow (auto-generated or manual depending on impl)
        let input_schema = workflow.input_schema();
        let output_schema = workflow.output_schema();

        let metadata = WorkflowMetadata {
            kind: workflow.kind().to_string(),
            name: workflow.name().to_string(),
            description: workflow.description().map(|s| s.to_string()),
            version: Some(workflow.version()),
            tags: workflow.tags(),
            cancellable: workflow.cancellable(),
            timeout_seconds: workflow.timeout_seconds().map(|s| s as u64),
            content_hash: None, // Will be computed during registration
            input_schema,
            output_schema,
        };

        let workflow = Arc::new(workflow);

        let execute_fn: BoxedWorkflowFn = Box::new(move |ctx, input| {
            let workflow = Arc::clone(&workflow);
            Box::pin(async move {
                let typed_input: I =
                    serde_json::from_value(input).map_err(FlovynError::Serialization)?;
                let output = workflow.execute(ctx.as_ref(), typed_input).await?;
                serde_json::to_value(output).map_err(FlovynError::Serialization)
            })
        });

        self.register_raw(RegisteredWorkflow::new(metadata, execute_fn))
    }

    /// Register a simple workflow with just a kind and execution function.
    /// Note: No schema is generated for simple workflows since there's no type information.
    pub fn register_simple<F, Fut>(&self, kind: &str, execute_fn: F) -> Result<()>
    where
        F: Fn(Arc<dyn WorkflowContext + Send + Sync>, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        let metadata = WorkflowMetadata {
            kind: kind.to_string(),
            name: kind.to_string(),
            description: None,
            version: None,
            tags: vec![],
            cancellable: true,
            timeout_seconds: None,
            content_hash: None,
            input_schema: None,
            output_schema: None,
        };

        let boxed_fn: BoxedWorkflowFn = Box::new(move |ctx, input| {
            let fut = execute_fn(ctx, input);
            Box::pin(fut)
        });

        self.register_raw(RegisteredWorkflow::new(metadata, boxed_fn))
    }

    /// Get a registered workflow by kind
    pub fn get(&self, kind: &str) -> Option<Arc<RegisteredWorkflow>> {
        self.workflows.read().get(kind).cloned()
    }

    /// Check if a workflow kind is registered
    pub fn has(&self, kind: &str) -> bool {
        self.workflows.read().contains_key(kind)
    }

    /// Get all registered workflow kinds
    pub fn get_registered_kinds(&self) -> Vec<String> {
        self.workflows.read().keys().cloned().collect()
    }

    /// Check if there are any registered workflows
    pub fn has_registrations(&self) -> bool {
        !self.workflows.read().is_empty()
    }

    /// Get all workflow metadata
    pub fn get_all_metadata(&self) -> Vec<WorkflowMetadata> {
        self.workflows
            .read()
            .values()
            .map(|w| w.metadata.clone())
            .collect()
    }

    /// Get the number of registered workflows
    pub fn len(&self) -> usize {
        self.workflows.read().len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.workflows.read().is_empty()
    }
}

impl std::fmt::Debug for WorkflowRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kinds: Vec<_> = self.get_registered_kinds();
        f.debug_struct("WorkflowRegistry")
            .field("workflows", &kinds)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_workflow_metadata() {
        let metadata = WorkflowMetadata {
            kind: "payment-workflow".to_string(),
            name: "Payment Workflow".to_string(),
            description: Some("Processes payments".to_string()),
            version: Some(SemanticVersion::new(1, 0, 0)),
            tags: vec!["payment".to_string(), "finance".to_string()],
            cancellable: true,
            timeout_seconds: Some(300),
            content_hash: None,
            input_schema: Some(json!({"type": "object"})),
            output_schema: None,
        };

        assert_eq!(metadata.kind, "payment-workflow");
        assert_eq!(metadata.name, "Payment Workflow");
        assert!(metadata.description.is_some());
        assert!(metadata.version.is_some());
        assert_eq!(metadata.tags.len(), 2);
        assert!(metadata.cancellable);
        assert_eq!(metadata.timeout_seconds, Some(300));
        assert!(metadata.input_schema.is_some());
    }

    #[test]
    fn test_workflow_registry_new() {
        let registry = WorkflowRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(!registry.has_registrations());
    }

    #[test]
    fn test_workflow_registry_register_simple() {
        let registry = WorkflowRegistry::new();

        let result = registry.register_simple("test-workflow", |_ctx, input| async move {
            Ok(json!({"received": input}))
        });

        assert!(result.is_ok());
        assert!(registry.has("test-workflow"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_workflow_registry_duplicate_registration() {
        let registry = WorkflowRegistry::new();

        registry
            .register_simple("test-workflow", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let result =
            registry.register_simple("test-workflow", |_ctx, _input| async move { Ok(json!({})) });

        assert!(result.is_err());
        match result {
            Err(FlovynError::InvalidConfiguration(msg)) => {
                assert!(msg.contains("already registered"));
            }
            _ => panic!("Expected InvalidConfiguration error"),
        }
    }

    #[test]
    fn test_workflow_registry_get() {
        let registry = WorkflowRegistry::new();

        registry
            .register_simple("my-workflow", |_ctx, _input| async move {
                Ok(json!({"status": "ok"}))
            })
            .unwrap();

        let workflow = registry.get("my-workflow");
        assert!(workflow.is_some());
        assert_eq!(workflow.unwrap().metadata.kind, "my-workflow");

        let missing = registry.get("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_workflow_registry_has() {
        let registry = WorkflowRegistry::new();

        registry
            .register_simple("workflow-a", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        assert!(registry.has("workflow-a"));
        assert!(!registry.has("workflow-b"));
    }

    #[test]
    fn test_workflow_registry_get_registered_kinds() {
        let registry = WorkflowRegistry::new();

        registry
            .register_simple("workflow-1", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("workflow-2", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("workflow-3", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let kinds = registry.get_registered_kinds();
        assert_eq!(kinds.len(), 3);
        assert!(kinds.contains(&"workflow-1".to_string()));
        assert!(kinds.contains(&"workflow-2".to_string()));
        assert!(kinds.contains(&"workflow-3".to_string()));
    }

    #[test]
    fn test_workflow_registry_get_all_metadata() {
        let registry = WorkflowRegistry::new();

        registry
            .register_simple("workflow-a", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("workflow-b", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let metadata = registry.get_all_metadata();
        assert_eq!(metadata.len(), 2);

        let kinds: Vec<_> = metadata.iter().map(|m| m.kind.clone()).collect();
        assert!(kinds.contains(&"workflow-a".to_string()));
        assert!(kinds.contains(&"workflow-b".to_string()));
    }

    #[test]
    fn test_workflow_registry_register_with_metadata() {
        let registry = WorkflowRegistry::new();

        let metadata = WorkflowMetadata {
            kind: "complex-workflow".to_string(),
            name: "Complex Workflow".to_string(),
            description: Some("A complex workflow".to_string()),
            version: Some(SemanticVersion::new(2, 1, 0)),
            tags: vec!["complex".to_string()],
            cancellable: false,
            timeout_seconds: Some(600),
            content_hash: None,
            input_schema: None,
            output_schema: None,
        };

        let execute_fn: BoxedWorkflowFn =
            Box::new(|_ctx, input| Box::pin(async move { Ok(json!({"processed": input})) }));

        let result = registry.register_raw(RegisteredWorkflow::new(metadata, execute_fn));
        assert!(result.is_ok());

        let workflow = registry.get("complex-workflow").unwrap();
        assert_eq!(workflow.metadata.name, "Complex Workflow");
        assert_eq!(
            workflow.metadata.description,
            Some("A complex workflow".to_string())
        );
        assert!(!workflow.metadata.cancellable);
        assert_eq!(workflow.metadata.timeout_seconds, Some(600));
    }

    #[tokio::test]
    async fn test_registered_workflow_execute() {
        use crate::workflow::context_impl::WorkflowContextImpl;
        use crate::workflow::recorder::CommandCollector;
        use uuid::Uuid;

        let metadata = WorkflowMetadata {
            kind: "adder".to_string(),
            name: "Adder".to_string(),
            description: None,
            version: None,
            tags: vec![],
            cancellable: true,
            timeout_seconds: None,
            content_hash: None,
            input_schema: None,
            output_schema: None,
        };

        let execute_fn: BoxedWorkflowFn = Box::new(|_ctx, input| {
            Box::pin(async move {
                let a = input.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
                let b = input.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
                Ok(json!({"sum": a + b}))
            })
        });

        let registered = RegisteredWorkflow::new(metadata, execute_fn);

        let ctx = WorkflowContextImpl::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            json!({}),
            CommandCollector::new(),
            vec![],
            chrono::Utc::now().timestamp_millis(),
        );

        let result = registered
            .execute(Arc::new(ctx), json!({"a": 10, "b": 20}))
            .await
            .unwrap();

        assert_eq!(result, json!({"sum": 30}));
    }

    #[test]
    fn test_workflow_registry_debug() {
        let registry = WorkflowRegistry::new();

        registry
            .register_simple("wf-1", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("wf-2", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let debug_str = format!("{:?}", registry);
        assert!(debug_str.contains("WorkflowRegistry"));
    }

    #[test]
    fn test_registered_workflow_debug() {
        let metadata = WorkflowMetadata {
            kind: "test".to_string(),
            name: "Test".to_string(),
            description: None,
            version: None,
            tags: vec![],
            cancellable: true,
            timeout_seconds: None,
            content_hash: None,
            input_schema: None,
            output_schema: None,
        };

        let execute_fn: BoxedWorkflowFn =
            Box::new(|_ctx, _input| Box::pin(async { Ok(json!({})) }));

        let registered = RegisteredWorkflow::new(metadata, execute_fn);
        let debug_str = format!("{:?}", registered);

        assert!(debug_str.contains("RegisteredWorkflow"));
        assert!(debug_str.contains("test"));
    }
}
