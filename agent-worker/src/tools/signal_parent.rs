//! signal-parent tool — send a signal to the parent agent (child-only).
//!
//! This tool is conditionally registered — only available when the agent has a parent.

use async_trait::async_trait;
use flovyn_worker_sdk::agent::AgentContext;
use flovyn_worker_sdk::error::{FlovynError, Result};
use serde_json::{json, Value};

use super::AgentTool;

pub struct SignalParentTool;

#[async_trait]
impl AgentTool for SignalParentTool {
    fn name(&self) -> &str {
        "signal-parent"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "signal-parent",
            "description": "Send a signal to your parent agent. Use this to report progress, request help when stuck, or share intermediate results. Your parent will see this when it checks on you.",
            "parameters": {
                "type": "object",
                "properties": {
                    "signal_name": {
                        "type": "string",
                        "description": "Signal type",
                        "enum": ["progress", "needs-help", "status", "result"]
                    },
                    "payload": {
                        "type": "string",
                        "description": "The message content. For 'needs-help', describe what you need. For 'progress', describe what you've done."
                    }
                },
                "required": ["signal_name", "payload"]
            }
        })
    }

    async fn execute(&self, ctx: &dyn AgentContext, args: Value) -> Result<Value> {
        let signal_name = args["signal_name"]
            .as_str()
            .ok_or_else(|| FlovynError::Other("signal_name is required".into()))?;

        let payload = args
            .get("payload")
            .cloned()
            .unwrap_or(Value::Null);

        ctx.signal_parent(signal_name, payload).await?;

        Ok(json!({"status": "sent", "signal": signal_name}))
    }
}

/// System prompt section for child agents with parent communication instructions.
pub const CHILD_SYSTEM_PROMPT: &str = r#"## Parent Communication

You were spawned by a parent agent to work on a subtask. You can communicate
with your parent using the signal-parent tool:

- Use "progress" to report what you've completed and what's next.
- Use "needs-help" when you're stuck and need guidance. Your parent will
  respond via a signal you'll receive as a user message.
- Use "result" to share intermediate results before completing.

Signal your parent proactively — don't wait until you're completely done.
Your parent may be coordinating multiple agents and needs visibility."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_has_required_fields() {
        let tool = SignalParentTool;
        let schema = tool.schema();
        assert_eq!(schema["name"], "signal-parent");
        let required = schema["parameters"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("signal_name")));
        assert!(required.contains(&json!("payload")));
    }

    #[test]
    fn test_child_system_prompt_not_empty() {
        assert!(!CHILD_SYSTEM_PROMPT.is_empty());
        assert!(CHILD_SYSTEM_PROMPT.contains("signal-parent"));
    }
}
