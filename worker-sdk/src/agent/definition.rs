//! AgentDefinition trait

use crate::agent::context::AgentContext;
use crate::common::version::SemanticVersion;
use crate::error::Result;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

/// Generate JSON Schema from a type that implements JsonSchema.
pub fn generate_schema<T: JsonSchema>() -> Value {
    let schema = schemars::schema_for!(T);
    serde_json::to_value(schema).unwrap_or(Value::Null)
}

/// Definition of an agent with typed input and output.
///
/// Agents are long-running, conversation-style AI workloads that support:
/// - Tree-structured conversation entries
/// - Lightweight checkpointing for recovery
/// - Task scheduling with result waiting
/// - Signal-based user interaction
/// - Real-time streaming to clients
///
/// Input and output types must implement `JsonSchema` to enable automatic
/// schema generation.
///
/// # Example
///
/// ```rust,ignore
/// use flovyn_worker_sdk::prelude::*;
///
/// #[derive(Default)]
/// struct EchoAgent;
///
/// #[async_trait]
/// impl AgentDefinition for EchoAgent {
///     type Input = String;
///     type Output = String;
///
///     fn kind(&self) -> &str { "echo-agent" }
///
///     async fn execute(&self, ctx: &dyn AgentContext, input: Self::Input) -> Result<Self::Output> {
///         ctx.append_entry(EntryRole::User, &json!({"text": &input})).await?;
///         ctx.checkpoint(&json!({})).await?;
///         Ok(format!("Echo: {}", input))
///     }
/// }
/// ```
#[async_trait]
pub trait AgentDefinition: Send + Sync {
    /// Input type for the agent (must derive JsonSchema for auto-generation)
    type Input: Serialize + DeserializeOwned + JsonSchema + Send;
    /// Output type for the agent (must derive JsonSchema for auto-generation)
    type Output: Serialize + DeserializeOwned + JsonSchema + Send;

    /// Unique identifier for this agent type (e.g., "react-agent", "code-agent")
    fn kind(&self) -> &str;

    /// Execute the agent with the given context and input
    async fn execute(&self, ctx: &dyn AgentContext, input: Self::Input) -> Result<Self::Output>;

    /// Human-readable name for the agent (defaults to kind)
    fn name(&self) -> &str {
        self.kind()
    }

    /// Version of this agent definition
    fn version(&self) -> SemanticVersion {
        SemanticVersion::default()
    }

    /// Optional description of the agent
    fn description(&self) -> Option<&str> {
        None
    }

    /// Timeout in seconds for agent execution (None = no timeout)
    fn timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this agent can be cancelled
    fn cancellable(&self) -> bool {
        true
    }

    /// Tags for categorizing the agent
    fn tags(&self) -> Vec<String> {
        vec![]
    }

    /// JSON Schema for agent input validation.
    /// Default: auto-generated from Input type using schemars.
    fn input_schema(&self) -> Option<Value> {
        Some(generate_schema::<Self::Input>())
    }

    /// JSON Schema for agent output validation.
    /// Default: auto-generated from Output type using schemars.
    fn output_schema(&self) -> Option<Value> {
        Some(generate_schema::<Self::Output>())
    }
}

/// Type alias for dynamic agent input/output
pub type DynamicAgentInput = Map<String, Value>;
pub type DynamicAgentOutput = Map<String, Value>;

/// Helper trait for implementing dynamic (untyped) agents.
///
/// Since DynamicAgent uses `Map<String, Value>` for input/output,
/// you must manually provide schemas via `input_schema()` and `output_schema()`.
/// For typed agents, use `AgentDefinition` directly to get auto-generated schemas.
#[async_trait]
pub trait DynamicAgent: Send + Sync {
    /// Unique identifier for this agent type
    fn kind(&self) -> &str;

    /// Execute the agent with dynamic input/output
    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput>;

    /// Human-readable name for the agent (defaults to kind)
    fn name(&self) -> &str {
        self.kind()
    }

    /// Version of this agent definition
    fn version(&self) -> SemanticVersion {
        SemanticVersion::default()
    }

    /// Optional description of the agent
    fn description(&self) -> Option<&str> {
        None
    }

    /// Timeout in seconds for agent execution (None = no timeout)
    fn timeout_seconds(&self) -> Option<u32> {
        None
    }

    /// Whether this agent can be cancelled
    fn cancellable(&self) -> bool {
        true
    }

    /// Tags for categorizing the agent
    fn tags(&self) -> Vec<String> {
        vec![]
    }

    /// JSON Schema for agent input validation.
    /// Must be provided manually for DynamicAgent since input is untyped.
    fn input_schema(&self) -> Option<Value> {
        None
    }

    /// JSON Schema for agent output validation.
    /// Must be provided manually for DynamicAgent since output is untyped.
    fn output_schema(&self) -> Option<Value> {
        None
    }
}

// Implement AgentDefinition for any DynamicAgent
#[async_trait]
impl<T: DynamicAgent> AgentDefinition for T {
    type Input = DynamicAgentInput;
    type Output = DynamicAgentOutput;

    fn kind(&self) -> &str {
        DynamicAgent::kind(self)
    }

    async fn execute(&self, ctx: &dyn AgentContext, input: Self::Input) -> Result<Self::Output> {
        DynamicAgent::execute(self, ctx, input).await
    }

    fn name(&self) -> &str {
        DynamicAgent::name(self)
    }

    fn version(&self) -> SemanticVersion {
        DynamicAgent::version(self)
    }

    fn description(&self) -> Option<&str> {
        DynamicAgent::description(self)
    }

    fn timeout_seconds(&self) -> Option<u32> {
        DynamicAgent::timeout_seconds(self)
    }

    fn cancellable(&self) -> bool {
        DynamicAgent::cancellable(self)
    }

    fn tags(&self) -> Vec<String> {
        DynamicAgent::tags(self)
    }

    fn input_schema(&self) -> Option<Value> {
        DynamicAgent::input_schema(self)
    }

    fn output_schema(&self) -> Option<Value> {
        DynamicAgent::output_schema(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test struct for DynamicAgent
    struct TestDynamicAgent;

    #[async_trait]
    impl DynamicAgent for TestDynamicAgent {
        fn kind(&self) -> &str {
            "test-agent"
        }

        async fn execute(
            &self,
            _ctx: &dyn AgentContext,
            mut input: DynamicAgentInput,
        ) -> Result<DynamicAgentOutput> {
            input.insert("processed".to_string(), Value::Bool(true));
            Ok(input)
        }

        fn version(&self) -> SemanticVersion {
            SemanticVersion::new(2, 0, 0)
        }

        fn description(&self) -> Option<&str> {
            Some("A test agent")
        }

        fn timeout_seconds(&self) -> Option<u32> {
            Some(300)
        }

        fn cancellable(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_dynamic_agent_kind() {
        let agent = TestDynamicAgent;
        assert_eq!(DynamicAgent::kind(&agent), "test-agent");
    }

    #[test]
    fn test_dynamic_agent_name_defaults_to_kind() {
        let agent = TestDynamicAgent;
        assert_eq!(DynamicAgent::name(&agent), DynamicAgent::kind(&agent));
    }

    #[test]
    fn test_dynamic_agent_version() {
        let agent = TestDynamicAgent;
        assert_eq!(DynamicAgent::version(&agent), SemanticVersion::new(2, 0, 0));
    }

    #[test]
    fn test_dynamic_agent_description() {
        let agent = TestDynamicAgent;
        assert_eq!(DynamicAgent::description(&agent), Some("A test agent"));
    }

    #[test]
    fn test_dynamic_agent_timeout_seconds() {
        let agent = TestDynamicAgent;
        assert_eq!(DynamicAgent::timeout_seconds(&agent), Some(300));
    }

    #[test]
    fn test_dynamic_agent_cancellable() {
        let agent = TestDynamicAgent;
        assert!(DynamicAgent::cancellable(&agent));
    }

    #[test]
    fn test_dynamic_agent_tags_default_empty() {
        let agent = TestDynamicAgent;
        assert!(DynamicAgent::tags(&agent).is_empty());
    }

    // Test that AgentDefinition is implemented for DynamicAgent
    #[test]
    fn test_agent_definition_impl() {
        let agent = TestDynamicAgent;
        let def: &dyn AgentDefinition<Input = DynamicAgentInput, Output = DynamicAgentOutput> =
            &agent;
        assert_eq!(def.kind(), "test-agent");
        assert_eq!(def.version(), SemanticVersion::new(2, 0, 0));
    }
}
