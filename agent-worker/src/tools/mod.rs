//! LLM sub-agent tools for hierarchical agent orchestration.
//!
//! These tools expose hierarchical agent operations to LLMs via JSON schemas.
//! Each tool maps LLM tool call arguments to `AgentContext` methods.
//!
//! Tools:
//! - `spawn-agent` — Create a child agent (non-blocking)
//! - `children` — Check/wait on child agents
//! - `signal-child` — Send signal to a child agent
//! - `signal-parent` — Send signal to parent (child-only)

pub mod children;
pub mod signal_child;
pub mod signal_parent;
pub mod spawn_agent;

use async_trait::async_trait;
use flovyn_worker_sdk::agent::AgentContext;
use flovyn_worker_sdk::error::Result;
use serde_json::Value;

/// Definition of an LLM tool with schema and execution logic.
#[async_trait]
pub trait AgentTool: Send + Sync {
    /// Tool name as the LLM sees it
    fn name(&self) -> &str;

    /// JSON schema for the tool (name, description, parameters)
    fn schema(&self) -> Value;

    /// Execute the tool with the given arguments
    async fn execute(&self, ctx: &dyn AgentContext, args: Value) -> Result<Value>;
}

/// Collect all sub-agent tools.
///
/// If `has_parent` is true, includes the `signal-parent` tool (child-only).
pub fn sub_agent_tools(has_parent: bool) -> Vec<Box<dyn AgentTool>> {
    let mut tools: Vec<Box<dyn AgentTool>> = vec![
        Box::new(spawn_agent::SpawnAgentTool),
        Box::new(children::ChildrenTool),
        Box::new(signal_child::SignalChildTool),
    ];

    if has_parent {
        tools.push(Box::new(signal_parent::SignalParentTool));
    }

    tools
}
