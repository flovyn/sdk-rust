//! Signal Workflow (External Events)
//!
//! Demonstrates the use of signals for handling external events.
//! Signals are the recommended pattern for:
//! - Receiving external events (webhooks, user actions)
//! - Conversational workflows (chatbots, interactive sessions)
//! - Event-driven processing
//!
//! Signals differ from promises in that:
//! - Signals are consumed in FIFO order (queue semantics)
//! - Multiple signals can be sent before the workflow consumes them
//! - SignalWithStart atomically creates a workflow and sends the first signal

use async_trait::async_trait;
use flovyn_worker_sdk::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::info;

// ============================================================================
// Simple Signal Workflow
// ============================================================================

/// Input for simple signal workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WaitForSignalInput {
    /// Description of what signal we're waiting for
    pub description: String,
}

/// Output from simple signal workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WaitForSignalOutput {
    /// The name of the signal that was received
    pub signal_name: String,
    /// The value that was sent with the signal
    pub signal_value: serde_json::Value,
}

/// A workflow that waits for a single external signal
///
/// This demonstrates the basic signal pattern:
/// 1. Workflow starts and waits for a signal
/// 2. External system sends a signal via SignalWithStart or Signal API
/// 3. Workflow receives and processes the signal
pub struct WaitForSignalWorkflow;

#[async_trait]
impl WorkflowDefinition for WaitForSignalWorkflow {
    type Input = WaitForSignalInput;
    type Output = WaitForSignalOutput;

    fn kind(&self) -> &str {
        "wait-for-signal-workflow"
    }

    fn name(&self) -> &str {
        "Wait for Signal Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates waiting for an external signal")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(3600) // 1 hour timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "signal".to_string(),
            "external-event".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            description = %input.description,
            "Signal workflow started, waiting for signal"
        );

        // Wait for a signal named "signal" (or use any name you want to filter by)
        // Note: Each signal name has its own queue, so signals are filtered by name
        let signal = ctx.wait_for_signal_raw("signal").await?;

        info!(
            signal_name = %signal.name,
            "Received signal"
        );

        Ok(WaitForSignalOutput {
            signal_name: signal.name,
            signal_value: signal.value,
        })
    }
}

// ============================================================================
// Conversational Workflow
// ============================================================================

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Input for conversational workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConversationInput {
    /// Initial system prompt or context
    pub system_prompt: Option<String>,
}

/// Output from conversational workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConversationOutput {
    /// Full conversation history
    pub messages: Vec<ChatMessage>,
    /// Total number of turns
    pub turn_count: u32,
    /// Whether conversation ended normally
    pub completed: bool,
}

/// A conversational workflow that handles multiple signals
///
/// This pattern is useful for:
/// - Chatbots and interactive assistants
/// - Multi-step wizards
/// - Game sessions
///
/// The workflow:
/// 1. Starts and waits for user messages (signals)
/// 2. Processes each message and generates a response
/// 3. Continues until receiving an "end" signal
pub struct ConversationWorkflow;

#[async_trait]
impl WorkflowDefinition for ConversationWorkflow {
    type Input = ConversationInput;
    type Output = ConversationOutput;

    fn kind(&self) -> &str {
        "conversation-workflow"
    }

    fn name(&self) -> &str {
        "Conversational Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates multi-signal conversational pattern")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(86400) // 24 hour session timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "signal".to_string(),
            "conversation".to_string(),
            "chatbot".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let mut messages: Vec<ChatMessage> = Vec::new();
        let mut turn_count = 0u32;

        // Add system prompt if provided
        if let Some(prompt) = &input.system_prompt {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: prompt.clone(),
            });
        }

        info!("Conversation workflow started");

        // Main conversation loop
        // All messages come via the "message" signal queue
        // An "end" message is indicated by the message content
        loop {
            // Check for cancellation
            ctx.check_cancellation().await?;

            info!(turn = turn_count + 1, "Waiting for user message");

            // Wait for a "message" signal
            let signal = ctx.wait_for_signal_raw("message").await?;

            // Extract message content
            let user_message = match &signal.value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(obj) => {
                    // Check for end/goodbye command
                    if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
                        if cmd == "end" || cmd == "goodbye" {
                            info!("Received end command, closing conversation");
                            messages.push(ChatMessage {
                                role: "system".to_string(),
                                content: "Conversation ended by user".to_string(),
                            });
                            break;
                        }
                    }
                    // Extract content from various possible fields
                    obj.get("content")
                        .or_else(|| obj.get("message"))
                        .or_else(|| obj.get("text"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| signal.value.to_string())
                }
                _ => signal.value.to_string(),
            };

            // Check for end keywords in the message itself
            let lower = user_message.to_lowercase();
            if lower == "end" || lower == "goodbye" || lower == "quit" || lower == "exit" {
                info!("Received end message, closing conversation");
                messages.push(ChatMessage {
                    role: "system".to_string(),
                    content: "Conversation ended by user".to_string(),
                });
                break;
            }

            // Add user message to history
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: user_message.clone(),
            });

            info!(message = %user_message, "Received user message");

            // Generate a response (in a real app, this would call an LLM)
            let response = generate_response(&user_message, &messages);
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });

            turn_count += 1;

            info!(
                turn = turn_count,
                response = %response,
                "Generated response"
            );

            // Safety limit to prevent infinite loops
            if turn_count >= 100 {
                info!("Reached maximum turn limit");
                messages.push(ChatMessage {
                    role: "system".to_string(),
                    content: "Maximum conversation length reached".to_string(),
                });
                break;
            }
        }

        Ok(ConversationOutput {
            messages,
            turn_count,
            completed: true,
        })
    }
}

/// Simple response generator (placeholder for LLM integration)
fn generate_response(user_message: &str, _history: &[ChatMessage]) -> String {
    // In a real application, this would call an LLM API
    let lower = user_message.to_lowercase();
    if lower.contains("hello") || lower.contains("hi") {
        "Hello! How can I help you today?".to_string()
    } else if lower.contains("help") {
        "I'm here to help! What would you like to know?".to_string()
    } else if lower.contains("thank") {
        "You're welcome! Is there anything else I can help with?".to_string()
    } else {
        format!("I received your message: \"{}\". How can I assist further?", user_message)
    }
}

// ============================================================================
// Event Collector Workflow
// ============================================================================

/// Input for event collector workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventCollectorInput {
    /// Session identifier
    pub session_id: String,
    /// Maximum events to collect before completing
    pub max_events: u32,
}

/// Collected event
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CollectedEvent {
    pub event_type: String,
    pub data: serde_json::Value,
    pub timestamp: i64,
}

/// Output from event collector workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EventCollectorOutput {
    pub session_id: String,
    pub events: Vec<CollectedEvent>,
    pub total_collected: u32,
}

/// A workflow that collects multiple events before processing
///
/// This pattern is useful for:
/// - Batch event processing
/// - Session analytics
/// - Aggregating related events
pub struct EventCollectorWorkflow;

#[async_trait]
impl WorkflowDefinition for EventCollectorWorkflow {
    type Input = EventCollectorInput;
    type Output = EventCollectorOutput;

    fn kind(&self) -> &str {
        "event-collector-workflow"
    }

    fn name(&self) -> &str {
        "Event Collector Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Collects multiple signals/events before processing")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(3600) // 1 hour collection window
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "signal".to_string(),
            "event-collection".to_string(),
            "batch".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let mut events: Vec<CollectedEvent> = Vec::new();

        info!(
            session_id = %input.session_id,
            max_events = input.max_events,
            "Event collector started"
        );

        // Collect events until we reach the limit or receive a "flush" command
        // All events come via the "event" signal queue
        // The signal value should be: { "type": "click", "data": {...} } or { "command": "flush" }
        while events.len() < input.max_events as usize {
            ctx.check_cancellation().await?;

            let signal = ctx.wait_for_signal_raw("event").await?;

            // Check for flush/complete command in the signal value
            if let Some(cmd) = signal.value.get("command").and_then(|v| v.as_str()) {
                if cmd == "flush" || cmd == "complete" {
                    info!("Received flush command, completing collection");
                    break;
                }
            }

            // Extract event type from the signal value
            let event_type = signal
                .value
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            // Extract event data (either from "data" field or use the whole value)
            let data = signal
                .value
                .get("data")
                .cloned()
                .unwrap_or(signal.value.clone());

            let event = CollectedEvent {
                event_type: event_type.clone(),
                data,
                timestamp: ctx.current_time_millis(),
            };

            events.push(event);

            info!(
                event_type = %event_type,
                collected = events.len(),
                max = input.max_events,
                "Collected event"
            );
        }

        let total = events.len() as u32;

        info!(
            session_id = %input.session_id,
            total_collected = total,
            "Event collection completed"
        );

        Ok(EventCollectorOutput {
            session_id: input.session_id,
            events,
            total_collected: total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_for_signal_workflow_kind() {
        let workflow = WaitForSignalWorkflow;
        assert_eq!(workflow.kind(), "wait-for-signal-workflow");
    }

    #[test]
    fn test_conversation_workflow_kind() {
        let workflow = ConversationWorkflow;
        assert_eq!(workflow.kind(), "conversation-workflow");
    }

    #[test]
    fn test_event_collector_workflow_kind() {
        let workflow = EventCollectorWorkflow;
        assert_eq!(workflow.kind(), "event-collector-workflow");
    }

    #[test]
    fn test_generate_response_hello() {
        let response = generate_response("Hello there!", &[]);
        assert!(response.contains("Hello"));
    }

    #[test]
    fn test_chat_message_serialization() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: "Test message".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("Test message"));
    }
}
