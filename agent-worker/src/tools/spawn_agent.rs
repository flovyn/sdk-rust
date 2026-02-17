//! spawn-agent tool — creates a child agent and returns immediately with a handle.

use async_trait::async_trait;
use flovyn_worker_sdk::agent::{AgentContext, AgentMode, Budget, SpawnOptions};
use flovyn_worker_sdk::error::Result;
use serde_json::{json, Value};

use super::AgentTool;

pub struct SpawnAgentTool;

#[async_trait]
impl AgentTool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn-agent"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "spawn-agent",
            "description": "Spawn a child agent to work on a subtask asynchronously. Returns a handle immediately — the child runs independently. Use 'children' to monitor progress, 'signal-child' to send guidance or cancel. You can either pick a predefined agent by kind, or define a custom agent with a system_prompt and tools.",
            "parameters": {
                "type": "object",
                "properties": {
                    "agent_kind": {
                        "type": "string",
                        "description": "Predefined agent type from the registry. Omit this to define a custom ad-hoc agent instead."
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "System prompt for an ad-hoc agent. Used when agent_kind is omitted."
                    },
                    "tools": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Tools available to an ad-hoc agent. Used when agent_kind is omitted."
                    },
                    "model": {
                        "type": "string",
                        "description": "Override the model (e.g. 'claude-sonnet-4'). Optional."
                    },
                    "mode": {
                        "type": "string",
                        "description": "Where to run: 'local' for file system access, 'remote' for cloud. Usually auto-resolved.",
                        "enum": ["local", "remote"]
                    },
                    "task": {
                        "type": "string",
                        "description": "Detailed task description for the child agent"
                    },
                    "context": {
                        "type": "string",
                        "description": "Relevant context from the current conversation to pass to the child"
                    },
                    "max_budget_tokens": {
                        "type": "integer",
                        "description": "Maximum token budget for the child agent"
                    },
                    "queue": {
                        "type": "string",
                        "description": "Advanced: override the target queue. Usually not needed."
                    }
                },
                "required": ["task"]
            }
        })
    }

    async fn execute(&self, ctx: &dyn AgentContext, args: Value) -> Result<Value> {
        let task = args["task"].as_str().unwrap_or("").to_string();
        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Build agent kind and mode
        let (agent_kind, mode) = if let Some(kind) = args.get("agent_kind").and_then(|v| v.as_str())
        {
            let mode_str = args
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("remote");
            let mode = match mode_str {
                "local" => AgentMode::Local(kind.to_string()),
                _ => AgentMode::Remote(kind.to_string()),
            };
            (kind.to_string(), mode)
        } else {
            // Ad-hoc agent
            let mode_str = args.get("mode").and_then(|v| v.as_str()).unwrap_or("local");
            let mode = match mode_str {
                "remote" => AgentMode::Remote("custom".to_string()),
                _ => AgentMode::Local("custom".to_string()),
            };
            ("custom".to_string(), mode)
        };

        // Build input
        let mut input = json!({"task": task});
        if let Some(ctx_str) = context {
            input["context"] = json!(ctx_str);
        }

        // Build spawn options
        let queue = args.get("queue").and_then(|v| v.as_str()).map(String::from);
        let budget = args
            .get("max_budget_tokens")
            .and_then(|v| v.as_u64())
            .map(|t| Budget {
                max_tokens: Some(t),
                max_cost_usd: None,
                max_tokens_per_call: None,
            });

        let options = SpawnOptions {
            queue,
            budget,
            ..Default::default()
        };

        let handle = ctx.spawn_agent(mode.clone(), input, options).await?;

        let mode_str = match &mode {
            AgentMode::Local(_) => "local",
            AgentMode::Remote(_) => "remote",
            AgentMode::External(_) => "external",
        };

        Ok(json!({
            "child_id": handle.child_id.to_string(),
            "agent_kind": agent_kind,
            "mode": mode_str,
            "status": "spawned"
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_has_required_fields() {
        let tool = SpawnAgentTool;
        let schema = tool.schema();
        assert_eq!(schema["name"], "spawn-agent");
        assert!(schema["parameters"]["properties"]["task"].is_object());
        assert!(schema["parameters"]["properties"]["agent_kind"].is_object());
        let required = schema["parameters"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("task")));
    }
}
