//! Agent tracing and observability
//!
//! This module provides the [`AgentTracer`] trait for structured observability
//! of agent execution, including turn lifecycle, tool calls, token usage,
//! errors, and checkpoints.
//!
//! ## Implementations
//!
//! - [`NoopTracer`] - Discards all trace events (default, zero overhead)
//! - [`StdoutTracer`] - Prints trace events to stdout (useful for debugging)
//! - [`CompositeTracer`] - Forwards events to multiple tracers
//!
//! ## Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::tracer::{AgentTracer, StdoutTracer};
//!
//! // Create a verbose tracer for debugging
//! let tracer = StdoutTracer::verbose();
//!
//! // Use with agent execution
//! tracer.trace_turn_start(agent_id, 1, &json!({"message": "Hello"}));
//! tracer.trace_tool_call(agent_id, "search", &json!({"query": "rust"}), Duration::from_millis(150));
//! tracer.trace_tokens(agent_id, 100, 50, "claude-3-opus");
//! tracer.trace_turn_end(agent_id, 1, &json!({"response": "Hi there!"}));
//! ```

use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

/// Trait for observing agent execution events.
///
/// Implementations must be thread-safe (`Send + Sync`) to support
/// concurrent agent execution.
pub trait AgentTracer: Send + Sync {
    /// Called when an agent turn begins.
    ///
    /// # Arguments
    /// * `agent_id` - Unique identifier for the agent instance
    /// * `turn` - Turn number (1-indexed)
    /// * `input` - Input data for this turn
    fn trace_turn_start(&self, agent_id: Uuid, turn: u64, input: &Value);

    /// Called when a tool is invoked.
    ///
    /// # Arguments
    /// * `agent_id` - Unique identifier for the agent instance
    /// * `tool` - Name of the tool being called
    /// * `input` - Input parameters for the tool
    /// * `latency` - Time taken to execute the tool
    fn trace_tool_call(&self, agent_id: Uuid, tool: &str, input: &Value, latency: Duration);

    /// Called to record token usage for a model call.
    ///
    /// # Arguments
    /// * `agent_id` - Unique identifier for the agent instance
    /// * `input_tokens` - Number of input tokens consumed
    /// * `output_tokens` - Number of output tokens generated
    /// * `model` - Model identifier (e.g., "claude-3-opus")
    fn trace_tokens(&self, agent_id: Uuid, input_tokens: u64, output_tokens: u64, model: &str);

    /// Called when an agent turn completes.
    ///
    /// # Arguments
    /// * `agent_id` - Unique identifier for the agent instance
    /// * `turn` - Turn number (1-indexed)
    /// * `output` - Output data from this turn
    fn trace_turn_end(&self, agent_id: Uuid, turn: u64, output: &Value);

    /// Called when an error occurs during agent execution.
    ///
    /// # Arguments
    /// * `agent_id` - Unique identifier for the agent instance
    /// * `error` - Error message
    /// * `context` - Optional additional context about the error
    fn trace_error(&self, agent_id: Uuid, error: &str, context: Option<&Value>);

    /// Called when a checkpoint is created.
    ///
    /// # Arguments
    /// * `agent_id` - Unique identifier for the agent instance
    /// * `segment` - Segment number for this checkpoint
    /// * `state_size_bytes` - Size of the checkpoint state in bytes
    fn trace_checkpoint(&self, agent_id: Uuid, segment: u64, state_size_bytes: usize);
}

/// A no-op tracer that discards all events.
///
/// This is the default tracer implementation with zero runtime overhead.
/// Use this in production when observability is handled by other means
/// or when tracing is not needed.
#[derive(Default, Clone, Debug)]
pub struct NoopTracer;

impl NoopTracer {
    /// Creates a new no-op tracer.
    pub fn new() -> Self {
        Self
    }
}

impl AgentTracer for NoopTracer {
    fn trace_turn_start(&self, _agent_id: Uuid, _turn: u64, _input: &Value) {}

    fn trace_tool_call(&self, _agent_id: Uuid, _tool: &str, _input: &Value, _latency: Duration) {}

    fn trace_tokens(&self, _agent_id: Uuid, _input_tokens: u64, _output_tokens: u64, _model: &str) {}

    fn trace_turn_end(&self, _agent_id: Uuid, _turn: u64, _output: &Value) {}

    fn trace_error(&self, _agent_id: Uuid, _error: &str, _context: Option<&Value>) {}

    fn trace_checkpoint(&self, _agent_id: Uuid, _segment: u64, _state_size_bytes: usize) {}
}

/// A tracer that prints events to stdout.
///
/// Useful for debugging and development. In verbose mode, includes
/// full input/output data in the trace output.
#[derive(Default, Clone, Debug)]
pub struct StdoutTracer {
    /// When true, includes full input/output values in trace output.
    pub verbose: bool,
}

impl StdoutTracer {
    /// Creates a new stdout tracer with verbose mode disabled.
    pub fn new() -> Self {
        Self { verbose: false }
    }

    /// Creates a new stdout tracer with verbose mode enabled.
    pub fn verbose() -> Self {
        Self { verbose: true }
    }
}

impl AgentTracer for StdoutTracer {
    fn trace_turn_start(&self, agent_id: Uuid, turn: u64, input: &Value) {
        if self.verbose {
            println!(
                "[AGENT {}] Turn {} started with input: {}",
                agent_id, turn, input
            );
        } else {
            println!("[AGENT {}] Turn {} started", agent_id, turn);
        }
    }

    fn trace_tool_call(&self, agent_id: Uuid, tool: &str, input: &Value, latency: Duration) {
        if self.verbose {
            println!(
                "[AGENT {}] Tool '{}' called with input: {} (latency: {:?})",
                agent_id, tool, input, latency
            );
        } else {
            println!(
                "[AGENT {}] Tool '{}' called (latency: {:?})",
                agent_id, tool, latency
            );
        }
    }

    fn trace_tokens(&self, agent_id: Uuid, input_tokens: u64, output_tokens: u64, model: &str) {
        println!(
            "[AGENT {}] Tokens: {} in / {} out (model: {})",
            agent_id, input_tokens, output_tokens, model
        );
    }

    fn trace_turn_end(&self, agent_id: Uuid, turn: u64, output: &Value) {
        if self.verbose {
            println!(
                "[AGENT {}] Turn {} completed with output: {}",
                agent_id, turn, output
            );
        } else {
            println!("[AGENT {}] Turn {} completed", agent_id, turn);
        }
    }

    fn trace_error(&self, agent_id: Uuid, error: &str, context: Option<&Value>) {
        match context {
            Some(ctx) if self.verbose => {
                println!("[AGENT {}] Error: {} (context: {})", agent_id, error, ctx);
            }
            _ => {
                println!("[AGENT {}] Error: {}", agent_id, error);
            }
        }
    }

    fn trace_checkpoint(&self, agent_id: Uuid, segment: u64, state_size_bytes: usize) {
        println!(
            "[AGENT {}] Checkpoint at segment {} ({} bytes)",
            agent_id, segment, state_size_bytes
        );
    }
}

/// A tracer that forwards events to multiple underlying tracers.
///
/// Use this to combine multiple tracers, for example to both log to stdout
/// and send metrics to a monitoring system.
pub struct CompositeTracer {
    tracers: Vec<Box<dyn AgentTracer>>,
}

impl CompositeTracer {
    /// Creates a new composite tracer with the given underlying tracers.
    pub fn new(tracers: Vec<Box<dyn AgentTracer>>) -> Self {
        Self { tracers }
    }
}

impl AgentTracer for CompositeTracer {
    fn trace_turn_start(&self, agent_id: Uuid, turn: u64, input: &Value) {
        for tracer in &self.tracers {
            tracer.trace_turn_start(agent_id, turn, input);
        }
    }

    fn trace_tool_call(&self, agent_id: Uuid, tool: &str, input: &Value, latency: Duration) {
        for tracer in &self.tracers {
            tracer.trace_tool_call(agent_id, tool, input, latency);
        }
    }

    fn trace_tokens(&self, agent_id: Uuid, input_tokens: u64, output_tokens: u64, model: &str) {
        for tracer in &self.tracers {
            tracer.trace_tokens(agent_id, input_tokens, output_tokens, model);
        }
    }

    fn trace_turn_end(&self, agent_id: Uuid, turn: u64, output: &Value) {
        for tracer in &self.tracers {
            tracer.trace_turn_end(agent_id, turn, output);
        }
    }

    fn trace_error(&self, agent_id: Uuid, error: &str, context: Option<&Value>) {
        for tracer in &self.tracers {
            tracer.trace_error(agent_id, error, context);
        }
    }

    fn trace_checkpoint(&self, agent_id: Uuid, segment: u64, state_size_bytes: usize) {
        for tracer in &self.tracers {
            tracer.trace_checkpoint(agent_id, segment, state_size_bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_noop_tracer_does_nothing() {
        let tracer = NoopTracer::new();
        let agent_id = Uuid::new_v4();

        // These should all complete without error
        tracer.trace_turn_start(agent_id, 1, &json!({"test": "input"}));
        tracer.trace_tool_call(agent_id, "tool", &json!({}), Duration::from_millis(100));
        tracer.trace_tokens(agent_id, 100, 50, "model");
        tracer.trace_turn_end(agent_id, 1, &json!({"test": "output"}));
        tracer.trace_error(agent_id, "error", Some(&json!({"ctx": "value"})));
        tracer.trace_checkpoint(agent_id, 1, 1024);
    }

    #[test]
    fn test_stdout_tracer_creation() {
        let tracer = StdoutTracer::new();
        assert!(!tracer.verbose);

        let verbose_tracer = StdoutTracer::verbose();
        assert!(verbose_tracer.verbose);
    }

    #[test]
    fn test_composite_tracer_forwards_to_all() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        struct CountingTracer {
            count: Arc<AtomicU64>,
        }

        impl AgentTracer for CountingTracer {
            fn trace_turn_start(&self, _: Uuid, _: u64, _: &Value) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
            fn trace_tool_call(&self, _: Uuid, _: &str, _: &Value, _: Duration) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
            fn trace_tokens(&self, _: Uuid, _: u64, _: u64, _: &str) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
            fn trace_turn_end(&self, _: Uuid, _: u64, _: &Value) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
            fn trace_error(&self, _: Uuid, _: &str, _: Option<&Value>) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
            fn trace_checkpoint(&self, _: Uuid, _: u64, _: usize) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
        }

        let count1 = Arc::new(AtomicU64::new(0));
        let count2 = Arc::new(AtomicU64::new(0));

        let tracer1 = CountingTracer {
            count: Arc::clone(&count1),
        };
        let tracer2 = CountingTracer {
            count: Arc::clone(&count2),
        };

        let composite = CompositeTracer::new(vec![Box::new(tracer1), Box::new(tracer2)]);

        let agent_id = Uuid::new_v4();
        composite.trace_turn_start(agent_id, 1, &json!({}));
        composite.trace_tool_call(agent_id, "tool", &json!({}), Duration::from_millis(100));
        composite.trace_tokens(agent_id, 100, 50, "model");
        composite.trace_turn_end(agent_id, 1, &json!({}));
        composite.trace_error(agent_id, "error", None);
        composite.trace_checkpoint(agent_id, 1, 1024);

        assert_eq!(count1.load(Ordering::SeqCst), 6);
        assert_eq!(count2.load(Ordering::SeqCst), 6);
    }
}
