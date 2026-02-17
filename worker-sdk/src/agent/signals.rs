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

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
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
/// Signals are buffered internally by name. When `wait_for_signal` is called,
/// buffered signals matching the name are returned first, then the channel
/// is polled for new signals.
pub struct ChannelSignalSource {
    receiver: Mutex<tokio::sync::mpsc::Receiver<(String, Value)>>,
    /// Buffered signals by name, for signals received but not yet consumed
    buffer: Mutex<HashMap<String, Vec<Value>>>,
}

impl ChannelSignalSource {
    /// Create a new channel signal source from an existing receiver.
    pub fn new(receiver: tokio::sync::mpsc::Receiver<(String, Value)>) -> Self {
        Self {
            receiver: Mutex::new(receiver),
            buffer: Mutex::new(HashMap::new()),
        }
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
        self.receiver.into_inner()
    }

    /// Drain any available signals from the channel into the buffer.
    async fn drain_channel_to_buffer(&self) {
        let mut rx = self.receiver.lock().await;
        let mut buf = self.buffer.lock().await;
        while let Ok((name, value)) = rx.try_recv() {
            buf.entry(name).or_default().push(value);
        }
    }
}

#[async_trait]
impl SignalSource for ChannelSignalSource {
    async fn wait_for_signal(&self, _agent_id: Uuid, signal_name: &str) -> SignalResult<Value> {
        // First check the buffer for already-received signals
        {
            let mut buf = self.buffer.lock().await;
            if let Some(values) = buf.get_mut(signal_name) {
                if !values.is_empty() {
                    return Ok(values.remove(0));
                }
            }
        }

        // No buffered signal - wait on the channel
        let mut rx = self.receiver.lock().await;
        loop {
            match rx.recv().await {
                Some((name, value)) => {
                    if name == signal_name {
                        return Ok(value);
                    }
                    // Buffer signals for other names
                    let mut buf = self.buffer.lock().await;
                    buf.entry(name).or_default().push(value);
                }
                None => {
                    return Err(FlovynError::Other(
                        "Signal channel closed".to_string(),
                    ));
                }
            }
        }
    }

    async fn has_signal(&self, _agent_id: Uuid, signal_name: &str) -> SignalResult<bool> {
        // Drain any pending channel messages into the buffer
        self.drain_channel_to_buffer().await;

        let buf = self.buffer.lock().await;
        Ok(buf
            .get(signal_name)
            .map(|v| !v.is_empty())
            .unwrap_or(false))
    }

    async fn drain_signals(&self, _agent_id: Uuid, signal_name: &str) -> SignalResult<Vec<Value>> {
        // Drain any pending channel messages into the buffer
        self.drain_channel_to_buffer().await;

        let mut buf = self.buffer.lock().await;
        Ok(buf.remove(signal_name).unwrap_or_default())
    }
}

/// Signal delivery mode for interactive signal sources.
#[cfg(feature = "local")]
enum SignalMode {
    /// Read signals from stdin as JSON lines.
    ///
    /// Each line is parsed as `{"signal_name": "...", "payload": ...}`.
    Stdin,
    /// Receive signals via a tokio mpsc channel.
    Channel(Mutex<tokio::sync::mpsc::Receiver<(String, Value)>>),
}

/// Interactive signal source for local agents.
///
/// Supports two modes:
/// - **Stdin**: Reads JSON lines from standard input. Enables interactive CLI agents.
/// - **Channel**: Receives signals programmatically. Used for IDE integrations, tests,
///   and parent agents sending signals to local children.
///
/// # Example
///
/// ```rust,ignore
/// use flovyn_worker_sdk::agent::signals::InteractiveSignalSource;
///
/// // Stdin mode - agent blocks and reads from terminal
/// let source = InteractiveSignalSource::stdin();
///
/// // Channel mode - signals sent programmatically
/// let (tx, rx) = tokio::sync::mpsc::channel(16);
/// let source = InteractiveSignalSource::channel(rx);
/// tx.send(("user-input".to_string(), json!({"text": "hello"}))).await?;
/// ```
#[cfg(feature = "local")]
pub struct InteractiveSignalSource {
    mode: SignalMode,
    /// Buffered signals by name
    buffer: Mutex<HashMap<String, Vec<Value>>>,
}

#[cfg(feature = "local")]
impl InteractiveSignalSource {
    /// Create an interactive signal source that reads from stdin.
    ///
    /// Each line from stdin is parsed as JSON with the format:
    /// `{"signal_name": "...", "payload": ...}`
    pub fn stdin() -> Self {
        Self {
            mode: SignalMode::Stdin,
            buffer: Mutex::new(HashMap::new()),
        }
    }

    /// Create an interactive signal source from a channel receiver.
    pub fn channel(rx: tokio::sync::mpsc::Receiver<(String, Value)>) -> Self {
        Self {
            mode: SignalMode::Channel(Mutex::new(rx)),
            buffer: Mutex::new(HashMap::new()),
        }
    }

    /// Read a signal from stdin by reading a JSON line.
    async fn read_stdin_signal() -> SignalResult<(String, Value)> {
        let line = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            std::io::stdin()
                .read_line(&mut line)
                .map_err(|e| FlovynError::Other(format!("Failed to read stdin: {e}")))?;
            Ok::<String, FlovynError>(line)
        })
        .await
        .map_err(|e| FlovynError::Other(format!("Stdin task failed: {e}")))??;

        let line = line.trim();
        if line.is_empty() {
            return Err(FlovynError::Other("Empty stdin input".to_string()));
        }

        // Parse as JSON object with signal_name and payload
        let parsed: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| FlovynError::Other(format!("Invalid JSON from stdin: {e}")))?;

        let signal_name = parsed
            .get("signal_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FlovynError::Other("Missing 'signal_name' in stdin input".to_string()))?
            .to_string();

        let payload = parsed
            .get("payload")
            .cloned()
            .unwrap_or(Value::Null);

        Ok((signal_name, payload))
    }

    /// Drain available channel messages into the buffer.
    async fn drain_channel_to_buffer(&self) {
        if let SignalMode::Channel(ref rx) = self.mode {
            let mut rx = rx.lock().await;
            let mut buf = self.buffer.lock().await;
            while let Ok((name, value)) = rx.try_recv() {
                buf.entry(name).or_default().push(value);
            }
        }
    }
}

#[cfg(feature = "local")]
#[async_trait]
impl SignalSource for InteractiveSignalSource {
    async fn wait_for_signal(&self, _agent_id: Uuid, signal_name: &str) -> SignalResult<Value> {
        // Check buffer first
        {
            let mut buf = self.buffer.lock().await;
            if let Some(values) = buf.get_mut(signal_name) {
                if !values.is_empty() {
                    return Ok(values.remove(0));
                }
            }
        }

        match &self.mode {
            SignalMode::Stdin => {
                // Read from stdin until we get the right signal
                loop {
                    let (name, payload) = Self::read_stdin_signal().await?;
                    if name == signal_name {
                        return Ok(payload);
                    }
                    // Buffer signals for other names
                    let mut buf = self.buffer.lock().await;
                    buf.entry(name).or_default().push(payload);
                }
            }
            SignalMode::Channel(rx) => {
                let mut rx = rx.lock().await;
                loop {
                    match rx.recv().await {
                        Some((name, value)) => {
                            if name == signal_name {
                                return Ok(value);
                            }
                            let mut buf = self.buffer.lock().await;
                            buf.entry(name).or_default().push(value);
                        }
                        None => {
                            return Err(FlovynError::Other(
                                "Signal channel closed".to_string(),
                            ));
                        }
                    }
                }
            }
        }
    }

    async fn has_signal(&self, _agent_id: Uuid, signal_name: &str) -> SignalResult<bool> {
        self.drain_channel_to_buffer().await;

        let buf = self.buffer.lock().await;
        Ok(buf
            .get(signal_name)
            .map(|v| !v.is_empty())
            .unwrap_or(false))
    }

    async fn drain_signals(&self, _agent_id: Uuid, signal_name: &str) -> SignalResult<Vec<Value>> {
        self.drain_channel_to_buffer().await;

        let mut buf = self.buffer.lock().await;
        Ok(buf.remove(signal_name).unwrap_or_default())
    }
}

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

        // Use SignalSource trait to receive the signal
        let received = source
            .wait_for_signal(Uuid::nil(), "test_signal")
            .await
            .unwrap();
        assert_eq!(received, payload);
    }

    #[tokio::test]
    async fn test_channel_signal_source_has_signal() {
        let (sender, source) = ChannelSignalSource::channel(16);

        // No signal yet
        assert!(!source.has_signal(Uuid::nil(), "test").await.unwrap());

        // Send a signal
        sender
            .send(("test".to_string(), serde_json::json!("value")))
            .await
            .unwrap();

        // Now there should be a signal
        assert!(source.has_signal(Uuid::nil(), "test").await.unwrap());
        // Different name should not have a signal
        assert!(!source.has_signal(Uuid::nil(), "other").await.unwrap());
    }

    #[tokio::test]
    async fn test_channel_signal_source_drain_signals() {
        let (sender, source) = ChannelSignalSource::channel(16);

        // Send multiple signals with same name
        sender
            .send(("test".to_string(), serde_json::json!(1)))
            .await
            .unwrap();
        sender
            .send(("test".to_string(), serde_json::json!(2)))
            .await
            .unwrap();
        sender
            .send(("other".to_string(), serde_json::json!(3)))
            .await
            .unwrap();

        // Drain should return both "test" signals
        let values = source.drain_signals(Uuid::nil(), "test").await.unwrap();
        assert_eq!(values, vec![serde_json::json!(1), serde_json::json!(2)]);

        // "other" should still be available
        let values = source.drain_signals(Uuid::nil(), "other").await.unwrap();
        assert_eq!(values, vec![serde_json::json!(3)]);

        // No more signals
        let values = source.drain_signals(Uuid::nil(), "test").await.unwrap();
        assert!(values.is_empty());
    }

    #[tokio::test]
    async fn test_channel_signal_source_buffers_other_signals() {
        let (sender, source) = ChannelSignalSource::channel(16);

        // Send signals with different names
        sender
            .send(("other".to_string(), serde_json::json!("buffered")))
            .await
            .unwrap();
        sender
            .send(("target".to_string(), serde_json::json!("found")))
            .await
            .unwrap();

        // wait_for_signal("target") should skip "other" and return "found"
        let result = source
            .wait_for_signal(Uuid::nil(), "target")
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!("found"));

        // "other" should still be buffered
        let values = source.drain_signals(Uuid::nil(), "other").await.unwrap();
        assert_eq!(values, vec![serde_json::json!("buffered")]);
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn test_interactive_signal_channel_delivery() {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let source = InteractiveSignalSource::channel(rx);

        // Send a signal
        tx.send(("user-input".to_string(), serde_json::json!({"text": "hello"})))
            .await
            .unwrap();

        // Should receive it
        let result = source
            .wait_for_signal(Uuid::nil(), "user-input")
            .await
            .unwrap();
        assert_eq!(result, serde_json::json!({"text": "hello"}));
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn test_interactive_signal_has_signal() {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let source = InteractiveSignalSource::channel(rx);

        // No signal initially
        assert!(!source.has_signal(Uuid::nil(), "test").await.unwrap());

        // Send a signal
        tx.send(("test".to_string(), serde_json::json!("value")))
            .await
            .unwrap();

        // Should have the signal
        assert!(source.has_signal(Uuid::nil(), "test").await.unwrap());
        assert!(!source.has_signal(Uuid::nil(), "other").await.unwrap());
    }

    #[cfg(feature = "local")]
    #[tokio::test]
    async fn test_interactive_signal_drain() {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let source = InteractiveSignalSource::channel(rx);

        // Send multiple signals
        tx.send(("test".to_string(), serde_json::json!(1)))
            .await
            .unwrap();
        tx.send(("test".to_string(), serde_json::json!(2)))
            .await
            .unwrap();
        tx.send(("other".to_string(), serde_json::json!(3)))
            .await
            .unwrap();

        // Drain "test" signals
        let values = source.drain_signals(Uuid::nil(), "test").await.unwrap();
        assert_eq!(values, vec![serde_json::json!(1), serde_json::json!(2)]);

        // "other" still available
        let values = source.drain_signals(Uuid::nil(), "other").await.unwrap();
        assert_eq!(values, vec![serde_json::json!(3)]);

        // No more
        let values = source.drain_signals(Uuid::nil(), "test").await.unwrap();
        assert!(values.is_empty());
    }
}
