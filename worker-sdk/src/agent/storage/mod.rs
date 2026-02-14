//! Storage abstraction for agent execution
//!
//! This module defines the `AgentStorage` trait and supporting types for
//! pluggable storage backends. The storage abstraction enables:
//!
//! - Remote storage via gRPC (production)
//! - In-memory storage (testing)
//! - Local SQLite storage (future: local mode)
//!
//! ## Design
//!
//! Agents batch commands (entries, tasks, signals) and commit them atomically
//! at suspension points. This reduces RPC calls and ensures consistent state
//! transitions.
//!
//! ## Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::storage::*;
//!
//! // Commit a batch of commands atomically
//! let batch = CommandBatch {
//!     segment: 1,
//!     sequence: 5,
//!     commands: vec![
//!         AgentCommand::AppendEntry {
//!             entry_id: Uuid::new_v4(),
//!             parent_id: None,
//!             role: "user".to_string(),
//!             content: json!({"text": "Hello"}),
//!         },
//!     ],
//!     checkpoint: Some(CheckpointData {
//!         state: json!({"turn": 1}),
//!         leaf_entry_id: None,
//!         token_usage: None,
//!     }),
//! };
//!
//! storage.commit_batch(agent_id, batch).await?;
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::FlovynError;

/// Result type for storage operations
pub type StorageResult<T> = Result<T, FlovynError>;

/// A batch of commands to commit atomically.
///
/// Commands are collected during agent execution and committed together
/// at suspension points (checkpoint, wait for signal, wait for tasks).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBatch {
    /// Segment number (increments on each recovery)
    pub segment: u64,
    /// Sequence number within the segment
    pub sequence: u64,
    /// Commands to execute
    pub commands: Vec<AgentCommand>,
    /// Optional checkpoint to persist with this batch
    pub checkpoint: Option<CheckpointData>,
}

/// Commands that can be batched for atomic commit.
///
/// These represent operations that modify agent state:
/// - AppendEntry: Add a conversation entry
/// - ScheduleTask: Schedule a task for execution
/// - WaitForSignal: Register interest in a signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentCommand {
    /// Append an entry to the conversation tree
    AppendEntry {
        /// Unique ID for this entry
        entry_id: Uuid,
        /// Parent entry ID (None for root)
        parent_id: Option<Uuid>,
        /// Entry role (system, user, assistant, tool_result)
        role: String,
        /// Entry content as JSON
        content: Value,
    },
    /// Schedule a task for execution
    ScheduleTask {
        /// Task execution ID (derived from idempotency_key)
        task_id: Uuid,
        /// Task kind (must match a registered task definition)
        kind: String,
        /// Task input as JSON
        input: Value,
        /// Task options
        options: TaskOptions,
        /// Content-based idempotency key for deduplication.
        /// Key = hash(agent_id + kind + canonical_json(input)).
        /// Same task + same input = same key = server returns existing task.
        idempotency_key: String,
    },
    /// Register interest in a signal
    WaitForSignal {
        /// Signal name to wait for
        signal_name: String,
    },
}

/// Checkpoint data to persist with a batch.
///
/// Checkpoints enable lightweight recovery - on resume, the agent
/// restores its state from the checkpoint rather than replaying events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointData {
    /// Application-specific state as JSON
    pub state: Value,
    /// ID of the leaf entry in the conversation tree
    pub leaf_entry_id: Option<Uuid>,
    /// Token usage for cost tracking
    pub token_usage: Option<TokenUsage>,
}

/// Task scheduling options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskOptions {
    /// Queue to schedule the task on (None = default queue)
    pub queue: Option<String>,
    /// Maximum retry attempts
    pub max_retries: Option<i32>,
    /// Task timeout in milliseconds
    pub timeout_ms: Option<i64>,
}

/// Token usage for cost tracking and observability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Number of input tokens consumed
    pub input_tokens: i64,
    /// Number of output tokens generated
    pub output_tokens: i64,
}

/// Segment state loaded on recovery.
///
/// Contains the checkpoint and any pending work from the segment.
#[derive(Debug, Clone)]
pub struct SegmentState {
    /// Segment number
    pub segment: u64,
    /// Checkpoint from this segment (if any)
    pub checkpoint: Option<CheckpointData>,
    /// Tasks that were scheduled but not yet completed
    pub pending_tasks: Vec<PendingTask>,
    /// Signals that the agent is waiting for
    pub pending_signals: Vec<String>,
}

/// A task that was scheduled but not yet completed.
#[derive(Debug, Clone)]
pub struct PendingTask {
    /// Task execution ID
    pub task_id: Uuid,
    /// Task kind
    pub kind: String,
    /// Current task status
    pub status: TaskStatus,
    /// Task result (if completed successfully)
    pub result: Option<TaskResult>,
}

/// Task execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task is queued, waiting for a worker
    Pending,
    /// Task is being executed by a worker
    Running,
    /// Task completed successfully
    Completed,
    /// Task failed after all retries
    Failed,
    /// Task was cancelled
    Cancelled,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "PENDING"),
            TaskStatus::Running => write!(f, "RUNNING"),
            TaskStatus::Completed => write!(f, "COMPLETED"),
            TaskStatus::Failed => write!(f, "FAILED"),
            TaskStatus::Cancelled => write!(f, "CANCELLED"),
        }
    }
}

/// Result of a completed task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Task execution ID
    pub task_id: Uuid,
    /// Final task status
    pub status: TaskStatus,
    /// Task output (if completed successfully)
    pub output: Option<Value>,
    /// Error message (if failed)
    pub error: Option<String>,
}

/// Storage backend for agent execution.
///
/// Implementations provide persistence for agent state, including:
/// - Command batches (entries, tasks, signals)
/// - Checkpoints for recovery
/// - Task results
/// - Signals
///
/// # Implementations
///
/// - `RemoteStorage`: gRPC-based storage (production)
/// - `InMemoryStorage`: In-memory storage (testing)
/// - `SqliteStorage`: Local SQLite storage (future: local mode)
#[async_trait]
pub trait AgentStorage: Send + Sync {
    /// Commit a batch of commands atomically.
    ///
    /// All commands in the batch are applied together. If any command
    /// fails, the entire batch should be rolled back.
    async fn commit_batch(&self, agent_id: Uuid, batch: CommandBatch) -> StorageResult<()>;

    /// Load segment state for recovery.
    ///
    /// Returns the checkpoint and pending work for the specified segment.
    async fn load_segment(&self, agent_id: Uuid, segment: u64) -> StorageResult<SegmentState>;

    /// Get the latest segment number for an agent.
    ///
    /// Returns 0 if no segments exist.
    async fn get_latest_segment(&self, agent_id: Uuid) -> StorageResult<u64>;

    /// Get the result of a completed task.
    ///
    /// Returns `None` if the task is still pending or doesn't exist.
    async fn get_task_result(&self, task_id: Uuid) -> StorageResult<Option<TaskResult>>;

    /// Get results for multiple tasks in one call.
    ///
    /// More efficient than calling `get_task_result` multiple times.
    /// Returns only tasks that have results (completed, failed, or cancelled).
    async fn get_task_results(&self, task_ids: &[Uuid]) -> StorageResult<Vec<TaskResult>>;

    /// Store a signal for an agent.
    ///
    /// Signals are stored in a FIFO queue per (agent_id, signal_name).
    async fn store_signal(
        &self,
        agent_id: Uuid,
        signal_name: &str,
        payload: Value,
    ) -> StorageResult<()>;

    /// Get and remove the next pending signal.
    ///
    /// Returns `None` if no signal is pending.
    async fn pop_signal(&self, agent_id: Uuid, signal_name: &str) -> StorageResult<Option<Value>>;

    /// Check if a signal is pending without consuming it.
    async fn has_signal(&self, agent_id: Uuid, signal_name: &str) -> StorageResult<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_command_batch_serialization() {
        let batch = CommandBatch {
            segment: 1,
            sequence: 5,
            commands: vec![
                AgentCommand::AppendEntry {
                    entry_id: Uuid::nil(),
                    parent_id: None,
                    role: "user".to_string(),
                    content: json!({"text": "Hello"}),
                },
                AgentCommand::ScheduleTask {
                    task_id: Uuid::nil(),
                    kind: "analyze".to_string(),
                    input: json!({"data": [1, 2, 3]}),
                    options: TaskOptions::default(),
                    idempotency_key: "task:test-key".to_string(),
                },
                AgentCommand::WaitForSignal {
                    signal_name: "userInput".to_string(),
                },
            ],
            checkpoint: Some(CheckpointData {
                state: json!({"turn": 1}),
                leaf_entry_id: Some(Uuid::nil()),
                token_usage: Some(TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                }),
            }),
        };

        // Should serialize to JSON without error
        let json = serde_json::to_string(&batch).unwrap();
        assert!(json.contains("AppendEntry"));
        assert!(json.contains("ScheduleTask"));
        assert!(json.contains("WaitForSignal"));

        // Should deserialize back
        let deserialized: CommandBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.segment, 1);
        assert_eq!(deserialized.sequence, 5);
        assert_eq!(deserialized.commands.len(), 3);
    }

    #[test]
    fn test_task_status_display() {
        assert_eq!(TaskStatus::Pending.to_string(), "PENDING");
        assert_eq!(TaskStatus::Running.to_string(), "RUNNING");
        assert_eq!(TaskStatus::Completed.to_string(), "COMPLETED");
        assert_eq!(TaskStatus::Failed.to_string(), "FAILED");
        assert_eq!(TaskStatus::Cancelled.to_string(), "CANCELLED");
    }

    #[test]
    fn test_task_options_default() {
        let options = TaskOptions::default();
        assert!(options.queue.is_none());
        assert!(options.max_retries.is_none());
        assert!(options.timeout_ms.is_none());
    }

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
    }

    #[test]
    fn test_checkpoint_data_serialization() {
        let checkpoint = CheckpointData {
            state: json!({"conversation_id": "abc123", "turn": 5}),
            leaf_entry_id: Some(Uuid::nil()),
            token_usage: Some(TokenUsage {
                input_tokens: 1000,
                output_tokens: 500,
            }),
        };

        let json = serde_json::to_string(&checkpoint).unwrap();
        let deserialized: CheckpointData = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.state,
            json!({"conversation_id": "abc123", "turn": 5})
        );
        assert_eq!(deserialized.leaf_entry_id, Some(Uuid::nil()));
        assert!(deserialized.token_usage.is_some());
    }
}

mod memory;
mod remote;

pub use memory::InMemoryStorage;
pub use remote::RemoteStorage;
