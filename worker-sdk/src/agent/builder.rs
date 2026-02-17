//! Agent builder for local mode.
//!
//! Provides a fluent API for constructing local agents with pluggable
//! storage, executor, and signal source backends.
//!
//! # Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::builder::AgentBuilder;
//! use flovyn_worker_sdk::agent::storage::{InMemoryStorage, SqliteStorage};
//! use flovyn_worker_sdk::agent::executor::LocalTaskExecutor;
//! use flovyn_worker_sdk::agent::signals::InteractiveSignalSource;
//!
//! // Ephemeral agent (no persistence)
//! let ctx = AgentBuilder::new()
//!     .storage(InMemoryStorage::new())
//!     .build()
//!     .await?;
//!
//! // Persistent local agent
//! let ctx = AgentBuilder::new()
//!     .storage(SqliteStorage::open("./agent.db").await?)
//!     .task_executor(LocalTaskExecutor::new())
//!     .signal_source(InteractiveSignalSource::stdin())
//!     .build()
//!     .await?;
//! ```

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use super::context::LoadedMessage;
use super::context_impl::AgentContextImpl;
use super::executor::{RemoteTaskExecutor, TaskExecutor};
use super::signals::{RemoteSignalSource, SignalSource};
use super::storage::{AgentStorage, InMemoryStorage};
use crate::error::FlovynError;

/// Builder for constructing local agent contexts.
///
/// Supports three composition modes from the design doc:
/// - **Ephemeral**: InMemoryStorage (default) - no persistence
/// - **Local + Persist**: SqliteStorage - durable local execution
/// - **Full Remote**: RemoteStorage - production cloud mode
pub struct AgentBuilder {
    agent_id: Uuid,
    org_id: Uuid,
    input: Value,
    storage: Option<Arc<dyn AgentStorage>>,
    task_executor: Option<Arc<dyn TaskExecutor>>,
    signal_source: Option<Arc<dyn SignalSource>>,
    messages: Vec<LoadedMessage>,
    checkpoint_state: Option<Value>,
    checkpoint_sequence: i32,
    leaf_entry_id: Option<Uuid>,
}

impl AgentBuilder {
    /// Create a new agent builder with defaults.
    ///
    /// Defaults to InMemoryStorage, RemoteTaskExecutor, and RemoteSignalSource.
    pub fn new() -> Self {
        Self {
            agent_id: Uuid::new_v4(),
            org_id: Uuid::nil(),
            input: Value::Null,
            storage: None,
            task_executor: None,
            signal_source: None,
            messages: Vec::new(),
            checkpoint_state: None,
            checkpoint_sequence: -1,
            leaf_entry_id: None,
        }
    }

    /// Set the agent execution ID.
    pub fn agent_id(mut self, id: Uuid) -> Self {
        self.agent_id = id;
        self
    }

    /// Set the organization ID.
    pub fn org_id(mut self, id: Uuid) -> Self {
        self.org_id = id;
        self
    }

    /// Set the agent input.
    pub fn input(mut self, input: Value) -> Self {
        self.input = input;
        self
    }

    /// Set the storage backend.
    pub fn storage(mut self, storage: impl AgentStorage + 'static) -> Self {
        self.storage = Some(Arc::new(storage));
        self
    }

    /// Set the task executor.
    pub fn task_executor(mut self, executor: impl TaskExecutor + 'static) -> Self {
        self.task_executor = Some(Arc::new(executor));
        self
    }

    /// Set the signal source.
    pub fn signal_source(mut self, source: impl SignalSource + 'static) -> Self {
        self.signal_source = Some(Arc::new(source));
        self
    }

    /// Set pre-loaded messages (for resume).
    pub fn messages(mut self, messages: Vec<LoadedMessage>) -> Self {
        self.messages = messages;
        self
    }

    /// Set checkpoint state (for resume).
    pub fn checkpoint(mut self, state: Value, sequence: i32, leaf_entry_id: Option<Uuid>) -> Self {
        self.checkpoint_state = Some(state);
        self.checkpoint_sequence = sequence;
        self.leaf_entry_id = leaf_entry_id;
        self
    }

    /// Build the agent context.
    ///
    /// Creates an `AgentContextImpl` with the configured backends.
    /// A dummy gRPC client is created that will fail on remote operations
    /// (which is expected for local-only agents).
    pub async fn build(self) -> Result<AgentContextImpl, FlovynError> {
        let storage = self
            .storage
            .unwrap_or_else(|| Arc::new(InMemoryStorage::new()));
        let task_executor = self
            .task_executor
            .unwrap_or_else(|| Arc::new(RemoteTaskExecutor::new()));
        let signal_source = self
            .signal_source
            .unwrap_or_else(|| Arc::new(RemoteSignalSource::new()));

        // Create a dummy gRPC client that will fail on remote operations.
        // For local agents, all operations go through the trait objects,
        // so the client is never used in normal operation.
        // We connect to a known-invalid endpoint that will fail on any call.
        let dummy_channel = tonic::transport::Channel::from_static("http://[::1]:1")
            .connect_lazy();
        let dummy_client =
            flovyn_worker_core::client::AgentDispatch::new(dummy_channel, "local-mode");

        Ok(AgentContextImpl::from_loaded_with_backends(
            dummy_client,
            self.agent_id,
            self.org_id,
            self.input,
            self.messages,
            self.checkpoint_state,
            self.checkpoint_sequence,
            self.leaf_entry_id,
            storage,
            task_executor,
            signal_source,
        ))
    }
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::AgentContext;

    #[tokio::test]
    async fn test_agent_builder_ephemeral() {
        // Build with defaults (InMemoryStorage)
        let ctx = AgentBuilder::new()
            .agent_id(Uuid::new_v4())
            .input(serde_json::json!({"prompt": "Hello"}))
            .build()
            .await
            .unwrap();

        // Verify the context was created
        assert_eq!(ctx.input_raw(), &serde_json::json!({"prompt": "Hello"}));
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn test_agent_builder_local_persist() {
        use super::super::storage::SqliteStorage;

        let storage = SqliteStorage::in_memory().await.unwrap();

        let ctx = AgentBuilder::new()
            .agent_id(Uuid::new_v4())
            .input(serde_json::json!({"task": "analyze"}))
            .storage(storage)
            .build()
            .await
            .unwrap();

        assert_eq!(ctx.input_raw(), &serde_json::json!({"task": "analyze"}));
    }
}
