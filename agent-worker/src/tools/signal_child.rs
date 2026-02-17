//! signal-child tool — send a signal to a running child agent.

use async_trait::async_trait;
use flovyn_worker_sdk::agent::{AgentContext, CancellationMode};
use flovyn_worker_sdk::error::{FlovynError, Result};
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

use super::AgentTool;

pub struct SignalChildTool;

#[async_trait]
impl AgentTool for SignalChildTool {
    fn name(&self) -> &str {
        "signal-child"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "signal-child",
            "description": "Send a signal to a running child agent. Use to answer help requests, provide guidance, give new instructions, or cancel the child (signal_name='cancel').",
            "parameters": {
                "type": "object",
                "properties": {
                    "child_id": {
                        "type": "string",
                        "description": "The child agent handle"
                    },
                    "signal_name": {
                        "type": "string",
                        "description": "Signal type",
                        "enum": ["guidance", "answer", "instruction", "cancel"]
                    },
                    "payload": {
                        "type": "string",
                        "description": "The message content to send to the child"
                    }
                },
                "required": ["child_id", "signal_name", "payload"]
            }
        })
    }

    async fn execute(&self, ctx: &dyn AgentContext, args: Value) -> Result<Value> {
        let child_id_str = args["child_id"]
            .as_str()
            .ok_or_else(|| FlovynError::Other("child_id is required".into()))?;
        let child_id = Uuid::parse_str(child_id_str)
            .map_err(|e| FlovynError::Other(format!("Invalid child_id: {}", e)))?;

        let signal_name = args["signal_name"]
            .as_str()
            .ok_or_else(|| FlovynError::Other("signal_name is required".into()))?;

        let payload = args.get("payload").cloned().unwrap_or(Value::Null);

        let handle = ctx.get_child_handle(child_id).await?;

        if signal_name == "cancel" {
            ctx.cancel_child(
                &handle,
                CancellationMode::Graceful {
                    timeout: Duration::from_secs(30),
                },
            )
            .await?;
            Ok(json!({"status": "cancelling", "child_id": child_id_str}))
        } else {
            ctx.signal_child(&handle, signal_name, payload).await?;
            Ok(json!({"status": "sent", "child_id": child_id_str, "signal": signal_name}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_has_required_fields() {
        let tool = SignalChildTool;
        let schema = tool.schema();
        assert_eq!(schema["name"], "signal-child");
        let required = schema["parameters"]["required"].as_array().unwrap();
        assert!(required.contains(&json!("child_id")));
        assert!(required.contains(&json!("signal_name")));
        assert!(required.contains(&json!("payload")));
    }
}
