//! Shared replay corpus tests
//!
//! These tests validate the SDK against shared JSON replay scenarios
//! that can be used for cross-language validation (Rust <-> Kotlin)

use chrono::Utc;
use flovyn_sdk::workflow::event::{EventType, ReplayEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// A replay corpus test scenario
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayScenario {
    pub name: String,
    pub description: String,
    pub workflow_execution_id: String,
    pub org_id: String,
    pub workflow_kind: String,
    pub input: Value,
    pub events: Vec<ScenarioEvent>,
    pub expected_commands: Vec<ExpectedCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioEvent {
    pub sequence_number: i32,
    pub event_type: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedCommand {
    #[serde(rename = "type")]
    pub command_type: String,
    #[serde(flatten)]
    pub fields: Value,
}

impl ScenarioEvent {
    /// Convert to SDK ReplayEvent
    pub fn to_replay_event(&self) -> ReplayEvent {
        let event_type = match self.event_type.as_str() {
            "WorkflowStarted" => EventType::WorkflowStarted,
            "WorkflowCompleted" => EventType::WorkflowCompleted,
            "WorkflowFailed" | "WorkflowExecutionFailed" => EventType::WorkflowExecutionFailed,
            "WorkflowSuspended" => EventType::WorkflowSuspended,
            "CancellationRequested" => EventType::CancellationRequested,
            "OperationCompleted" => EventType::OperationCompleted,
            "StateSet" => EventType::StateSet,
            "StateCleared" => EventType::StateCleared,
            "TaskScheduled" => EventType::TaskScheduled,
            "TaskCompleted" => EventType::TaskCompleted,
            "TaskFailed" => EventType::TaskFailed,
            "PromiseCreated" => EventType::PromiseCreated,
            "PromiseResolved" => EventType::PromiseResolved,
            "PromiseRejected" => EventType::PromiseRejected,
            "PromiseTimeout" => EventType::PromiseTimeout,
            "ChildWorkflowInitiated" => EventType::ChildWorkflowInitiated,
            "ChildWorkflowStarted" => EventType::ChildWorkflowStarted,
            "ChildWorkflowCompleted" => EventType::ChildWorkflowCompleted,
            "ChildWorkflowFailed" => EventType::ChildWorkflowFailed,
            "TimerStarted" => EventType::TimerStarted,
            "TimerFired" => EventType::TimerFired,
            "TimerCancelled" => EventType::TimerCancelled,
            _ => panic!("Unknown event type: {}", self.event_type),
        };

        ReplayEvent::new(
            self.sequence_number,
            event_type,
            self.data.clone(),
            Utc::now(),
        )
    }
}

/// Load all replay scenarios from the corpus directory
/// Note: Skips determinism-*.json files which are handled by TCK determinism_corpus tests
pub fn load_replay_corpus() -> Vec<ReplayScenario> {
    let corpus_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("shared")
        .join("replay-corpus");

    let mut scenarios = Vec::new();

    if corpus_dir.exists() {
        for entry in fs::read_dir(&corpus_dir).expect("Failed to read corpus directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            if let Some(name) = path.file_name() {
                let name_str = name.to_string_lossy();
                // Skip determinism-*.json files - they're handled by TCK determinism_corpus tests
                if name_str.starts_with("determinism-") {
                    continue;
                }
                if name_str.ends_with(".json") {
                    let content = fs::read_to_string(&path)
                        .unwrap_or_else(|_| panic!("Failed to read {:?}", path));
                    let scenario: ReplayScenario = serde_json::from_str(&content)
                        .unwrap_or_else(|_| panic!("Failed to parse {:?}", path));
                    scenarios.push(scenario);
                }
            }
        }
    }

    scenarios
}

#[test]
fn test_load_replay_corpus() {
    let scenarios = load_replay_corpus();
    assert!(!scenarios.is_empty(), "Should have at least one scenario");

    // Verify we can load expected scenarios
    let scenario_names: Vec<_> = scenarios.iter().map(|s| s.name.as_str()).collect();
    assert!(scenario_names.contains(&"simple-operation"));
    assert!(scenario_names.contains(&"multiple-operations"));
    assert!(scenario_names.contains(&"state-management"));
    assert!(scenario_names.contains(&"task-scheduling"));
    assert!(scenario_names.contains(&"timer"));
    assert!(scenario_names.contains(&"promise"));
    assert!(scenario_names.contains(&"child-workflow"));
}

#[test]
fn test_simple_operation_scenario() {
    let scenarios = load_replay_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "simple-operation")
        .expect("simple-operation scenario not found");

    assert_eq!(scenario.workflow_kind, "simple-workflow");
    assert_eq!(scenario.events.len(), 3);
    assert_eq!(scenario.expected_commands.len(), 2);

    // Verify events can be converted
    for event in &scenario.events {
        let _replay_event = event.to_replay_event();
    }
}

#[test]
fn test_multiple_operations_scenario() {
    let scenarios = load_replay_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "multiple-operations")
        .expect("multiple-operations scenario not found");

    assert_eq!(scenario.workflow_kind, "multi-operation-workflow");
    assert_eq!(scenario.events.len(), 5);
    assert_eq!(scenario.expected_commands.len(), 4);
}

#[test]
fn test_state_management_scenario() {
    let scenarios = load_replay_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "state-management")
        .expect("state-management scenario not found");

    assert_eq!(scenario.workflow_kind, "stateful-workflow");

    // Verify state events
    let state_set_events: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "StateSet")
        .collect();
    assert_eq!(state_set_events.len(), 3);

    let state_cleared_events: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "StateCleared")
        .collect();
    assert_eq!(state_cleared_events.len(), 1);
}

#[test]
fn test_task_scheduling_scenario() {
    let scenarios = load_replay_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "task-scheduling")
        .expect("task-scheduling scenario not found");

    assert_eq!(scenario.workflow_kind, "task-workflow");

    // Verify task events
    let task_scheduled: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .collect();
    assert_eq!(task_scheduled.len(), 2);

    let task_completed: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskCompleted")
        .collect();
    assert_eq!(task_completed.len(), 2);
}

#[test]
fn test_timer_scenario() {
    let scenarios = load_replay_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "timer")
        .expect("timer scenario not found");

    assert_eq!(scenario.workflow_kind, "timer-workflow");

    // Verify timer events
    let timer_started: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TimerStarted")
        .collect();
    assert_eq!(timer_started.len(), 1);

    let timer_fired: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TimerFired")
        .collect();
    assert_eq!(timer_fired.len(), 1);
}

#[test]
fn test_promise_scenario() {
    let scenarios = load_replay_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "promise")
        .expect("promise scenario not found");

    assert_eq!(scenario.workflow_kind, "promise-workflow");

    // Verify promise events
    let promise_created: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "PromiseCreated")
        .collect();
    assert_eq!(promise_created.len(), 1);

    let promise_resolved: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "PromiseResolved")
        .collect();
    assert_eq!(promise_resolved.len(), 1);
}

#[test]
fn test_child_workflow_scenario() {
    let scenarios = load_replay_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "child-workflow")
        .expect("child-workflow scenario not found");

    assert_eq!(scenario.workflow_kind, "parent-workflow");

    // Verify child workflow events
    let initiated: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "ChildWorkflowInitiated")
        .collect();
    assert_eq!(initiated.len(), 2);

    let started: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "ChildWorkflowStarted")
        .collect();
    assert_eq!(started.len(), 2);

    let completed: Vec<_> = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "ChildWorkflowCompleted")
        .collect();
    assert_eq!(completed.len(), 2);
}

#[test]
fn test_event_type_conversion() {
    let event_types = [
        "WorkflowStarted",
        "WorkflowCompleted",
        "WorkflowFailed",
        "WorkflowSuspended",
        "CancellationRequested",
        "OperationCompleted",
        "StateSet",
        "StateCleared",
        "TaskScheduled",
        "TaskCompleted",
        "TaskFailed",
        "PromiseCreated",
        "PromiseResolved",
        "PromiseTimeout",
        "ChildWorkflowInitiated",
        "ChildWorkflowStarted",
        "ChildWorkflowCompleted",
        "ChildWorkflowFailed",
        "TimerStarted",
        "TimerFired",
        "TimerCancelled",
    ];

    for event_type in event_types {
        let scenario_event = ScenarioEvent {
            sequence_number: 1,
            event_type: event_type.to_string(),
            data: serde_json::json!({}),
        };
        let _ = scenario_event.to_replay_event(); // Should not panic
    }
}

#[test]
fn test_all_scenarios_have_required_fields() {
    let scenarios = load_replay_corpus();

    for scenario in scenarios {
        assert!(!scenario.name.is_empty(), "Name should not be empty");
        assert!(
            !scenario.description.is_empty(),
            "Description should not be empty"
        );
        assert!(
            !scenario.workflow_execution_id.is_empty(),
            "Workflow execution ID should not be empty"
        );
        assert!(!scenario.org_id.is_empty(), "Org ID should not be empty");
        assert!(
            !scenario.workflow_kind.is_empty(),
            "Workflow kind should not be empty"
        );
        assert!(!scenario.events.is_empty(), "Events should not be empty");
        assert!(
            !scenario.expected_commands.is_empty(),
            "Expected commands should not be empty"
        );

        // First event should always be WorkflowStarted
        assert_eq!(
            scenario.events[0].event_type, "WorkflowStarted",
            "First event should be WorkflowStarted for scenario: {}",
            scenario.name
        );

        // Last event should be a terminal event
        let last_event = scenario.events.last().unwrap();
        assert!(
            [
                "WorkflowCompleted",
                "WorkflowFailed",
                "WorkflowSuspended",
                "WorkflowExecutionFailed"
            ]
            .contains(&last_event.event_type.as_str()),
            "Last event should be terminal for scenario: {}",
            scenario.name
        );
    }
}
