//! External agent protocol trait for delegating to non-Flovyn agent systems.
//!
//! Defines the interface for adapting external agent providers (like Pi, Claude Code,
//! or custom systems) into Flovyn's hierarchical agent model.
//!
//! No concrete implementations are provided — Pi/ClaudeCode adapters are separate
//! features. The `ExternalAgent::Custom(String)` variant provides the extension point.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for creating an external agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// System prompt or instructions for the external agent
    pub system_prompt: Option<String>,
    /// Model override if the external provider supports it
    pub model: Option<String>,
    /// Maximum tokens budget
    pub max_tokens: Option<u64>,
    /// Provider-specific configuration
    pub provider_config: Option<Value>,
}

/// Handle to an active external agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHandle {
    /// Provider-specific session identifier
    pub session_id: String,
    /// Provider name (e.g., "pi", "claude-code", "custom")
    pub provider: String,
}

/// Snapshot of an external agent session's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Current conversation messages
    pub messages: Vec<Message>,
    /// Provider-reported status
    pub status: String,
    /// Provider-specific metadata
    pub metadata: Option<Value>,
}

/// A message in the external agent's conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "user", "assistant", "system"
    pub role: String,
    /// Message content
    pub content: String,
}

/// Events produced by the external agent during execution.
///
/// Maps to Flovyn's internal event model:
/// - `Output` → stored as agent entry
/// - `ToolCall` → recorded as tool_call entry
/// - `Completed` → triggers completion flow
/// - `Failed` → triggers failure flow
/// - `NeedsInput` → triggers signal to parent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    /// Agent produced text output
    Output { content: String },
    /// Agent made a tool call
    ToolCall {
        tool_name: String,
        tool_input: Value,
    },
    /// Agent completed successfully
    Completed { output: Value },
    /// Agent failed with an error
    Failed { error: String },
    /// Agent needs input from the user/parent
    NeedsInput { prompt: String },
}

/// Protocol for interacting with external agent systems.
///
/// Implementors adapt a specific agent provider's API into Flovyn's model.
/// The adapter translates between Flovyn signals/events and the provider's
/// native communication protocol.
///
/// ## Event Mapping
///
/// | Flovyn Concept       | External Agent Equivalent         |
/// |---------------------|-----------------------------------|
/// | `spawn_agent`       | `create_session` + `send_message` |
/// | `signal_child`      | `send_message`                    |
/// | `poll_child_events` | `get_events`                      |
/// | `cancel_child`      | `cancel_session`                  |
/// | Child completion    | `AgentEvent::Completed`           |
/// | Child failure       | `AgentEvent::Failed`              |
/// | Needs help          | `AgentEvent::NeedsInput`          |
#[async_trait]
pub trait ExternalAgentProtocol: Send + Sync {
    /// Create a new session with the external agent provider.
    async fn create_session(
        &self,
        config: SessionConfig,
    ) -> Result<SessionHandle, Box<dyn std::error::Error + Send + Sync>>;

    /// Send a message to the external agent.
    async fn send_message(
        &self,
        session: &SessionHandle,
        message: Message,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Get pending events from the external agent.
    async fn get_events(
        &self,
        session: &SessionHandle,
    ) -> Result<Vec<AgentEvent>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get a snapshot of the session's current state.
    async fn get_snapshot(
        &self,
        session: &SessionHandle,
    ) -> Result<SessionSnapshot, Box<dyn std::error::Error + Send + Sync>>;

    /// Cancel the external agent session.
    async fn cancel_session(
        &self,
        session: &SessionHandle,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
