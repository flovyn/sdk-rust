//! Types for hierarchical agent relationships
//!
//! Defines child handles, events, spawn options, and related types
//! for parent-child agent orchestration.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// Handle to a spawned child agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildHandle {
    /// Unique ID of the child agent execution
    pub child_id: Uuid,
    /// Mode the child was spawned in
    pub mode: AgentMode,
}

/// How a child agent is executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMode {
    /// Run on a remote worker via queue routing
    Remote(String),
    /// Run in the same process as the parent
    Local(String),
    /// Delegated to an external agent system
    External(ExternalAgent),
}

/// External agent system integrations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExternalAgent {
    /// Anthropic's Pi agent
    Pi,
    /// Claude Code CLI agent
    ClaudeCode,
    /// Custom external agent protocol
    Custom(String),
}

/// Event from a child agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChildEvent {
    /// Child sent a signal to parent
    Signal {
        child_id: Uuid,
        signal_name: String,
        payload: serde_json::Value,
    },
    /// Child completed successfully
    Completed {
        child_id: Uuid,
        output: serde_json::Value,
    },
    /// Child failed with an error
    Failed {
        child_id: Uuid,
        error: String,
    },
    /// Child exceeded its budget
    BudgetExceeded {
        child_id: Uuid,
    },
    /// Child timed out
    TimedOut {
        child_id: Uuid,
    },
}

/// Info about a child event returned from polling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildEventInfo {
    pub child_id: Uuid,
    pub event: ChildEvent,
}

/// Options for spawning a child agent
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpawnOptions {
    /// Explicit target queue override. When set, bypasses automatic queue resolution.
    pub queue: Option<String>,
    /// Persistence mode for the child agent
    pub persistence: Option<Persistence>,
    /// Budget limits for the child agent
    pub budget: Option<Budget>,
    /// Timeout for the child agent
    pub timeout: Option<Duration>,
}

/// Budget limits for an agent execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    /// Maximum total tokens across all LLM calls
    pub max_tokens: Option<u64>,
    /// Maximum cost in USD
    pub max_cost_usd: Option<f64>,
    /// Maximum tokens per individual LLM call
    pub max_tokens_per_call: Option<u64>,
}

/// How much agent state to persist
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Persistence {
    /// Full checkpoint-based persistence (default)
    Full,
    /// Only persist minimal state (entries, no checkpoints)
    Minimal,
    /// No persistence — ephemeral execution
    Ephemeral,
}

/// How to cancel a child agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CancellationMode {
    /// Send a cancellation signal and wait for graceful shutdown
    Graceful {
        /// How long to wait before forcing cancellation
        timeout: Duration,
    },
    /// Immediately terminate the child
    Hard,
}

/// Options for handing off execution to another agent
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandoffOptions {
    /// How the handoff completes
    pub completion: Option<HandoffCompletion>,
    /// Spawn options for the target agent
    pub spawn: Option<SpawnOptions>,
}

/// How a handoff completes relative to the parent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HandoffCompletion {
    /// Parent waits for child to complete and returns child's output
    WaitForChild,
    /// Parent completes immediately after spawning child
    Immediate,
}

impl Default for HandoffCompletion {
    fn default() -> Self {
        Self::WaitForChild
    }
}
