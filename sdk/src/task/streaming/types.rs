//! Stream event types for real-time task data streaming.
//!
//! Stream events are ephemeral (not persisted) and delivered to connected clients
//! in real-time via Server-Sent Events (SSE). They are designed for:
//! - LLM token streaming
//! - Progress monitoring
//! - Real-time data pipelines
//! - Error notifications

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::SystemTime;
use uuid::Uuid;

/// Type of stream event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamEventType {
    /// LLM token or text chunk
    Token,
    /// Progress update (0.0-1.0)
    Progress,
    /// Arbitrary structured data
    Data,
    /// Error notification
    Error,
}

/// Stream event emitted by tasks during execution.
///
/// Events are ephemeral (not persisted) and delivered to connected clients
/// in real-time via Server-Sent Events (SSE).
///
/// # Example
///
/// ```rust
/// use flovyn_sdk::task::streaming::StreamEvent;
/// use serde_json::json;
///
/// // Stream a token
/// let event = StreamEvent::token("Hello");
///
/// // Stream progress
/// let event = StreamEvent::progress(0.5, Some("Processing..."));
///
/// // Stream data
/// let event = StreamEvent::data(json!({"count": 42})).unwrap();
///
/// // Stream an error
/// let event = StreamEvent::error("Rate limit exceeded", Some("RATE_LIMIT"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StreamEvent {
    /// LLM token or text chunk.
    ///
    /// Use for streaming text generation from language models.
    Token {
        /// The token or text chunk
        text: String,
    },

    /// Progress update with optional details.
    ///
    /// Use for long-running tasks to show completion percentage.
    Progress {
        /// Progress value (0.0 to 1.0), clamped during serialization
        #[serde(serialize_with = "serialize_progress")]
        progress: f64,
        /// Optional progress details
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<String>,
    },

    /// Arbitrary structured data.
    ///
    /// Use for streaming intermediate results or custom events.
    Data {
        /// Data payload (JSON-serializable)
        data: Value,
    },

    /// Error notification.
    ///
    /// Use to notify clients of recoverable errors during execution.
    /// For fatal errors, let the task fail normally.
    Error {
        /// Error message
        message: String,
        /// Optional error code
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl StreamEvent {
    /// Get the event type.
    pub fn event_type(&self) -> StreamEventType {
        match self {
            StreamEvent::Token { .. } => StreamEventType::Token,
            StreamEvent::Progress { .. } => StreamEventType::Progress,
            StreamEvent::Data { .. } => StreamEventType::Data,
            StreamEvent::Error { .. } => StreamEventType::Error,
        }
    }

    /// Create a token event.
    ///
    /// # Example
    ///
    /// ```rust
    /// use flovyn_sdk::task::streaming::StreamEvent;
    ///
    /// let event = StreamEvent::token("Hello, ");
    /// ```
    pub fn token(text: impl Into<String>) -> Self {
        StreamEvent::Token { text: text.into() }
    }

    /// Create a progress event.
    ///
    /// Progress values are clamped to 0.0-1.0 during serialization.
    ///
    /// # Example
    ///
    /// ```rust
    /// use flovyn_sdk::task::streaming::StreamEvent;
    ///
    /// let event = StreamEvent::progress(0.5, Some("Halfway done"));
    /// let event = StreamEvent::progress(0.75, None::<&str>);
    /// ```
    pub fn progress(progress: f64, details: Option<impl Into<String>>) -> Self {
        StreamEvent::Progress {
            progress,
            details: details.map(Into::into),
        }
    }

    /// Create a data event.
    ///
    /// # Example
    ///
    /// ```rust
    /// use flovyn_sdk::task::streaming::StreamEvent;
    /// use serde_json::json;
    ///
    /// let event = StreamEvent::data(json!({"rows_processed": 1000})).unwrap();
    /// ```
    pub fn data(data: impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(StreamEvent::Data {
            data: serde_json::to_value(data)?,
        })
    }

    /// Create a data event from a pre-serialized JSON value.
    pub fn data_value(data: Value) -> Self {
        StreamEvent::Data { data }
    }

    /// Create an error event.
    ///
    /// # Example
    ///
    /// ```rust
    /// use flovyn_sdk::task::streaming::StreamEvent;
    ///
    /// let event = StreamEvent::error("Connection failed", Some("CONN_ERR"));
    /// let event = StreamEvent::error("Retrying...", None::<&str>);
    /// ```
    pub fn error(message: impl Into<String>, code: Option<impl Into<String>>) -> Self {
        StreamEvent::Error {
            message: message.into(),
            code: code.map(Into::into),
        }
    }
}

/// Serialize progress value, clamping to 0.0-1.0 range.
fn serialize_progress<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_f64(value.clamp(0.0, 1.0))
}

/// Stream event with task context, used for client-side consumption.
#[derive(Debug, Clone)]
pub struct TaskStreamEvent {
    /// Task that emitted the event
    pub task_execution_id: Uuid,
    /// Workflow containing the task
    pub workflow_execution_id: Uuid,
    /// Event sequence number
    pub sequence: u32,
    /// The stream event
    pub event: StreamEvent,
    /// Timestamp when event was emitted
    pub timestamp: SystemTime,
}

impl TaskStreamEvent {
    /// Create a new TaskStreamEvent.
    pub fn new(
        task_execution_id: Uuid,
        workflow_execution_id: Uuid,
        sequence: u32,
        event: StreamEvent,
        timestamp: SystemTime,
    ) -> Self {
        Self {
            task_execution_id,
            workflow_execution_id,
            sequence,
            event,
            timestamp,
        }
    }

    /// Get the event type.
    pub fn event_type(&self) -> StreamEventType {
        self.event.event_type()
    }
}

/// Error type for streaming operations.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// Connection was closed.
    #[error("Connection closed")]
    ConnectionClosed,

    /// Stream operation timed out.
    #[error("Stream timeout")]
    Timeout,

    /// Failed to parse stream data.
    #[error("Parse error: {0}")]
    ParseError(String),

    /// HTTP request failed.
    #[error("HTTP error: {0}")]
    HttpError(String),

    /// gRPC request failed.
    #[error("gRPC error: {0}")]
    GrpcError(#[from] tonic::Status),

    /// Serialization failed.
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_stream_event_type_serialization() {
        assert_eq!(
            serde_json::to_string(&StreamEventType::Token).unwrap(),
            "\"token\""
        );
        assert_eq!(
            serde_json::to_string(&StreamEventType::Progress).unwrap(),
            "\"progress\""
        );
        assert_eq!(
            serde_json::to_string(&StreamEventType::Data).unwrap(),
            "\"data\""
        );
        assert_eq!(
            serde_json::to_string(&StreamEventType::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn test_stream_event_type_deserialization() {
        assert_eq!(
            serde_json::from_str::<StreamEventType>("\"token\"").unwrap(),
            StreamEventType::Token
        );
        assert_eq!(
            serde_json::from_str::<StreamEventType>("\"progress\"").unwrap(),
            StreamEventType::Progress
        );
    }

    #[test]
    fn test_stream_event_token() {
        let event = StreamEvent::token("Hello");
        assert_eq!(event.event_type(), StreamEventType::Token);

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "token");
        assert_eq!(json["text"], "Hello");
    }

    #[test]
    fn test_stream_event_progress() {
        let event = StreamEvent::progress(0.5, Some("Halfway"));
        assert_eq!(event.event_type(), StreamEventType::Progress);

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "progress");
        assert_eq!(json["progress"], 0.5);
        assert_eq!(json["details"], "Halfway");
    }

    #[test]
    fn test_stream_event_progress_without_details() {
        let event = StreamEvent::progress(0.75, None::<&str>);

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "progress");
        assert_eq!(json["progress"], 0.75);
        assert!(json.get("details").is_none());
    }

    #[test]
    fn test_stream_event_progress_clamping() {
        // Test negative progress is clamped to 0
        let event = StreamEvent::progress(-0.5, None::<&str>);
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["progress"], 0.0);

        // Test progress > 1 is clamped to 1
        let event = StreamEvent::progress(1.5, None::<&str>);
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["progress"], 1.0);

        // Test normal progress is not affected
        let event = StreamEvent::progress(0.5, None::<&str>);
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["progress"], 0.5);
    }

    #[test]
    fn test_stream_event_data() {
        let event = StreamEvent::data(json!({"count": 42, "name": "test"})).unwrap();
        assert_eq!(event.event_type(), StreamEventType::Data);

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "data");
        assert_eq!(json["data"]["count"], 42);
        assert_eq!(json["data"]["name"], "test");
    }

    #[test]
    fn test_stream_event_data_value() {
        let event = StreamEvent::data_value(json!({"key": "value"}));
        assert_eq!(event.event_type(), StreamEventType::Data);
    }

    #[test]
    fn test_stream_event_error() {
        let event = StreamEvent::error("Something went wrong", Some("ERR_001"));
        assert_eq!(event.event_type(), StreamEventType::Error);

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["message"], "Something went wrong");
        assert_eq!(json["code"], "ERR_001");
    }

    #[test]
    fn test_stream_event_error_without_code() {
        let event = StreamEvent::error("Error message", None::<&str>);

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["message"], "Error message");
        assert!(json.get("code").is_none());
    }

    #[test]
    fn test_stream_event_deserialization() {
        // Token
        let json = r#"{"type":"token","text":"Hello"}"#;
        let event: StreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, StreamEvent::Token { text } if text == "Hello"));

        // Progress
        let json = r#"{"type":"progress","progress":0.5,"details":"Test"}"#;
        let event: StreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, StreamEvent::Progress { progress, details }
            if progress == 0.5 && details == Some("Test".to_string())));

        // Data
        let json = r#"{"type":"data","data":{"key":"value"}}"#;
        let event: StreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, StreamEvent::Data { .. }));

        // Error
        let json = r#"{"type":"error","message":"Failed","code":"E001"}"#;
        let event: StreamEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, StreamEvent::Error { message, code }
            if message == "Failed" && code == Some("E001".to_string())));
    }

    #[test]
    fn test_task_stream_event() {
        let task_id = Uuid::new_v4();
        let workflow_id = Uuid::new_v4();
        let event = StreamEvent::token("test");
        let timestamp = SystemTime::now();

        let task_event = TaskStreamEvent::new(task_id, workflow_id, 42, event, timestamp);

        assert_eq!(task_event.task_execution_id, task_id);
        assert_eq!(task_event.workflow_execution_id, workflow_id);
        assert_eq!(task_event.sequence, 42);
        assert_eq!(task_event.event_type(), StreamEventType::Token);
    }

    #[test]
    fn test_stream_error_display() {
        let err = StreamError::ConnectionClosed;
        assert_eq!(err.to_string(), "Connection closed");

        let err = StreamError::Timeout;
        assert_eq!(err.to_string(), "Stream timeout");

        let err = StreamError::ParseError("invalid json".to_string());
        assert_eq!(err.to_string(), "Parse error: invalid json");

        let err = StreamError::HttpError("404 Not Found".to_string());
        assert_eq!(err.to_string(), "HTTP error: 404 Not Found");
    }
}
