//! children tool — check on or wait for child agents.

use async_trait::async_trait;
use flovyn_worker_sdk::agent::AgentContext;
use flovyn_worker_sdk::error::{FlovynError, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use super::AgentTool;

pub struct ChildrenTool;

#[async_trait]
impl AgentTool for ChildrenTool {
    fn name(&self) -> &str {
        "children"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "children",
            "description": "Check on or wait for child agents. With wait='none', returns immediately with current status. With wait='all', blocks until all children finish. With wait='any', returns when the first child has an event.",
            "parameters": {
                "type": "object",
                "properties": {
                    "child_ids": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Child agent handles from spawn-agent."
                    },
                    "wait": {
                        "type": "string",
                        "enum": ["none", "all", "any"],
                        "default": "none",
                        "description": "Blocking mode. 'none' = poll. 'all' = wait for all. 'any' = wait for first event."
                    }
                },
                "required": ["child_ids"]
            }
        })
    }

    async fn execute(&self, ctx: &dyn AgentContext, args: Value) -> Result<Value> {
        let child_ids = parse_child_ids(&args)?;
        let wait = args.get("wait").and_then(|w| w.as_str()).unwrap_or("none");

        // Resolve handles
        let mut handles = Vec::with_capacity(child_ids.len());
        for id in &child_ids {
            handles.push(ctx.get_child_handle(*id).await?);
        }

        match wait {
            "none" => {
                let events = ctx.poll_child_events(&handles).await?;
                let children: Vec<Value> = events
                    .iter()
                    .map(|info| {
                        json!({
                            "child_id": info.child_id.to_string(),
                            "event": format!("{:?}", info.event),
                        })
                    })
                    .collect();
                Ok(json!({"children": children}))
            }
            "all" => {
                let results = ctx.join_children(&handles).await?;
                let result_values: Vec<Value> = results
                    .iter()
                    .map(|event| match event {
                        flovyn_worker_sdk::agent::ChildEvent::Completed { child_id, output } => {
                            json!({"child_id": child_id.to_string(), "status": "completed", "output": output})
                        }
                        flovyn_worker_sdk::agent::ChildEvent::Failed { child_id, error } => {
                            json!({"child_id": child_id.to_string(), "status": "failed", "error": error})
                        }
                        flovyn_worker_sdk::agent::ChildEvent::Signal {
                            child_id,
                            signal_name,
                            payload,
                        } => {
                            json!({"child_id": child_id.to_string(), "status": "signal", "signal_name": signal_name, "payload": payload})
                        }
                        other => {
                            json!({"event": format!("{:?}", other)})
                        }
                    })
                    .collect();
                Ok(json!({"results": result_values}))
            }
            "any" => {
                let event = ctx.select_child(&handles).await?;
                let result = match &event {
                    flovyn_worker_sdk::agent::ChildEvent::Completed { child_id, output } => {
                        json!({"child_id": child_id.to_string(), "event": "completed", "output": output})
                    }
                    flovyn_worker_sdk::agent::ChildEvent::Failed { child_id, error } => {
                        json!({"child_id": child_id.to_string(), "event": "failed", "error": error})
                    }
                    flovyn_worker_sdk::agent::ChildEvent::Signal {
                        child_id,
                        signal_name,
                        payload,
                    } => {
                        json!({"child_id": child_id.to_string(), "event": "signal", "signal_name": signal_name, "payload": payload})
                    }
                    other => json!({"event": format!("{:?}", other)}),
                };
                Ok(result)
            }
            other => Err(FlovynError::Other(format!(
                "Invalid wait mode: '{}'. Must be 'none', 'all', or 'any'",
                other
            ))),
        }
    }
}

#[allow(clippy::result_large_err)]
fn parse_child_ids(args: &Value) -> Result<Vec<Uuid>> {
    let ids = args["child_ids"]
        .as_array()
        .ok_or_else(|| FlovynError::Other("child_ids must be an array".into()))?;

    ids.iter()
        .map(|id| {
            let s = id
                .as_str()
                .ok_or_else(|| FlovynError::Other("child_id must be a string".into()))?;
            Uuid::parse_str(s)
                .map_err(|e| FlovynError::Other(format!("Invalid child_id '{}': {}", s, e)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_has_required_fields() {
        let tool = ChildrenTool;
        let schema = tool.schema();
        assert_eq!(schema["name"], "children");
        assert!(schema["parameters"]["properties"]["child_ids"].is_object());
        assert!(schema["parameters"]["properties"]["wait"].is_object());
    }

    #[test]
    fn test_parse_child_ids_valid() {
        let id = Uuid::new_v4();
        let args = json!({"child_ids": [id.to_string()]});
        let ids = parse_child_ids(&args).unwrap();
        assert_eq!(ids, vec![id]);
    }

    #[test]
    fn test_parse_child_ids_invalid() {
        let args = json!({"child_ids": ["not-a-uuid"]});
        assert!(parse_child_ids(&args).is_err());
    }
}
