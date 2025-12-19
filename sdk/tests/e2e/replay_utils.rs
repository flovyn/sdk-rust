//! Replay test utilities for E2E testing
//!
//! Provides helpers for converting server events to replay events
//! and creating test contexts for replay validation.

use chrono::{DateTime, Utc};
use flovyn_sdk::workflow::context::WorkflowContext;
use flovyn_sdk::workflow::context_impl::WorkflowContextImpl;
use flovyn_sdk::workflow::event::{EventType, ReplayEvent};
use flovyn_sdk::workflow::recorder::CommandCollector;
use uuid::Uuid;

use super::harness::WorkflowEventResponse;

/// Convert a server event type string to SDK EventType.
pub fn parse_event_type(event_type: &str) -> EventType {
    match event_type {
        "WorkflowStarted" | "WORKFLOW_STARTED" => EventType::WorkflowStarted,
        "WorkflowCompleted" | "WORKFLOW_COMPLETED" => EventType::WorkflowCompleted,
        "WorkflowFailed"
        | "WORKFLOW_FAILED"
        | "WorkflowExecutionFailed"
        | "WORKFLOW_EXECUTION_FAILED" => EventType::WorkflowExecutionFailed,
        "WorkflowSuspended" | "WORKFLOW_SUSPENDED" => EventType::WorkflowSuspended,
        "CancellationRequested" | "CANCELLATION_REQUESTED" => EventType::CancellationRequested,
        "OperationCompleted" | "OPERATION_COMPLETED" => EventType::OperationCompleted,
        "StateSet" | "STATE_SET" => EventType::StateSet,
        "StateCleared" | "STATE_CLEARED" => EventType::StateCleared,
        "TaskScheduled" | "TASK_SCHEDULED" => EventType::TaskScheduled,
        "TaskCompleted" | "TASK_COMPLETED" => EventType::TaskCompleted,
        "TaskFailed" | "TASK_FAILED" => EventType::TaskFailed,
        "PromiseCreated" | "PROMISE_CREATED" => EventType::PromiseCreated,
        "PromiseResolved" | "PROMISE_RESOLVED" => EventType::PromiseResolved,
        "PromiseRejected" | "PROMISE_REJECTED" => EventType::PromiseRejected,
        "PromiseTimeout" | "PROMISE_TIMEOUT" => EventType::PromiseTimeout,
        "ChildWorkflowInitiated" | "CHILD_WORKFLOW_INITIATED" => EventType::ChildWorkflowInitiated,
        "ChildWorkflowStarted" | "CHILD_WORKFLOW_STARTED" => EventType::ChildWorkflowStarted,
        "ChildWorkflowCompleted" | "CHILD_WORKFLOW_COMPLETED" => EventType::ChildWorkflowCompleted,
        "ChildWorkflowFailed" | "CHILD_WORKFLOW_FAILED" => EventType::ChildWorkflowFailed,
        "TimerStarted" | "TIMER_STARTED" => EventType::TimerStarted,
        "TimerFired" | "TIMER_FIRED" => EventType::TimerFired,
        "TimerCancelled" | "TIMER_CANCELLED" => EventType::TimerCancelled,
        _ => panic!("Unknown event type: {}", event_type),
    }
}

/// Convert a WorkflowEventResponse to a ReplayEvent.
pub fn to_replay_event(event: &WorkflowEventResponse) -> ReplayEvent {
    let event_type = parse_event_type(&event.event_type);
    let timestamp = event
        .created_at
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());

    ReplayEvent::new(
        event.sequence_number,
        event_type,
        event.data.clone(),
        timestamp,
    )
}

/// Convert a list of server events to replay events.
pub fn to_replay_events(events: &[WorkflowEventResponse]) -> Vec<ReplayEvent> {
    events.iter().map(to_replay_event).collect()
}

/// Create a WorkflowContextImpl for replay testing from server events.
pub fn create_replay_context(
    workflow_execution_id: Uuid,
    tenant_id: Uuid,
    input: serde_json::Value,
    events: &[WorkflowEventResponse],
) -> WorkflowContextImpl<CommandCollector> {
    let replay_events = to_replay_events(events);

    WorkflowContextImpl::new(
        workflow_execution_id,
        tenant_id,
        input,
        CommandCollector::new(),
        replay_events,
        Utc::now().timestamp_millis(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_event_type_pascal_case() {
        assert_eq!(
            parse_event_type("WorkflowStarted"),
            EventType::WorkflowStarted
        );
        assert_eq!(parse_event_type("TaskScheduled"), EventType::TaskScheduled);
        assert_eq!(parse_event_type("TaskCompleted"), EventType::TaskCompleted);
        assert_eq!(
            parse_event_type("ChildWorkflowInitiated"),
            EventType::ChildWorkflowInitiated
        );
        assert_eq!(parse_event_type("TimerStarted"), EventType::TimerStarted);
    }

    #[test]
    fn test_parse_event_type_screaming_snake_case() {
        assert_eq!(
            parse_event_type("WORKFLOW_STARTED"),
            EventType::WorkflowStarted
        );
        assert_eq!(parse_event_type("TASK_SCHEDULED"), EventType::TaskScheduled);
        assert_eq!(parse_event_type("TASK_COMPLETED"), EventType::TaskCompleted);
        assert_eq!(
            parse_event_type("CHILD_WORKFLOW_INITIATED"),
            EventType::ChildWorkflowInitiated
        );
        assert_eq!(parse_event_type("TIMER_STARTED"), EventType::TimerStarted);
    }

    #[test]
    fn test_to_replay_event() {
        let event = WorkflowEventResponse {
            sequence_number: 1,
            event_type: "TaskScheduled".to_string(),
            data: json!({ "taskType": "process-item", "taskExecutionId": "task-1" }),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let replay_event = to_replay_event(&event);
        assert_eq!(replay_event.sequence_number(), 1);
        assert_eq!(replay_event.event_type(), EventType::TaskScheduled);
        assert_eq!(replay_event.get_string("taskType"), Some("process-item"));
    }

    #[test]
    fn test_to_replay_events() {
        let events = vec![
            WorkflowEventResponse {
                sequence_number: 1,
                event_type: "WorkflowStarted".to_string(),
                data: json!({ "input": {} }),
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
            WorkflowEventResponse {
                sequence_number: 2,
                event_type: "TaskScheduled".to_string(),
                data: json!({ "taskType": "task-A" }),
                created_at: "2024-01-01T00:00:01Z".to_string(),
            },
        ];

        let replay_events = to_replay_events(&events);
        assert_eq!(replay_events.len(), 2);
        assert_eq!(replay_events[0].event_type(), EventType::WorkflowStarted);
        assert_eq!(replay_events[1].event_type(), EventType::TaskScheduled);
    }

    #[test]
    fn test_create_replay_context() {
        let events = vec![
            WorkflowEventResponse {
                sequence_number: 1,
                event_type: "WorkflowStarted".to_string(),
                data: json!({ "input": { "value": 42 } }),
                created_at: "2024-01-01T00:00:00Z".to_string(),
            },
            WorkflowEventResponse {
                sequence_number: 2,
                event_type: "TaskScheduled".to_string(),
                data: json!({ "taskType": "process", "taskExecutionId": "task-1" }),
                created_at: "2024-01-01T00:00:01Z".to_string(),
            },
        ];

        let ctx = create_replay_context(
            Uuid::new_v4(),
            Uuid::new_v4(),
            json!({ "value": 42 }),
            &events,
        );

        // Context should be created successfully
        assert!(!ctx.workflow_execution_id().is_nil());
    }
}
