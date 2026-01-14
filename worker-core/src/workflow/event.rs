//! Workflow event types for replay

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Event types that can be recorded during workflow execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    // Workflow lifecycle events
    WorkflowStarted,
    WorkflowCompleted,
    WorkflowExecutionFailed,
    WorkflowSuspended,
    CancellationRequested,

    // Operation events
    OperationCompleted,

    // State events
    StateSet,
    StateCleared,

    // Task events
    TaskScheduled,
    TaskCompleted,
    TaskFailed,
    TaskCancelled,

    // Promise events
    PromiseCreated,
    PromiseResolved,
    PromiseRejected,
    PromiseTimeout,

    // Child workflow events
    ChildWorkflowInitiated,
    ChildWorkflowStarted,
    ChildWorkflowCompleted,
    ChildWorkflowFailed,
    ChildWorkflowCancelled,

    // Timer events
    TimerStarted,
    TimerFired,
    TimerCancelled,
}

impl EventType {
    /// Check if this event type is a terminal workflow event
    pub fn is_workflow_terminal(&self) -> bool {
        matches!(
            self,
            Self::WorkflowCompleted | Self::WorkflowExecutionFailed
        )
    }

    /// Check if this event type is a terminal task event
    pub fn is_task_terminal(&self) -> bool {
        matches!(
            self,
            Self::TaskCompleted | Self::TaskFailed | Self::TaskCancelled
        )
    }

    /// Check if this event type is a terminal child workflow event
    pub fn is_child_workflow_terminal(&self) -> bool {
        matches!(
            self,
            Self::ChildWorkflowCompleted | Self::ChildWorkflowFailed | Self::ChildWorkflowCancelled
        )
    }

    /// Check if this event type is a terminal promise event
    pub fn is_promise_terminal(&self) -> bool {
        matches!(
            self,
            Self::PromiseResolved | Self::PromiseRejected | Self::PromiseTimeout
        )
    }

    /// Get the string representation matching Kotlin's EventType
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkflowStarted => "WORKFLOW_STARTED",
            Self::WorkflowCompleted => "WORKFLOW_COMPLETED",
            Self::WorkflowExecutionFailed => "WORKFLOW_EXECUTION_FAILED",
            Self::WorkflowSuspended => "WORKFLOW_SUSPENDED",
            Self::CancellationRequested => "CANCELLATION_REQUESTED",
            Self::OperationCompleted => "OPERATION_COMPLETED",
            Self::StateSet => "STATE_SET",
            Self::StateCleared => "STATE_CLEARED",
            Self::TaskScheduled => "TASK_SCHEDULED",
            Self::TaskCompleted => "TASK_COMPLETED",
            Self::TaskFailed => "TASK_FAILED",
            Self::TaskCancelled => "TASK_CANCELLED",
            Self::PromiseCreated => "PROMISE_CREATED",
            Self::PromiseResolved => "PROMISE_RESOLVED",
            Self::PromiseRejected => "PROMISE_REJECTED",
            Self::PromiseTimeout => "PROMISE_TIMEOUT",
            Self::ChildWorkflowInitiated => "CHILD_WORKFLOW_INITIATED",
            Self::ChildWorkflowStarted => "CHILD_WORKFLOW_STARTED",
            Self::ChildWorkflowCompleted => "CHILD_WORKFLOW_COMPLETED",
            Self::ChildWorkflowFailed => "CHILD_WORKFLOW_FAILED",
            Self::ChildWorkflowCancelled => "CHILD_WORKFLOW_CANCELLED",
            Self::TimerStarted => "TIMER_STARTED",
            Self::TimerFired => "TIMER_FIRED",
            Self::TimerCancelled => "TIMER_CANCELLED",
        }
    }
}

/// A replay event from the event log
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayEvent {
    /// Sequence number of this event (1-indexed)
    #[serde(rename = "sequenceNumber")]
    sequence_number: i32,

    /// Type of the event
    #[serde(rename = "type")]
    event_type: EventType,

    /// Event data (varies by event type)
    data: Value,

    /// Timestamp of the event
    #[serde(rename = "timestamp")]
    timestamp: DateTime<Utc>,
}

impl ReplayEvent {
    /// Create a new replay event
    pub fn new(
        sequence_number: i32,
        event_type: EventType,
        data: Value,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            sequence_number,
            event_type,
            data,
            timestamp,
        }
    }

    /// Get the sequence number
    pub fn sequence_number(&self) -> i32 {
        self.sequence_number
    }

    /// Get the event type
    pub fn event_type(&self) -> EventType {
        self.event_type
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }

    /// Get a field from the event data as a string
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(|v| v.as_str())
    }

    /// Get a field from the event data as an i64
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.data.get(key).and_then(|v| v.as_i64())
    }

    /// Get a field from the event data
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    /// Get the data value
    pub fn data(&self) -> &Value {
        &self.data
    }

    // === Builder methods for setting event data fields ===

    /// Set the operation name in the event data
    pub fn with_operation_name(mut self, name: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("operationName".to_string(), Value::String(name));
        }
        self
    }

    /// Set the result in the event data
    pub fn with_result(mut self, result: Value) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("result".to_string(), result);
        }
        self
    }

    /// Set the state key in the event data
    pub fn with_state_key(mut self, key: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("key".to_string(), Value::String(key));
        }
        self
    }

    /// Set the task kind in the event data
    pub fn with_task_kind(mut self, kind: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("kind".to_string(), Value::String(kind));
        }
        self
    }

    /// Set the timer ID in the event data
    pub fn with_timer_id(mut self, timer_id: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("timerId".to_string(), Value::String(timer_id));
        }
        self
    }

    /// Set the promise ID in the event data
    pub fn with_promise_name(mut self, name: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("promiseId".to_string(), Value::String(name));
        }
        self
    }

    /// Set the child workflow name in the event data
    pub fn with_child_workflow_name(mut self, name: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("childWorkflowName".to_string(), Value::String(name));
        }
        self
    }

    /// Set the child workflow kind in the event data
    pub fn with_child_workflow_kind(mut self, kind: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("childWorkflowKind".to_string(), Value::String(kind));
        }
        self
    }

    /// Set the error in the event data
    pub fn with_error(mut self, error: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("error".to_string(), Value::String(error));
        }
        self
    }

    /// Set the error message in the event data
    pub fn with_error_message(mut self, message: String) -> Self {
        if let Value::Object(ref mut map) = self.data {
            map.insert("errorMessage".to_string(), Value::String(message));
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_is_workflow_terminal() {
        assert!(EventType::WorkflowCompleted.is_workflow_terminal());
        assert!(EventType::WorkflowExecutionFailed.is_workflow_terminal());
        assert!(!EventType::WorkflowStarted.is_workflow_terminal());
        assert!(!EventType::WorkflowSuspended.is_workflow_terminal());
        assert!(!EventType::OperationCompleted.is_workflow_terminal());
    }

    #[test]
    fn test_event_type_is_task_terminal() {
        assert!(EventType::TaskCompleted.is_task_terminal());
        assert!(EventType::TaskFailed.is_task_terminal());
        assert!(!EventType::TaskScheduled.is_task_terminal());
    }

    #[test]
    fn test_event_type_is_child_workflow_terminal() {
        assert!(EventType::ChildWorkflowCompleted.is_child_workflow_terminal());
        assert!(EventType::ChildWorkflowFailed.is_child_workflow_terminal());
        assert!(!EventType::ChildWorkflowInitiated.is_child_workflow_terminal());
        assert!(!EventType::ChildWorkflowStarted.is_child_workflow_terminal());
    }

    #[test]
    fn test_event_type_is_promise_terminal() {
        assert!(EventType::PromiseResolved.is_promise_terminal());
        assert!(EventType::PromiseRejected.is_promise_terminal());
        assert!(EventType::PromiseTimeout.is_promise_terminal());
        assert!(!EventType::PromiseCreated.is_promise_terminal());
    }

    #[test]
    fn test_event_type_as_str() {
        assert_eq!(EventType::WorkflowStarted.as_str(), "WORKFLOW_STARTED");
        assert_eq!(
            EventType::OperationCompleted.as_str(),
            "OPERATION_COMPLETED"
        );
        assert_eq!(EventType::TaskScheduled.as_str(), "TASK_SCHEDULED");
    }

    #[test]
    fn test_event_type_serde() {
        let event_type = EventType::OperationCompleted;
        let json = serde_json::to_string(&event_type).unwrap();
        assert_eq!(json, "\"OPERATION_COMPLETED\"");

        let parsed: EventType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event_type);
    }

    #[test]
    fn test_all_event_types_serialize() {
        let event_types = vec![
            EventType::WorkflowStarted,
            EventType::WorkflowCompleted,
            EventType::WorkflowExecutionFailed,
            EventType::WorkflowSuspended,
            EventType::CancellationRequested,
            EventType::OperationCompleted,
            EventType::StateSet,
            EventType::StateCleared,
            EventType::TaskScheduled,
            EventType::TaskCompleted,
            EventType::TaskFailed,
            EventType::PromiseCreated,
            EventType::PromiseResolved,
            EventType::PromiseRejected,
            EventType::PromiseTimeout,
            EventType::ChildWorkflowInitiated,
            EventType::ChildWorkflowStarted,
            EventType::ChildWorkflowCompleted,
            EventType::ChildWorkflowFailed,
            EventType::TimerStarted,
            EventType::TimerFired,
            EventType::TimerCancelled,
        ];

        for event_type in event_types {
            let json = serde_json::to_string(&event_type).unwrap();
            let parsed: EventType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, event_type);
        }
    }

    // ReplayEvent tests

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn test_replay_event_new() {
        let event = ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            serde_json::json!({"operationName": "fetch-user", "result": {"id": 123}}),
            now(),
        );

        assert_eq!(event.sequence_number(), 1);
        assert_eq!(event.event_type(), EventType::OperationCompleted);
        assert_eq!(event.get_string("operationName"), Some("fetch-user"));
    }

    #[test]
    fn test_replay_event_get_methods() {
        let event = ReplayEvent::new(
            5,
            EventType::TaskScheduled,
            serde_json::json!({
                "kind": "payment-task",
                "taskExecutionId": "abc-123",
                "timeout": 30000
            }),
            now(),
        );

        assert_eq!(event.get_string("kind"), Some("payment-task"));
        assert_eq!(event.get_i64("timeout"), Some(30000));
        assert!(event.get("nonexistent").is_none());
    }

    #[test]
    fn test_replay_event_serde() {
        let event = ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            serde_json::json!({"operationName": "test", "result": 42}),
            now(),
        );

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("sequenceNumber"));
        assert!(json.contains("OPERATION_COMPLETED"));

        let parsed: ReplayEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
    }

    #[test]
    fn test_replay_event_builder_operation_name() {
        let event = ReplayEvent::new(
            0,
            EventType::OperationCompleted,
            serde_json::json!({}),
            now(),
        )
        .with_operation_name("fetch-user".to_string());

        assert_eq!(event.get_string("operationName"), Some("fetch-user"));
    }

    #[test]
    fn test_replay_event_builder_result() {
        let event = ReplayEvent::new(
            0,
            EventType::OperationCompleted,
            serde_json::json!({}),
            now(),
        )
        .with_result(serde_json::json!({"id": 123}));

        assert_eq!(event.get("result"), Some(&serde_json::json!({"id": 123})));
    }

    #[test]
    fn test_replay_event_builder_state_key() {
        let event = ReplayEvent::new(0, EventType::StateSet, serde_json::json!({}), now())
            .with_state_key("user-count".to_string());

        assert_eq!(event.get_string("key"), Some("user-count"));
    }

    #[test]
    fn test_replay_event_builder_task_kind() {
        let event = ReplayEvent::new(0, EventType::TaskScheduled, serde_json::json!({}), now())
            .with_task_kind("payment-task".to_string());

        assert_eq!(event.get_string("kind"), Some("payment-task"));
    }

    #[test]
    fn test_replay_event_builder_timer_id() {
        let event = ReplayEvent::new(0, EventType::TimerStarted, serde_json::json!({}), now())
            .with_timer_id("timer-1".to_string());

        assert_eq!(event.get_string("timerId"), Some("timer-1"));
    }

    #[test]
    fn test_replay_event_builder_promise_name() {
        let event = ReplayEvent::new(0, EventType::PromiseCreated, serde_json::json!({}), now())
            .with_promise_name("user-approval".to_string());

        assert_eq!(event.get_string("promiseId"), Some("user-approval"));
    }

    #[test]
    fn test_replay_event_builder_child_workflow() {
        let event = ReplayEvent::new(
            0,
            EventType::ChildWorkflowInitiated,
            serde_json::json!({}),
            now(),
        )
        .with_child_workflow_name("sub-workflow".to_string())
        .with_child_workflow_kind("payment-workflow".to_string());

        assert_eq!(event.get_string("childWorkflowName"), Some("sub-workflow"));
        assert_eq!(
            event.get_string("childWorkflowKind"),
            Some("payment-workflow")
        );
    }

    #[test]
    fn test_replay_event_builder_error() {
        let event = ReplayEvent::new(
            0,
            EventType::WorkflowExecutionFailed,
            serde_json::json!({}),
            now(),
        )
        .with_error("payment-failed".to_string())
        .with_error_message("Insufficient funds".to_string());

        assert_eq!(event.get_string("error"), Some("payment-failed"));
        assert_eq!(event.get_string("errorMessage"), Some("Insufficient funds"));
    }
}
