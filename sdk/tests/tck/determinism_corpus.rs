//! Determinism corpus tests for sequence-based replay
//!
//! These tests validate determinism violation detection using JSON corpus files.
//! The corpus files define replay scenarios and expected violations.

use chrono::Utc;
use flovyn_sdk::error::{DeterminismViolationError, FlovynError};
use flovyn_sdk::workflow::context::WorkflowContext;
use flovyn_sdk::workflow::context_impl::WorkflowContextImpl;
use flovyn_sdk::workflow::event::{EventType, ReplayEvent};
use flovyn_sdk::workflow::recorder::CommandCollector;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// A determinism test scenario
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeterminismScenario {
    pub name: String,
    pub description: String,
    pub workflow_execution_id: String,
    pub tenant_id: String,
    pub workflow_kind: String,
    pub input: Value,
    pub events: Vec<ScenarioEvent>,
    #[serde(default)]
    pub expected_commands: Vec<ExpectedCommand>,
    #[serde(default)]
    pub replay_commands: Vec<ReplayCommand>,
    pub determinism_test: DeterminismTest,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayCommand {
    #[serde(rename = "type")]
    pub command_type: String,
    #[serde(flatten)]
    pub fields: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeterminismTest {
    #[serde(rename = "type")]
    pub test_type: String,
    #[serde(default)]
    pub violation_type: Option<String>,
    pub description: String,
    #[serde(default)]
    pub expected_error_contains: Vec<String>,
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
            "TaskCancelled" => EventType::TaskCancelled,
            "PromiseCreated" => EventType::PromiseCreated,
            "PromiseResolved" => EventType::PromiseResolved,
            "PromiseRejected" => EventType::PromiseRejected,
            "PromiseTimeout" => EventType::PromiseTimeout,
            "ChildWorkflowInitiated" => EventType::ChildWorkflowInitiated,
            "ChildWorkflowStarted" => EventType::ChildWorkflowStarted,
            "ChildWorkflowCompleted" => EventType::ChildWorkflowCompleted,
            "ChildWorkflowFailed" => EventType::ChildWorkflowFailed,
            "ChildWorkflowCancelled" => EventType::ChildWorkflowCancelled,
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

/// Load all determinism scenarios from the corpus directory
pub fn load_determinism_corpus() -> Vec<DeterminismScenario> {
    let corpus_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("shared")
        .join("replay-corpus");

    let mut scenarios = Vec::new();

    if corpus_dir.exists() {
        for entry in fs::read_dir(&corpus_dir).expect("Failed to read corpus directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            // Load determinism-*.json and parallel-*.json files
            if let Some(name) = path.file_name() {
                let name_str = name.to_string_lossy();
                if (name_str.starts_with("determinism-") || name_str.starts_with("parallel-"))
                    && name_str.ends_with(".json")
                {
                    let content = fs::read_to_string(&path)
                        .unwrap_or_else(|_| panic!("Failed to read {:?}", path));
                    let scenario: DeterminismScenario = serde_json::from_str(&content)
                        .unwrap_or_else(|e| panic!("Failed to parse {:?}: {}", path, e));
                    scenarios.push(scenario);
                }
            }
        }
    }

    scenarios
}

/// Create a WorkflowContextImpl from a scenario for testing
fn create_test_context(scenario: &DeterminismScenario) -> WorkflowContextImpl<CommandCollector> {
    let workflow_execution_id =
        Uuid::parse_str(&scenario.workflow_execution_id).expect("Invalid workflow_execution_id");
    let tenant_id = Uuid::parse_str(&scenario.tenant_id).expect("Invalid tenant_id");

    let replay_events: Vec<ReplayEvent> = scenario
        .events
        .iter()
        .map(|e| e.to_replay_event())
        .collect();

    WorkflowContextImpl::new(
        workflow_execution_id,
        tenant_id,
        scenario.input.clone(),
        CommandCollector::new(),
        replay_events,
        chrono::Utc::now().timestamp_millis(),
    )
}

/// Execute a replay command against the context and return the result
async fn execute_replay_command(
    ctx: &WorkflowContextImpl<CommandCollector>,
    command: &ReplayCommand,
) -> Result<Value, FlovynError> {
    match command.command_type.as_str() {
        "ScheduleTask" => {
            let task_type = command
                .fields
                .get("kind")
                .and_then(|v| v.as_str())
                .expect("ScheduleTask requires taskType");
            let input = command.fields.get("input").cloned().unwrap_or(Value::Null);
            ctx.schedule_raw(task_type, input).await
        }
        "ScheduleChildWorkflow" => {
            let name = command
                .fields
                .get("workflowName")
                .and_then(|v| v.as_str())
                .expect("ScheduleChildWorkflow requires workflowName");
            let kind = command
                .fields
                .get("workflowKind")
                .and_then(|v| v.as_str())
                .expect("ScheduleChildWorkflow requires workflowKind");
            let input = command.fields.get("input").cloned().unwrap_or(Value::Null);
            ctx.schedule_workflow_raw(name, kind, input).await
        }
        "StartTimer" => {
            let timer_id = command
                .fields
                .get("timerId")
                .and_then(|v| v.as_str())
                .expect("StartTimer requires timerId");
            let duration_ms = command
                .fields
                .get("duration")
                .and_then(|v| v.as_u64())
                .unwrap_or(1000);
            // Use sleep which uses the timer_id from the internal counter
            ctx.sleep(std::time::Duration::from_millis(duration_ms))
                .await
                .map(|_| Value::Null)?;
            // Note: Timer ID validation happens through sleep() call
            // but we may need a more direct timer API for full control
            Ok(Value::String(timer_id.to_string()))
        }
        "RecordOperation" => {
            let name = command
                .fields
                .get("operationName")
                .and_then(|v| v.as_str())
                .expect("RecordOperation requires operationName");
            ctx.run_raw(name, serde_json::json!({})).await
        }
        "CreatePromise" => {
            let name = command
                .fields
                .get("promiseName")
                .and_then(|v| v.as_str())
                .expect("CreatePromise requires promiseName");
            let timeout_ms = command.fields.get("timeout").and_then(|v| v.as_u64());
            if let Some(ms) = timeout_ms {
                ctx.promise_with_timeout_raw(name, std::time::Duration::from_millis(ms))
                    .await
            } else {
                ctx.promise_raw(name).await
            }
        }
        "SetState" => {
            let key = command
                .fields
                .get("key")
                .and_then(|v| v.as_str())
                .expect("SetState requires key");
            let value = command.fields.get("value").cloned().unwrap_or(Value::Null);
            ctx.set_raw(key, value.clone()).await?;
            Ok(value)
        }
        "ClearState" => {
            let key = command
                .fields
                .get("key")
                .and_then(|v| v.as_str())
                .expect("ClearState requires key");
            ctx.clear(key).await?;
            Ok(Value::Null)
        }
        _ => panic!("Unknown replay command type: {}", command.command_type),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_load_determinism_corpus() {
    let scenarios = load_determinism_corpus();
    assert!(
        !scenarios.is_empty(),
        "Should have at least one determinism scenario"
    );

    // Verify we have the expected determinism scenarios
    let scenario_names: Vec<_> = scenarios.iter().map(|s| s.name.as_str()).collect();
    assert!(
        scenario_names.contains(&"determinism-task-loop"),
        "Missing determinism-task-loop scenario"
    );
    assert!(
        scenario_names.contains(&"determinism-violation-task-type"),
        "Missing determinism-violation-task-type scenario"
    );
    assert!(
        scenario_names.contains(&"determinism-violation-task-order"),
        "Missing determinism-violation-task-order scenario"
    );
    assert!(
        scenario_names.contains(&"determinism-child-workflow-loop"),
        "Missing determinism-child-workflow-loop scenario"
    );
    assert!(
        scenario_names.contains(&"determinism-violation-child-name"),
        "Missing determinism-violation-child-name scenario"
    );
    assert!(
        scenario_names.contains(&"determinism-violation-child-kind"),
        "Missing determinism-violation-child-kind scenario"
    );
    assert!(
        scenario_names.contains(&"determinism-mixed-commands"),
        "Missing determinism-mixed-commands scenario"
    );

    // Verify we have the expected parallel execution scenarios
    assert!(
        scenario_names.contains(&"parallel-two-tasks"),
        "Missing parallel-two-tasks scenario"
    );
    assert!(
        scenario_names.contains(&"parallel-task-and-timer"),
        "Missing parallel-task-and-timer scenario"
    );
    assert!(
        scenario_names.contains(&"parallel-three-tasks-one-fails"),
        "Missing parallel-three-tasks-one-fails scenario"
    );
    assert!(
        scenario_names.contains(&"parallel-select-first-wins"),
        "Missing parallel-select-first-wins scenario"
    );
    assert!(
        scenario_names.contains(&"parallel-timeout-success"),
        "Missing parallel-timeout-success scenario"
    );
    assert!(
        scenario_names.contains(&"parallel-timeout-expires"),
        "Missing parallel-timeout-expires scenario"
    );
    assert!(
        scenario_names.contains(&"parallel-mixed-operations"),
        "Missing parallel-mixed-operations scenario"
    );
}

#[test]
fn test_all_determinism_scenarios_have_required_fields() {
    let scenarios = load_determinism_corpus();

    for scenario in scenarios {
        assert!(!scenario.name.is_empty(), "Name should not be empty");
        assert!(
            !scenario.description.is_empty(),
            "Description should not be empty for {}",
            scenario.name
        );
        assert!(
            !scenario.workflow_execution_id.is_empty(),
            "Workflow execution ID should not be empty for {}",
            scenario.name
        );
        assert!(
            !scenario.tenant_id.is_empty(),
            "Tenant ID should not be empty for {}",
            scenario.name
        );
        assert!(
            !scenario.workflow_kind.is_empty(),
            "Workflow kind should not be empty for {}",
            scenario.name
        );
        assert!(
            !scenario.events.is_empty(),
            "Events should not be empty for {}",
            scenario.name
        );

        // First event should always be WorkflowStarted
        assert_eq!(
            scenario.events[0].event_type, "WorkflowStarted",
            "First event should be WorkflowStarted for scenario: {}",
            scenario.name
        );

        // Determinism test should have a valid type
        assert!(
            scenario.determinism_test.test_type == "valid_replay"
                || scenario.determinism_test.test_type == "expect_violation",
            "Invalid test_type for scenario: {}",
            scenario.name
        );

        // Violation scenarios should specify violation type and expected errors
        if scenario.determinism_test.test_type == "expect_violation" {
            assert!(
                scenario.determinism_test.violation_type.is_some(),
                "Violation scenarios must specify violation_type for: {}",
                scenario.name
            );
            assert!(
                !scenario.replay_commands.is_empty(),
                "Violation scenarios must specify replay_commands for: {}",
                scenario.name
            );
        }

        // Valid replay scenarios should specify expected commands
        if scenario.determinism_test.test_type == "valid_replay" {
            assert!(
                !scenario.expected_commands.is_empty(),
                "Valid replay scenarios should specify expected_commands for: {}",
                scenario.name
            );
        }
    }
}

#[test]
fn test_event_type_conversion_for_determinism_scenarios() {
    let scenarios = load_determinism_corpus();

    for scenario in &scenarios {
        for event in &scenario.events {
            // This will panic if the event type is unknown
            let _replay_event = event.to_replay_event();
        }
    }
}

#[tokio::test]
async fn test_determinism_task_loop_valid_replay() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-task-loop")
        .expect("determinism-task-loop scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    // Create context with replay events
    let ctx = create_test_context(scenario);

    // Verify context can be created with the scenario events
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );

    // For valid replay, all events should parse correctly
    assert_eq!(scenario.events.len(), 8);

    // Verify task events are correctly filtered
    let task_scheduled_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    assert_eq!(task_scheduled_count, 3, "Expected 3 TaskScheduled events");
}

#[tokio::test]
async fn test_determinism_violation_task_type() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-violation-task-type")
        .expect("determinism-violation-task-type scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "expect_violation",
        "Expected expect_violation test type"
    );

    let ctx = create_test_context(scenario);

    // Execute the replay commands - should cause a violation
    assert!(!scenario.replay_commands.is_empty());
    let result = execute_replay_command(&ctx, &scenario.replay_commands[0]).await;

    assert!(result.is_err(), "Expected determinism violation error");
    let err = result.unwrap_err();

    // Verify it's a determinism violation
    match &err {
        FlovynError::DeterminismViolation(violation) => match violation {
            DeterminismViolationError::TaskTypeMismatch {
                expected, actual, ..
            } => {
                assert_eq!(expected, "original-task");
                assert_eq!(actual, "changed-task");
            }
            _ => panic!("Expected TaskTypeMismatch violation, got {:?}", violation),
        },
        _ => panic!("Expected DeterminismViolation error, got {:?}", err),
    }

    // Verify error message contains expected strings
    let err_str = err.to_string();
    for expected in &scenario.determinism_test.expected_error_contains {
        assert!(
            err_str.contains(expected),
            "Error '{}' should contain '{}'",
            err_str,
            expected
        );
    }
}

#[tokio::test]
async fn test_determinism_violation_task_order() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-violation-task-order")
        .expect("determinism-violation-task-order scenario not found");

    let ctx = create_test_context(scenario);

    // First replay command schedules task-B when history expects task-A
    let result = execute_replay_command(&ctx, &scenario.replay_commands[0]).await;

    assert!(result.is_err(), "Expected determinism violation error");
    let err = result.unwrap_err();

    match &err {
        FlovynError::DeterminismViolation(DeterminismViolationError::TaskTypeMismatch {
            expected,
            actual,
            ..
        }) => {
            assert_eq!(expected, "task-A");
            assert_eq!(actual, "task-B");
        }
        _ => panic!("Expected TaskTypeMismatch violation, got {:?}", err),
    }
}

#[tokio::test]
async fn test_determinism_child_workflow_loop_valid_replay() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-child-workflow-loop")
        .expect("determinism-child-workflow-loop scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify child workflow events are correctly identified
    let child_workflow_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "ChildWorkflowInitiated")
        .count();
    assert_eq!(
        child_workflow_count, 3,
        "Expected 3 ChildWorkflowInitiated events"
    );

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}

#[tokio::test]
async fn test_determinism_violation_child_name() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-violation-child-name")
        .expect("determinism-violation-child-name scenario not found");

    let ctx = create_test_context(scenario);

    // Execute the replay command - should cause a name mismatch violation
    let result = execute_replay_command(&ctx, &scenario.replay_commands[0]).await;

    assert!(result.is_err(), "Expected determinism violation error");
    let err = result.unwrap_err();

    match &err {
        FlovynError::DeterminismViolation(DeterminismViolationError::ChildWorkflowMismatch {
            field,
            expected,
            actual,
            ..
        }) => {
            assert_eq!(field, "name");
            assert_eq!(expected, "original-child");
            assert_eq!(actual, "renamed-child");
        }
        _ => panic!("Expected ChildWorkflowMismatch violation, got {:?}", err),
    }
}

#[tokio::test]
async fn test_determinism_violation_child_kind() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-violation-child-kind")
        .expect("determinism-violation-child-kind scenario not found");

    let ctx = create_test_context(scenario);

    // Execute the replay command - should cause a kind mismatch violation
    let result = execute_replay_command(&ctx, &scenario.replay_commands[0]).await;

    assert!(result.is_err(), "Expected determinism violation error");
    let err = result.unwrap_err();

    match &err {
        FlovynError::DeterminismViolation(DeterminismViolationError::ChildWorkflowMismatch {
            field,
            expected,
            actual,
            ..
        }) => {
            assert_eq!(field, "kind");
            assert_eq!(expected, "original-kind");
            assert_eq!(actual, "changed-kind");
        }
        _ => panic!("Expected ChildWorkflowMismatch violation, got {:?}", err),
    }
}

#[tokio::test]
async fn test_determinism_mixed_commands_valid_replay() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-mixed-commands")
        .expect("determinism-mixed-commands scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify different event types are present
    let task_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    let timer_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TimerStarted")
        .count();
    let child_workflow_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "ChildWorkflowInitiated")
        .count();

    assert_eq!(task_count, 2, "Expected 2 TaskScheduled events");
    assert_eq!(timer_count, 1, "Expected 1 TimerStarted event");
    assert_eq!(
        child_workflow_count, 1,
        "Expected 1 ChildWorkflowInitiated event"
    );

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}

/// Run all determinism scenarios
#[tokio::test]
async fn test_all_determinism_scenarios() {
    let scenarios = load_determinism_corpus();

    for scenario in scenarios {
        println!("Testing scenario: {}", scenario.name);

        let ctx = create_test_context(&scenario);

        match scenario.determinism_test.test_type.as_str() {
            "valid_replay" => {
                // For valid replay scenarios, verify context creation succeeds
                // and events are properly parsed
                assert_eq!(
                    ctx.workflow_execution_id().to_string(),
                    scenario.workflow_execution_id,
                    "Context creation should succeed for valid replay scenario: {}",
                    scenario.name
                );
            }
            "expect_violation" => {
                // For violation scenarios, execute replay commands and verify error
                if !scenario.replay_commands.is_empty() {
                    let result = execute_replay_command(&ctx, &scenario.replay_commands[0]).await;
                    assert!(
                        result.is_err(),
                        "Expected violation in scenario: {}",
                        scenario.name
                    );

                    let err = result.unwrap_err();
                    assert!(
                        matches!(err, FlovynError::DeterminismViolation(_)),
                        "Expected DeterminismViolation in scenario: {}, got {:?}",
                        scenario.name,
                        err
                    );
                }
            }
            _ => panic!(
                "Unknown test_type: {} in scenario: {}",
                scenario.determinism_test.test_type, scenario.name
            ),
        }
    }
}

#[tokio::test]
async fn test_determinism_loop_shortened_valid_replay() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "determinism-loop-shortened")
        .expect("determinism-loop-shortened scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    // Create context with replay events
    let ctx = create_test_context(scenario);

    // Verify context can be created with the scenario events
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );

    // Verify 7 events (WorkflowStarted + 3 task iterations, but only 2 will be replayed)
    assert_eq!(scenario.events.len(), 7);

    // Verify task events are correctly filtered
    let task_scheduled_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    assert_eq!(task_scheduled_count, 3, "Expected 3 TaskScheduled events");

    // Execute replay commands - only 2 tasks (shortened from 3)
    for (i, replay_cmd) in scenario.replay_commands.iter().enumerate() {
        let result = execute_replay_command(&ctx, replay_cmd).await;
        assert!(
            result.is_ok(),
            "Task {} should replay successfully: {:?}",
            i,
            result
        );
        // Verify we get the correct result
        let expected_result = serde_json::json!({"value": i + 1});
        assert_eq!(
            result.unwrap(),
            expected_result,
            "Task {} result mismatch",
            i
        );
    }

    // Workflow completes without accessing task-3
    // Events 5-6 are orphaned, but this is valid behavior
}

// ============================================================================
// Parallel Execution Tests
// ============================================================================

#[tokio::test]
async fn test_parallel_two_tasks() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "parallel-two-tasks")
        .expect("parallel-two-tasks scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify two TaskScheduled events
    let task_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    assert_eq!(task_count, 2, "Expected 2 parallel TaskScheduled events");

    // Execute replay commands
    for replay_cmd in &scenario.replay_commands {
        let result = execute_replay_command(&ctx, replay_cmd).await;
        assert!(
            result.is_ok(),
            "Parallel task should replay successfully: {:?}",
            result
        );
    }
}

#[tokio::test]
async fn test_parallel_task_and_timer() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "parallel-task-and-timer")
        .expect("parallel-task-and-timer scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify we have both task and timer events
    let task_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    let timer_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TimerStarted")
        .count();
    assert_eq!(task_count, 1, "Expected 1 TaskScheduled event");
    assert_eq!(timer_count, 1, "Expected 1 TimerStarted event");

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}

#[tokio::test]
async fn test_parallel_three_tasks_one_fails() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "parallel-three-tasks-one-fails")
        .expect("parallel-three-tasks-one-fails scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify three TaskScheduled events
    let task_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    assert_eq!(task_count, 3, "Expected 3 parallel TaskScheduled events");

    // Verify one TaskFailed event
    let failed_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskFailed")
        .count();
    assert_eq!(failed_count, 1, "Expected 1 TaskFailed event");

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}

#[tokio::test]
async fn test_parallel_select_first_wins() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "parallel-select-first-wins")
        .expect("parallel-select-first-wins scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify two TaskScheduled events (racing)
    let task_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    assert_eq!(task_count, 2, "Expected 2 racing TaskScheduled events");

    // Verify one TaskCancelled event (loser)
    let cancelled_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskCancelled")
        .count();
    assert_eq!(cancelled_count, 1, "Expected 1 TaskCancelled event");

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}

#[tokio::test]
async fn test_parallel_timeout_success() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "parallel-timeout-success")
        .expect("parallel-timeout-success scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify task and timer events
    let task_completed = scenario
        .events
        .iter()
        .any(|e| e.event_type == "TaskCompleted");
    let timer_cancelled = scenario
        .events
        .iter()
        .any(|e| e.event_type == "TimerCancelled");

    assert!(task_completed, "Expected TaskCompleted event");
    assert!(
        timer_cancelled,
        "Expected TimerCancelled event (timeout cancelled)"
    );

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}

#[tokio::test]
async fn test_parallel_timeout_expires() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "parallel-timeout-expires")
        .expect("parallel-timeout-expires scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify timer fires and task is cancelled
    let timer_fired = scenario.events.iter().any(|e| e.event_type == "TimerFired");
    let task_cancelled = scenario
        .events
        .iter()
        .any(|e| e.event_type == "TaskCancelled");
    let workflow_failed = scenario
        .events
        .iter()
        .any(|e| e.event_type == "WorkflowExecutionFailed");

    assert!(timer_fired, "Expected TimerFired event");
    assert!(task_cancelled, "Expected TaskCancelled event");
    assert!(workflow_failed, "Expected WorkflowExecutionFailed event");

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}

#[tokio::test]
async fn test_parallel_mixed_operations() {
    let scenarios = load_determinism_corpus();
    let scenario = scenarios
        .iter()
        .find(|s| s.name == "parallel-mixed-operations")
        .expect("parallel-mixed-operations scenario not found");

    assert_eq!(
        scenario.determinism_test.test_type, "valid_replay",
        "Expected valid_replay test type"
    );

    let ctx = create_test_context(scenario);

    // Verify we have different operation types scheduled in parallel
    let task_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TaskScheduled")
        .count();
    let timer_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "TimerStarted")
        .count();
    let child_workflow_count = scenario
        .events
        .iter()
        .filter(|e| e.event_type == "ChildWorkflowInitiated")
        .count();

    assert_eq!(task_count, 2, "Expected 2 TaskScheduled events");
    assert_eq!(timer_count, 1, "Expected 1 TimerStarted event");
    assert_eq!(
        child_workflow_count, 1,
        "Expected 1 ChildWorkflowInitiated event"
    );

    // Context should be valid
    assert_eq!(
        ctx.workflow_execution_id().to_string(),
        scenario.workflow_execution_id
    );
}
