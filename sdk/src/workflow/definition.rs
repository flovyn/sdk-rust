//! WorkflowDefinition trait

use crate::common::version::SemanticVersion;
use crate::error::Result;
use crate::workflow::context::WorkflowContext;
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

/// Definition of a workflow with typed input and output
#[async_trait]
pub trait WorkflowDefinition: Send + Sync {
    /// Input type for the workflow
    type Input: Serialize + DeserializeOwned + Send;
    /// Output type for the workflow
    type Output: Serialize + DeserializeOwned + Send;

    /// Unique identifier for this workflow type
    fn kind(&self) -> &str;

    /// Execute the workflow with the given context and input
    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output>;

    /// Human-readable name for the workflow (defaults to kind)
    fn name(&self) -> &str {
        self.kind()
    }

    /// Version of this workflow definition
    fn version(&self) -> SemanticVersion {
        SemanticVersion::default()
    }

    /// Optional description of the workflow
    fn description(&self) -> Option<&str> {
        None
    }

    /// Timeout in seconds for the entire workflow (None = no timeout)
    fn timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this workflow can be cancelled
    fn cancellable(&self) -> bool {
        false
    }

    /// Tags for categorizing the workflow
    fn tags(&self) -> Vec<String> {
        vec![]
    }
}

/// Type alias for dynamic workflows that use Map<String, Value> for input/output
pub type DynamicInput = Map<String, Value>;
pub type DynamicOutput = Map<String, Value>;

/// Helper trait for implementing dynamic workflows
#[async_trait]
pub trait DynamicWorkflow: Send + Sync {
    /// Unique identifier for this workflow type
    fn kind(&self) -> &str;

    /// Execute the workflow with dynamic input/output
    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: DynamicInput,
    ) -> Result<DynamicOutput>;

    /// Human-readable name for the workflow (defaults to kind)
    fn name(&self) -> &str {
        self.kind()
    }

    /// Version of this workflow definition
    fn version(&self) -> SemanticVersion {
        SemanticVersion::default()
    }

    /// Optional description of the workflow
    fn description(&self) -> Option<&str> {
        None
    }

    /// Timeout in seconds for the entire workflow (None = no timeout)
    fn timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this workflow can be cancelled
    fn cancellable(&self) -> bool {
        false
    }

    /// Tags for categorizing the workflow
    fn tags(&self) -> Vec<String> {
        vec![]
    }
}

// Implement WorkflowDefinition for any DynamicWorkflow
#[async_trait]
impl<T: DynamicWorkflow> WorkflowDefinition for T {
    type Input = DynamicInput;
    type Output = DynamicOutput;

    fn kind(&self) -> &str {
        DynamicWorkflow::kind(self)
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        DynamicWorkflow::execute(self, ctx, input).await
    }

    fn name(&self) -> &str {
        DynamicWorkflow::name(self)
    }

    fn version(&self) -> SemanticVersion {
        DynamicWorkflow::version(self)
    }

    fn description(&self) -> Option<&str> {
        DynamicWorkflow::description(self)
    }

    fn timeout_seconds(&self) -> Option<u32> {
        DynamicWorkflow::timeout_seconds(self)
    }

    fn cancellable(&self) -> bool {
        DynamicWorkflow::cancellable(self)
    }

    fn tags(&self) -> Vec<String> {
        DynamicWorkflow::tags(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test struct for DynamicWorkflow
    struct TestDynamicWorkflow;

    #[async_trait]
    impl DynamicWorkflow for TestDynamicWorkflow {
        fn kind(&self) -> &str {
            "test-workflow"
        }

        async fn execute(
            &self,
            _ctx: &dyn WorkflowContext,
            mut input: DynamicInput,
        ) -> Result<DynamicOutput> {
            // Echo the input with a result field
            input.insert("processed".to_string(), Value::Bool(true));
            Ok(input)
        }

        fn version(&self) -> SemanticVersion {
            SemanticVersion::new(1, 2, 3)
        }

        fn description(&self) -> Option<&str> {
            Some("A test workflow")
        }

        fn cancellable(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_dynamic_workflow_kind() {
        let workflow = TestDynamicWorkflow;
        assert_eq!(DynamicWorkflow::kind(&workflow), "test-workflow");
    }

    #[test]
    fn test_dynamic_workflow_name_defaults_to_kind() {
        let workflow = TestDynamicWorkflow;
        assert_eq!(
            DynamicWorkflow::name(&workflow),
            DynamicWorkflow::kind(&workflow)
        );
    }

    #[test]
    fn test_dynamic_workflow_version() {
        let workflow = TestDynamicWorkflow;
        assert_eq!(
            DynamicWorkflow::version(&workflow),
            SemanticVersion::new(1, 2, 3)
        );
    }

    #[test]
    fn test_dynamic_workflow_description() {
        let workflow = TestDynamicWorkflow;
        assert_eq!(
            DynamicWorkflow::description(&workflow),
            Some("A test workflow")
        );
    }

    #[test]
    fn test_dynamic_workflow_timeout_defaults_to_none() {
        let workflow = TestDynamicWorkflow;
        assert_eq!(DynamicWorkflow::timeout_seconds(&workflow), None);
    }

    #[test]
    fn test_dynamic_workflow_cancellable() {
        let workflow = TestDynamicWorkflow;
        assert!(DynamicWorkflow::cancellable(&workflow));
    }

    #[test]
    fn test_dynamic_workflow_tags_default_empty() {
        let workflow = TestDynamicWorkflow;
        assert!(DynamicWorkflow::tags(&workflow).is_empty());
    }

    // Test that WorkflowDefinition is implemented for DynamicWorkflow
    #[test]
    fn test_workflow_definition_impl() {
        let workflow = TestDynamicWorkflow;
        let def: &dyn WorkflowDefinition<Input = DynamicInput, Output = DynamicOutput> = &workflow;
        assert_eq!(def.kind(), "test-workflow");
        assert_eq!(def.version(), SemanticVersion::new(1, 2, 3));
    }
}
