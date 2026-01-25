//! # Flovyn Worker NAPI
//!
//! This crate provides NAPI-RS bindings for the Flovyn workflow orchestration platform,
//! enabling Node.js SDK support.
//!
//! ## Architecture
//!
//! The NAPI layer exposes an activation-based protocol where:
//! - Core polls for work and returns `WorkflowActivation` / `TaskActivation`
//! - Node.js SDK processes activations and returns completions
//! - Core handles all gRPC communication, replay, and determinism validation
//!
//! ## Exported Types
//!
//! ### Classes (objects with methods)
//! - [`NapiWorker`] - Main worker object with poll/complete methods
//! - [`NapiClient`] - Client for starting workflows, queries, signals
//! - [`NapiWorkflowContext`] - Replay-aware workflow context
//! - [`NapiTaskContext`] - Task context with streaming support
//!
//! ### Objects (plain data)
//! - [`WorkerConfig`] - Worker configuration
//! - [`ClientConfig`] - Client configuration
//!
//! ### Enums
//! - [`WorkflowCompletionStatus`] - Workflow completion result
//! - [`TaskCompletion`] - Task completion result

#![deny(clippy::all)]

mod activation;
mod client;
mod command;
mod config;
mod context;
mod error;
mod types;
mod worker;

pub use activation::*;
pub use client::*;
pub use command::*;
pub use config::*;
pub use context::*;
pub use error::*;
pub use types::*;
pub use worker::*;
