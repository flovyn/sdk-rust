//! AgentRegistry - Registry for agent definitions

use crate::agent::context::AgentContext;
use crate::agent::definition::AgentDefinition;
use crate::common::version::SemanticVersion;
use crate::error::{FlovynError, Result};
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Agent metadata extracted from an agent definition
#[derive(Debug, Clone)]
pub struct AgentMetadata {
    /// Unique agent kind identifier
    pub kind: String,
    /// Human-readable name
    pub name: String,
    /// Description of the agent
    pub description: Option<String>,
    /// Version of the agent
    pub version: Option<SemanticVersion>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Timeout in seconds
    pub timeout_seconds: Option<u32>,
    /// Whether the agent can be cancelled
    pub cancellable: bool,
    /// JSON Schema for input validation (auto-generated from Input type)
    pub input_schema: Option<Value>,
    /// JSON Schema for output validation (auto-generated from Output type)
    pub output_schema: Option<Value>,
}

/// Type alias for boxed agent execution functions
pub type BoxedAgentFn = Box<
    dyn Fn(
            Arc<dyn AgentContext + Send + Sync>,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// A registered agent with its metadata and execution function
pub struct RegisteredAgent {
    /// Agent metadata
    pub metadata: AgentMetadata,
    /// Boxed execution function
    execute_fn: BoxedAgentFn,
}

impl RegisteredAgent {
    /// Create a new registered agent
    pub fn new(metadata: AgentMetadata, execute_fn: BoxedAgentFn) -> Self {
        Self {
            metadata,
            execute_fn,
        }
    }

    /// Execute the agent
    pub async fn execute(
        &self,
        ctx: Arc<dyn AgentContext + Send + Sync>,
        input: Value,
    ) -> Result<Value> {
        (self.execute_fn)(ctx, input).await
    }
}

impl std::fmt::Debug for RegisteredAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredAgent")
            .field("metadata", &self.metadata)
            .field("execute_fn", &"<function>")
            .finish()
    }
}

/// Registry for code-first agent definitions.
/// Workers register their agent implementations here.
#[derive(Default)]
pub struct AgentRegistry {
    agents: RwLock<HashMap<String, Arc<RegisteredAgent>>>,
}

impl AgentRegistry {
    /// Create a new empty agent registry
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// Register an agent with metadata and execution function
    pub fn register_raw(&self, agent: RegisteredAgent) -> Result<()> {
        let kind = agent.metadata.kind.clone();
        let mut agents = self.agents.write();

        if agents.contains_key(&kind) {
            return Err(FlovynError::InvalidConfiguration(format!(
                "Agent '{}' is already registered. Each agent kind must be unique within a worker.",
                kind
            )));
        }

        agents.insert(kind, Arc::new(agent));
        Ok(())
    }

    /// Register an agent definition
    ///
    /// This is the primary way to register agents. The agent's metadata
    /// is extracted from the trait implementation.
    ///
    /// Schemas are auto-generated from Input/Output types using schemars.
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register(MyAgent);
    /// ```
    pub fn register<A, I, O>(&self, agent: A) -> Result<()>
    where
        A: AgentDefinition<Input = I, Output = O> + 'static,
        I: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
        O: Serialize + DeserializeOwned + schemars::JsonSchema + Send + 'static,
    {
        // Get schemas from the agent (auto-generated or manual depending on impl)
        let input_schema = agent.input_schema();
        let output_schema = agent.output_schema();

        let metadata = AgentMetadata {
            kind: agent.kind().to_string(),
            name: agent.name().to_string(),
            description: agent.description().map(|s| s.to_string()),
            version: Some(agent.version()),
            tags: agent.tags(),
            timeout_seconds: agent.timeout_seconds(),
            cancellable: agent.cancellable(),
            input_schema,
            output_schema,
        };

        let agent = Arc::new(agent);

        let execute_fn: BoxedAgentFn = Box::new(move |ctx, input| {
            let agent = Arc::clone(&agent);
            Box::pin(async move {
                let typed_input: I =
                    serde_json::from_value(input).map_err(FlovynError::Serialization)?;
                let output = agent.execute(ctx.as_ref(), typed_input).await?;
                serde_json::to_value(output).map_err(FlovynError::Serialization)
            })
        });

        self.register_raw(RegisteredAgent::new(metadata, execute_fn))
    }

    /// Register a simple agent with just a kind and execution function.
    /// Note: No schema is generated for simple agents since there's no type information.
    pub fn register_simple<F, Fut>(&self, kind: &str, execute_fn: F) -> Result<()>
    where
        F: Fn(Arc<dyn AgentContext + Send + Sync>, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        let metadata = AgentMetadata {
            kind: kind.to_string(),
            name: kind.to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: None,
            cancellable: true,
            input_schema: None,
            output_schema: None,
        };

        let boxed_fn: BoxedAgentFn = Box::new(move |ctx, input| {
            let fut = execute_fn(ctx, input);
            Box::pin(fut)
        });

        self.register_raw(RegisteredAgent::new(metadata, boxed_fn))
    }

    /// Get a registered agent by kind
    pub fn get(&self, kind: &str) -> Option<Arc<RegisteredAgent>> {
        self.agents.read().get(kind).cloned()
    }

    /// Check if an agent kind is registered
    pub fn has(&self, kind: &str) -> bool {
        self.agents.read().contains_key(kind)
    }

    /// Get all registered agent kinds
    pub fn get_registered_kinds(&self) -> Vec<String> {
        self.agents.read().keys().cloned().collect()
    }

    /// Check if there are any registered agents
    pub fn has_registrations(&self) -> bool {
        !self.agents.read().is_empty()
    }

    /// Get all agent metadata
    pub fn get_all_metadata(&self) -> Vec<AgentMetadata> {
        self.agents
            .read()
            .values()
            .map(|a| a.metadata.clone())
            .collect()
    }

    /// Get the number of registered agents
    pub fn len(&self) -> usize {
        self.agents.read().len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.agents.read().is_empty()
    }
}

impl std::fmt::Debug for AgentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kinds: Vec<_> = self.get_registered_kinds();
        f.debug_struct("AgentRegistry")
            .field("agents", &kinds)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_agent_metadata() {
        let metadata = AgentMetadata {
            kind: "code-agent".to_string(),
            name: "Code Agent".to_string(),
            description: Some("An agent for code assistance".to_string()),
            version: Some(SemanticVersion::new(1, 0, 0)),
            tags: vec!["code".to_string(), "assistant".to_string()],
            timeout_seconds: Some(600),
            cancellable: true,
            input_schema: Some(json!({"type": "object"})),
            output_schema: None,
        };

        assert_eq!(metadata.kind, "code-agent");
        assert_eq!(metadata.name, "Code Agent");
        assert!(metadata.description.is_some());
        assert!(metadata.version.is_some());
        assert_eq!(metadata.tags.len(), 2);
        assert_eq!(metadata.timeout_seconds, Some(600));
        assert!(metadata.cancellable);
        assert!(metadata.input_schema.is_some());
    }

    #[test]
    fn test_agent_registry_new() {
        let registry = AgentRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(!registry.has_registrations());
    }

    #[test]
    fn test_agent_registry_register_simple() {
        let registry = AgentRegistry::new();

        let result = registry.register_simple("test-agent", |_ctx, input| async move {
            Ok(json!({"received": input}))
        });

        assert!(result.is_ok());
        assert!(registry.has("test-agent"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_agent_registry_duplicate_registration() {
        let registry = AgentRegistry::new();

        registry
            .register_simple("test-agent", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let result =
            registry.register_simple("test-agent", |_ctx, _input| async move { Ok(json!({})) });

        assert!(result.is_err());
        match result {
            Err(FlovynError::InvalidConfiguration(msg)) => {
                assert!(msg.contains("already registered"));
            }
            _ => panic!("Expected InvalidConfiguration error"),
        }
    }

    #[test]
    fn test_agent_registry_get() {
        let registry = AgentRegistry::new();

        registry
            .register_simple("my-agent", |_ctx, _input| async move {
                Ok(json!({"status": "ok"}))
            })
            .unwrap();

        let agent = registry.get("my-agent");
        assert!(agent.is_some());
        assert_eq!(agent.unwrap().metadata.kind, "my-agent");

        let missing = registry.get("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_agent_registry_has() {
        let registry = AgentRegistry::new();

        registry
            .register_simple("agent-a", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        assert!(registry.has("agent-a"));
        assert!(!registry.has("agent-b"));
    }

    #[test]
    fn test_agent_registry_get_registered_kinds() {
        let registry = AgentRegistry::new();

        registry
            .register_simple("agent-1", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("agent-2", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("agent-3", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let kinds = registry.get_registered_kinds();
        assert_eq!(kinds.len(), 3);
        assert!(kinds.contains(&"agent-1".to_string()));
        assert!(kinds.contains(&"agent-2".to_string()));
        assert!(kinds.contains(&"agent-3".to_string()));
    }

    #[test]
    fn test_agent_registry_get_all_metadata() {
        let registry = AgentRegistry::new();

        registry
            .register_simple("agent-a", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("agent-b", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let metadata = registry.get_all_metadata();
        assert_eq!(metadata.len(), 2);

        let kinds: Vec<_> = metadata.iter().map(|m| m.kind.clone()).collect();
        assert!(kinds.contains(&"agent-a".to_string()));
        assert!(kinds.contains(&"agent-b".to_string()));
    }

    #[test]
    fn test_agent_registry_register_with_metadata() {
        let registry = AgentRegistry::new();

        let metadata = AgentMetadata {
            kind: "complex-agent".to_string(),
            name: "Complex Agent".to_string(),
            description: Some("A complex agent".to_string()),
            version: Some(SemanticVersion::new(2, 1, 0)),
            tags: vec!["complex".to_string()],
            timeout_seconds: Some(1200),
            cancellable: false,
            input_schema: None,
            output_schema: None,
        };

        let execute_fn: BoxedAgentFn =
            Box::new(|_ctx, input| Box::pin(async move { Ok(json!({"processed": input})) }));

        let result = registry.register_raw(RegisteredAgent::new(metadata, execute_fn));
        assert!(result.is_ok());

        let agent = registry.get("complex-agent").unwrap();
        assert_eq!(agent.metadata.name, "Complex Agent");
        assert_eq!(
            agent.metadata.description,
            Some("A complex agent".to_string())
        );
        assert!(!agent.metadata.cancellable);
        assert_eq!(agent.metadata.timeout_seconds, Some(1200));
    }

    #[test]
    fn test_agent_registry_debug() {
        let registry = AgentRegistry::new();

        registry
            .register_simple("agent-1", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        registry
            .register_simple("agent-2", |_ctx, _input| async move { Ok(json!({})) })
            .unwrap();

        let debug_str = format!("{:?}", registry);
        assert!(debug_str.contains("AgentRegistry"));
    }

    #[test]
    fn test_registered_agent_debug() {
        let metadata = AgentMetadata {
            kind: "test".to_string(),
            name: "Test".to_string(),
            description: None,
            version: None,
            tags: vec![],
            timeout_seconds: None,
            cancellable: true,
            input_schema: None,
            output_schema: None,
        };

        let execute_fn: BoxedAgentFn = Box::new(|_ctx, _input| Box::pin(async { Ok(json!({})) }));

        let registered = RegisteredAgent::new(metadata, execute_fn);
        let debug_str = format!("{:?}", registered);

        assert!(debug_str.contains("RegisteredAgent"));
        assert!(debug_str.contains("test"));
    }
}
