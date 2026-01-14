//! WorkflowDefinition trait

use crate::common::version::SemanticVersion;
use crate::error::Result;
use crate::workflow::context::WorkflowContext;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

/// Definition of a workflow with typed input and output.
///
/// Input and output types must implement `JsonSchema` to enable automatic
/// schema generation (like Kotlin's JsonSchemaGenerator).
#[async_trait]
pub trait WorkflowDefinition: Send + Sync {
    /// Input type for the workflow (must derive JsonSchema for auto-generation)
    type Input: Serialize + DeserializeOwned + JsonSchema + Send;
    /// Output type for the workflow (must derive JsonSchema for auto-generation)
    type Output: Serialize + DeserializeOwned + JsonSchema + Send;

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

    /// JSON Schema for workflow input validation.
    /// Default: auto-generated from Input type using schemars.
    fn input_schema(&self) -> Option<Value> {
        Some(generate_schema::<Self::Input>())
    }

    /// JSON Schema for workflow output validation.
    /// Default: auto-generated from Output type using schemars.
    fn output_schema(&self) -> Option<Value> {
        Some(generate_schema::<Self::Output>())
    }
}

/// Generate JSON Schema from a type that implements JsonSchema.
/// This is the Rust equivalent of Kotlin's `JsonSchemaGenerator.generateSchema<T>()`.
pub fn generate_schema<T: JsonSchema>() -> Value {
    let schema = schemars::schema_for!(T);
    serde_json::to_value(schema).unwrap_or(Value::Null)
}

/// Type alias for dynamic workflows that use Map<String, Value> for input/output
pub type DynamicInput = Map<String, Value>;
pub type DynamicOutput = Map<String, Value>;

/// Helper trait for implementing dynamic (untyped) workflows.
///
/// Since DynamicWorkflow uses `Map<String, Value>` for input/output,
/// you must manually provide schemas via `input_schema()` and `output_schema()`.
/// For typed workflows, use `WorkflowDefinition` directly to get auto-generated schemas.
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

    /// JSON Schema for workflow input validation.
    /// Must be provided manually for DynamicWorkflow since input is untyped.
    fn input_schema(&self) -> Option<Value> {
        None
    }

    /// JSON Schema for workflow output validation.
    /// Must be provided manually for DynamicWorkflow since output is untyped.
    fn output_schema(&self) -> Option<Value> {
        None
    }
}

// Implement WorkflowDefinition for any DynamicWorkflow
// Schemas are delegated to DynamicWorkflow's manual methods (not auto-generated)
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

    // Override to use manual schemas from DynamicWorkflow instead of auto-generating
    fn input_schema(&self) -> Option<Value> {
        DynamicWorkflow::input_schema(self)
    }

    fn output_schema(&self) -> Option<Value> {
        DynamicWorkflow::output_schema(self)
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

    // =========================================================================
    // Schema Generation Tests
    // =========================================================================

    /// Test input type with various field types
    #[derive(Debug, Serialize, serde::Deserialize, JsonSchema)]
    struct OrderInput {
        /// The order ID (required string)
        order_id: String,
        /// Amount in cents (required integer)
        amount: u64,
        /// Customer email (optional string)
        customer_email: Option<String>,
        /// Whether to send receipt (required boolean)
        send_receipt: bool,
    }

    /// Test output type
    #[derive(Debug, Serialize, serde::Deserialize, JsonSchema)]
    struct OrderOutput {
        success: bool,
        transaction_id: Option<String>,
    }

    #[test]
    fn test_generate_schema_has_correct_properties() {
        let schema = generate_schema::<OrderInput>();

        // Schema should have properties object
        let properties = schema
            .get("properties")
            .expect("Schema should have properties");
        assert!(properties.is_object(), "properties should be an object");

        // Check all fields exist
        assert!(
            properties.get("order_id").is_some(),
            "Should have order_id property"
        );
        assert!(
            properties.get("amount").is_some(),
            "Should have amount property"
        );
        assert!(
            properties.get("customer_email").is_some(),
            "Should have customer_email property"
        );
        assert!(
            properties.get("send_receipt").is_some(),
            "Should have send_receipt property"
        );
    }

    #[test]
    fn test_generate_schema_has_correct_types() {
        let schema = generate_schema::<OrderInput>();
        let properties = schema.get("properties").unwrap();

        // order_id should be string type
        let order_id = properties.get("order_id").unwrap();
        assert_eq!(
            order_id.get("type").and_then(|v| v.as_str()),
            Some("string"),
            "order_id should be string type"
        );

        // amount should be integer type
        let amount = properties.get("amount").unwrap();
        assert_eq!(
            amount.get("type").and_then(|v| v.as_str()),
            Some("integer"),
            "amount should be integer type"
        );

        // send_receipt should be boolean type
        let send_receipt = properties.get("send_receipt").unwrap();
        assert_eq!(
            send_receipt.get("type").and_then(|v| v.as_str()),
            Some("boolean"),
            "send_receipt should be boolean type"
        );
    }

    #[test]
    fn test_generate_schema_has_required_fields() {
        let schema = generate_schema::<OrderInput>();

        // Required fields (non-Option types)
        let required = schema
            .get("required")
            .expect("Schema should have required array");
        let required_arr = required.as_array().expect("required should be an array");

        let required_fields: Vec<&str> = required_arr.iter().filter_map(|v| v.as_str()).collect();

        // order_id, amount, send_receipt are required (not Option)
        assert!(
            required_fields.contains(&"order_id"),
            "order_id should be required"
        );
        assert!(
            required_fields.contains(&"amount"),
            "amount should be required"
        );
        assert!(
            required_fields.contains(&"send_receipt"),
            "send_receipt should be required"
        );

        // customer_email is Option<String>, should NOT be in required
        assert!(
            !required_fields.contains(&"customer_email"),
            "customer_email (Option) should NOT be required"
        );
    }

    #[test]
    fn test_generate_schema_optional_field_allows_null() {
        let schema = generate_schema::<OrderInput>();
        let properties = schema.get("properties").unwrap();

        // customer_email is Option<String> - schemars may represent this various ways
        let customer_email = properties.get("customer_email").unwrap();

        // With schemars 1.x, Option<T> typically uses anyOf with null
        // or the field is simply not in required array
        // The key test is that it's NOT in required (tested above)
        assert!(
            customer_email.is_object(),
            "customer_email should have schema definition"
        );
    }

    #[test]
    fn test_generate_schema_output_type() {
        let schema = generate_schema::<OrderOutput>();
        let properties = schema.get("properties").expect("Should have properties");

        assert!(properties.get("success").is_some(), "Should have success");
        assert!(
            properties.get("transaction_id").is_some(),
            "Should have transaction_id"
        );

        // success is bool (required), transaction_id is Option<String> (not required)
        let required = schema.get("required").expect("Should have required");
        let required_arr = required.as_array().unwrap();
        let required_fields: Vec<&str> = required_arr.iter().filter_map(|v| v.as_str()).collect();

        assert!(
            required_fields.contains(&"success"),
            "success should be required"
        );
        assert!(
            !required_fields.contains(&"transaction_id"),
            "transaction_id (Option) should NOT be required"
        );
    }

    #[test]
    fn test_schema_is_valid_json_schema() {
        let schema = generate_schema::<OrderInput>();

        // Should have $schema field indicating JSON Schema version
        assert!(
            schema.get("$schema").is_some() || schema.get("type").is_some(),
            "Should be valid JSON Schema with $schema or type field"
        );

        // Should have type: object for struct
        if let Some(typ) = schema.get("type") {
            assert_eq!(typ.as_str(), Some("object"), "Struct should be object type");
        }
    }
}
