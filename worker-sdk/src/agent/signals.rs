//! Signal delivery abstraction for agent execution
//!
//! This module defines the `SignalSource` trait for pluggable signal delivery.
//! Signals are external events that can resume suspended agents.
//!
//! ## Implementations
//!
//! - `RemoteSignalSource`: Signals delivered via server (production)
//! - `ChannelSignalSource`: Signals delivered via tokio channel (local agents)
//!
//! ## Design
//!
//! Remote agents suspend when waiting for signals - the server handles
//! signal delivery and resumes the agent. Local agents can use channels
//! for in-process signal delivery without server involvement.
//!
//! ## Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::signals::*;
//!
//! // Remote signals (production) - handled via suspension
//! let source = RemoteSignalSource::new();
//!
//! // Channel signals (local/testing)
//! let (sender, source) = ChannelSignalSource::channel(16);
//! sender.send(("user_input".to_string(), json!({"text": "hello"}))).await?;
//! ```

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use crate::error::FlovynError;

/// Result type for signal operations
pub type SignalResult<T> = Result<T, FlovynError>;

/// Abstraction for receiving signals.
///
/// Agents can wait for signals from external sources. The signal source
/// determines how signals are delivered:
///
/// - Remote: Signals arrive via server, agent suspends until signal arrives
/// - Channel: Signals arrive via tokio channel, for local/in-process agents
#[async_trait]
pub trait SignalSource: Send + Sync {
    /// Wait for a signal with the given name.
    ///
    /// This method blocks until a signal arrives. For remote signals,
    /// this triggers agent suspension.
    async fn wait_for_signal(&self, agent_id: Uuid, signal_name: &str) -> SignalResult<Value>;

    /// Check if a signal is available without consuming it.
    ///
    /// Returns `true` if at least one signal with the given name is pending.
    async fn has_signal(&self, agent_id: Uuid, signal_name: &str) -> SignalResult<bool>;

    /// Consume and return all pending signals with the given name.
    ///
    /// Returns an empty vector if no signals are pending.
    async fn drain_signals(&self, agent_id: Uuid, signal_name: &str) -> SignalResult<Vec<Value>>;
}

/// Remote signal source for production agents.
///
/// Remote signals are handled via agent suspension - when an agent waits
/// for a signal, it suspends and the server handles signal delivery.
/// This struct is a placeholder that returns errors for direct calls,
/// since the actual signal handling happens at a higher level.
pub struct RemoteSignalSource;

impl RemoteSignalSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RemoteSignalSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SignalSource for RemoteSignalSource {
    async fn wait_for_signal(&self, _agent_id: Uuid, _signal_name: &str) -> SignalResult<Value> {
        Err(FlovynError::Other(
            "Remote signals are handled via agent suspension".to_string(),
        ))
    }

    async fn has_signal(&self, _agent_id: Uuid, _signal_name: &str) -> SignalResult<bool> {
        Ok(false)
    }

    async fn drain_signals(&self, _agent_id: Uuid, _signal_name: &str) -> SignalResult<Vec<Value>> {
        Ok(vec![])
    }
}

/// Channel-based signal source for local agents.
///
/// Receives signals via a tokio mpsc channel. This enables in-process
/// signal delivery for local agents without server involvement.
///
/// The SignalSource trait implementation is deferred to Phase 4 (Local Agent Mode).
pub struct ChannelSignalSource {
    // Allow dead_code until Phase 4 implements SignalSource for ChannelSignalSource
    #[allow(dead_code)]
    receiver: tokio::sync::mpsc::Receiver<(String, Value)>,
}

impl ChannelSignalSource {
    /// Create a new channel signal source from an existing receiver.
    pub fn new(receiver: tokio::sync::mpsc::Receiver<(String, Value)>) -> Self {
        Self { receiver }
    }

    /// Create a channel pair for signal delivery.
    ///
    /// Returns a sender (for external code to send signals) and a
    /// ChannelSignalSource (for the agent to receive signals).
    ///
    /// # Arguments
    ///
    /// * `buffer` - Channel buffer size
    pub fn channel(buffer: usize) -> (tokio::sync::mpsc::Sender<(String, Value)>, Self) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer);
        (tx, Self::new(rx))
    }

    /// Consume self and return the underlying receiver.
    ///
    /// This is useful for testing or when direct access to the channel is needed.
    pub fn into_receiver(self) -> tokio::sync::mpsc::Receiver<(String, Value)> {
        self.receiver
    }
}

// Note: ChannelSignalSource SignalSource impl deferred to Phase 4 (Local Agent Mode)

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_remote_signal_source_wait_returns_error() {
        let source = RemoteSignalSource::new();
        let result = source.wait_for_signal(Uuid::nil(), "test").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("suspension"));
    }

    #[tokio::test]
    async fn test_remote_signal_source_has_signal_returns_false() {
        let source = RemoteSignalSource::new();
        let result = source.has_signal(Uuid::nil(), "test").await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_remote_signal_source_drain_returns_empty() {
        let source = RemoteSignalSource::new();
        let result = source.drain_signals(Uuid::nil(), "test").await;
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_remote_signal_source_default() {
        let source = RemoteSignalSource;
        let result = source.has_signal(Uuid::nil(), "test").await;
        assert!(!result.unwrap());
    }

    #[test]
    fn test_channel_signal_source_channel() {
        let (sender, _source) = ChannelSignalSource::channel(16);
        // Verify the sender is functional (can be cloned)
        let _sender2 = sender.clone();
    }

    #[tokio::test]
    async fn test_channel_signal_source_receives_signal() {
        let (sender, source) = ChannelSignalSource::channel(16);

        // Send a signal
        let payload = serde_json::json!({"message": "hello"});
        sender
            .send(("test_signal".to_string(), payload.clone()))
            .await
            .unwrap();

        // Convert to inner receiver to verify signal was sent
        // (SignalSource impl is deferred to Phase 4)
        let mut receiver = source.into_receiver();
        let received = receiver.recv().await.unwrap();
        assert_eq!(received.0, "test_signal");
        assert_eq!(received.1, payload);
    }
}
