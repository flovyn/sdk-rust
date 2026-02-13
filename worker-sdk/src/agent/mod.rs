//! Agent module for durable agent execution
//!
//! This module provides the core traits and types for defining and executing
//! durable agents with checkpoint-based recovery.
//!
//! ## Overview
//!
//! Agents are long-running, conversation-style AI workloads that:
//! - Store conversation entries in a tree structure
//! - Support lightweight checkpointing for recovery
//! - Can schedule tasks and wait for signals
//! - Stream events to connected clients
//!
//! ## Key Traits
//!
//! - [`AgentDefinition`] - Define a typed agent with input/output types
//! - [`AgentContext`] - Execution context for agent operations
//!
//! ## Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::prelude::*;
//!
//! struct EchoAgent;
//!
//! #[async_trait]
//! impl AgentDefinition for EchoAgent {
//!     type Input = String;
//!     type Output = String;
//!
//!     fn kind(&self) -> &str { "echo-agent" }
//!
//!     async fn execute(&self, ctx: &dyn AgentContext, input: Self::Input) -> Result<Self::Output> {
//!         // Append user message to conversation
//!         ctx.append_entry(EntryRole::User, &json!({"text": &input})).await?;
//!
//!         // Checkpoint state
//!         ctx.checkpoint(&json!({})).await?;
//!
//!         // Return response
//!         Ok(format!("Echo: {}", input))
//!     }
//! }
//! ```

pub mod combinators;
pub mod context;
pub mod context_impl;
pub mod definition;
pub mod executor;
pub mod future;
pub mod registry;
pub mod signals;
pub mod storage;
pub mod tracer;

// Re-export commonly used types
pub use combinators::{
    agent_join_all, agent_join_all_outcomes, agent_join_all_settled, agent_select, agent_select_ok,
    agent_select_with_cancel, AgentTaskHandle, CancelAttempt, SelectWithCancelResult,
    SettledResult, TaskOutcome,
};
pub use context::{AgentContext, AgentContextExt, EntryRole, EntryType, ScheduleAgentTaskOptions};
pub use context_impl::AgentContextImpl;
pub use definition::{AgentDefinition, DynamicAgent};
pub use executor::{ExecutorResult, RemoteTaskExecutor, TaskExecutor};
pub use registry::{AgentMetadata, AgentRegistry, RegisteredAgent};
pub use signals::{ChannelSignalSource, RemoteSignalSource, SignalResult, SignalSource};
pub use storage::{
    AgentCommand, AgentStorage, CheckpointData, CommandBatch, PendingTask,
    RemoteStorage as RemoteAgentStorage, SegmentState, StorageResult, TaskOptions, TaskResult,
    TaskStatus, TokenUsage as StorageTokenUsage,
};
pub use future::AgentTaskFutureRaw;
pub use tracer::{AgentTracer, CompositeTracer, NoopTracer, StdoutTracer};
