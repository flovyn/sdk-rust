# Test Strategy: Sequence-Based Replay

## Overview

Testing determinism is critical because bugs can be subtle and only manifest during replay after workflow code changes. This document defines a comprehensive multi-layer testing strategy.

## Test Layers

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 5: Chaos/Mutation Tests (detect regression paths)       │
├─────────────────────────────────────────────────────────────────┤
│  Layer 4: E2E Replay Tests (real server, real replay)          │
├─────────────────────────────────────────────────────────────────┤
│  Layer 3: TCK Corpus Tests (cross-language validation)         │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: Validator Unit Tests (DeterminismValidator)          │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1: Context Unit Tests (WorkflowContextImpl matching)    │
└─────────────────────────────────────────────────────────────────┘
```

---

## Layer 1: Context Unit Tests

Test the core matching logic in `WorkflowContextImpl` without server.

### Test File: `sdk/src/workflow/context_impl.rs` (module tests)

```rust
#[cfg(test)]
mod sequence_matching_tests {
    use super::*;

    // Helper to create context with pre-populated events
    fn context_with_events(events: Vec<ReplayEvent>) -> WorkflowContextImpl {
        // ... builder pattern
    }

    // =========================================
    // TASK MATCHING TESTS
    // =========================================

    #[test]
    fn task_matches_at_correct_sequence() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TaskScheduled, json!({
                "taskType": "ProcessPayment",
                "taskExecutionId": "task-001"
            })),
            event(3, EventType::TaskCompleted, json!({
                "taskExecutionId": "task-001",
                "result": {"success": true}
            })),
        ];

        let ctx = context_with_events(events);

        // Workflow code calls schedule at seq=2
        let result = ctx.schedule_raw("ProcessPayment", json!({})).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"success": true}));
    }

    #[test]
    fn task_type_mismatch_raises_determinism_violation() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TaskScheduled, json!({
                "taskType": "ProcessPayment",
                "taskExecutionId": "task-001"
            })),
        ];

        let ctx = context_with_events(events);

        // Workflow code calls with DIFFERENT task type
        let result = ctx.schedule_raw("SendEmail", json!({})).await;

        assert!(matches!(result, Err(FlovynError::DeterminismViolation { .. })));
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("ProcessPayment"));
        assert!(err_msg.contains("SendEmail"));
    }

    #[test]
    fn multiple_tasks_same_type_match_by_sequence() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TaskScheduled, json!({
                "taskType": "ProcessItem",
                "taskExecutionId": "task-001"
            })),
            event(3, EventType::TaskCompleted, json!({
                "taskExecutionId": "task-001",
                "result": "result-1"
            })),
            event(4, EventType::TaskScheduled, json!({
                "taskType": "ProcessItem",
                "taskExecutionId": "task-002"
            })),
            event(5, EventType::TaskCompleted, json!({
                "taskExecutionId": "task-002",
                "result": "result-2"
            })),
            event(6, EventType::TaskScheduled, json!({
                "taskType": "ProcessItem",
                "taskExecutionId": "task-003"
            })),
            event(7, EventType::TaskCompleted, json!({
                "taskExecutionId": "task-003",
                "result": "result-3"
            })),
        ];

        let ctx = context_with_events(events);

        // Simulate loop: for i in 0..3 { ctx.schedule::<ProcessItem>() }
        let r1 = ctx.schedule_raw("ProcessItem", json!({})).await.unwrap();
        let r2 = ctx.schedule_raw("ProcessItem", json!({})).await.unwrap();
        let r3 = ctx.schedule_raw("ProcessItem", json!({})).await.unwrap();

        assert_eq!(r1, json!("result-1"));
        assert_eq!(r2, json!("result-2"));
        assert_eq!(r3, json!("result-3"));
    }

    #[test]
    fn wrong_event_type_at_sequence_raises_violation() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TimerStarted, json!({ "timerId": "timer-1" })),
        ];

        let ctx = context_with_events(events);

        // Expected TaskScheduled but got TimerStarted
        let result = ctx.schedule_raw("ProcessPayment", json!({})).await;

        assert!(matches!(result, Err(FlovynError::DeterminismViolation { .. })));
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("TaskScheduled"));
        assert!(err_msg.contains("TimerStarted"));
    }

    // =========================================
    // CHILD WORKFLOW MATCHING TESTS
    // =========================================

    #[test]
    fn child_workflow_matches_by_sequence_and_name() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::ChildWorkflowInitiated, json!({
                "childExecutionName": "process-order-1",
                "childWorkflowKind": "OrderProcessor"
            })),
            event(3, EventType::ChildWorkflowCompleted, json!({
                "childExecutionName": "process-order-1",
                "output": {"orderId": "123"}
            })),
        ];

        let ctx = context_with_events(events);

        let result = ctx.schedule_workflow_raw("process-order-1", "OrderProcessor", json!({})).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"orderId": "123"}));
    }

    #[test]
    fn child_workflow_name_mismatch_raises_violation() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::ChildWorkflowInitiated, json!({
                "childExecutionName": "process-order-1",
                "childWorkflowKind": "OrderProcessor"
            })),
        ];

        let ctx = context_with_events(events);

        // Wrong name
        let result = ctx.schedule_workflow_raw("process-order-2", "OrderProcessor", json!({})).await;

        assert!(matches!(result, Err(FlovynError::DeterminismViolation { .. })));
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("process-order-1"));
        assert!(err_msg.contains("process-order-2"));
    }

    #[test]
    fn child_workflow_kind_mismatch_raises_violation() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::ChildWorkflowInitiated, json!({
                "childExecutionName": "process-order-1",
                "childWorkflowKind": "OrderProcessor"
            })),
        ];

        let ctx = context_with_events(events);

        // Wrong kind
        let result = ctx.schedule_workflow_raw("process-order-1", "PaymentProcessor", json!({})).await;

        assert!(matches!(result, Err(FlovynError::DeterminismViolation { .. })));
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("OrderProcessor"));
        assert!(err_msg.contains("PaymentProcessor"));
    }

    // =========================================
    // LOOP EDGE CASES
    // =========================================

    #[test]
    fn loop_extended_is_valid() {
        // Original: 2 iterations, now 3
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TaskScheduled, json!({"taskType": "T", "taskExecutionId": "t1"})),
            event(3, EventType::TaskCompleted, json!({"taskExecutionId": "t1", "result": 1})),
            event(4, EventType::TaskScheduled, json!({"taskType": "T", "taskExecutionId": "t2"})),
            event(5, EventType::TaskCompleted, json!({"taskExecutionId": "t2", "result": 2})),
            // No event at seq 6-7 yet
        ];

        let ctx = context_with_events(events);

        let r1 = ctx.schedule_raw("T", json!({})).await.unwrap();
        let r2 = ctx.schedule_raw("T", json!({})).await.unwrap();
        let r3 = ctx.schedule_raw("T", json!({})).await; // New command, no event

        assert_eq!(r1, json!(1));
        assert_eq!(r2, json!(2));
        // r3 should suspend or submit new task - NOT a violation
        assert!(matches!(r3, Err(FlovynError::Suspended { .. })));
    }

    #[test]
    fn loop_shortened_behavior() {
        // Original: 3 iterations, now 2
        // The workflow completes without replaying seq 6-7
        // This is valid - orphaned events are allowed
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TaskScheduled, json!({"taskType": "T", "taskExecutionId": "t1"})),
            event(3, EventType::TaskCompleted, json!({"taskExecutionId": "t1", "result": 1})),
            event(4, EventType::TaskScheduled, json!({"taskType": "T", "taskExecutionId": "t2"})),
            event(5, EventType::TaskCompleted, json!({"taskExecutionId": "t2", "result": 2})),
            event(6, EventType::TaskScheduled, json!({"taskType": "T", "taskExecutionId": "t3"})),
            event(7, EventType::TaskCompleted, json!({"taskExecutionId": "t3", "result": 3})),
        ];

        let ctx = context_with_events(events);

        // Only 2 iterations
        let r1 = ctx.schedule_raw("T", json!({})).await.unwrap();
        let r2 = ctx.schedule_raw("T", json!({})).await.unwrap();
        // Workflow completes, seq 6-7 never accessed

        assert_eq!(r1, json!(1));
        assert_eq!(r2, json!(2));
        // Not a violation - workflow just completed earlier
    }

    #[test]
    fn mixed_task_types_in_loop_must_match_order() {
        // for i in 0..2 { A, B }
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TaskScheduled, json!({"taskType": "A", "taskExecutionId": "a1"})),
            event(3, EventType::TaskCompleted, json!({"taskExecutionId": "a1", "result": "A1"})),
            event(4, EventType::TaskScheduled, json!({"taskType": "B", "taskExecutionId": "b1"})),
            event(5, EventType::TaskCompleted, json!({"taskExecutionId": "b1", "result": "B1"})),
            event(6, EventType::TaskScheduled, json!({"taskType": "A", "taskExecutionId": "a2"})),
            event(7, EventType::TaskCompleted, json!({"taskExecutionId": "a2", "result": "A2"})),
            event(8, EventType::TaskScheduled, json!({"taskType": "B", "taskExecutionId": "b2"})),
            event(9, EventType::TaskCompleted, json!({"taskExecutionId": "b2", "result": "B2"})),
        ];

        let ctx = context_with_events(events);

        // Correct order
        assert_eq!(ctx.schedule_raw("A", json!({})).await.unwrap(), json!("A1"));
        assert_eq!(ctx.schedule_raw("B", json!({})).await.unwrap(), json!("B1"));
        assert_eq!(ctx.schedule_raw("A", json!({})).await.unwrap(), json!("A2"));
        assert_eq!(ctx.schedule_raw("B", json!({})).await.unwrap(), json!("B2"));
    }

    #[test]
    fn mixed_task_types_wrong_order_raises_violation() {
        let events = vec![
            event(1, EventType::WorkflowStarted, json!({})),
            event(2, EventType::TaskScheduled, json!({"taskType": "A", "taskExecutionId": "a1"})),
        ];

        let ctx = context_with_events(events);

        // Code changed order: B before A
        let result = ctx.schedule_raw("B", json!({})).await;

        assert!(matches!(result, Err(FlovynError::DeterminismViolation { .. })));
    }
}
```

---

## Layer 2: Validator Unit Tests

Test `DeterminismValidator` directly.

### Test File: `sdk/src/worker/determinism.rs` (module tests)

```rust
#[cfg(test)]
mod determinism_validator_tests {
    use super::*;

    #[test]
    fn validate_schedule_task_matches_task_scheduled() {
        let events = vec![event(1, EventType::TaskScheduled, json!({"taskType": "MyTask"}))];
        let validator = DeterminismValidator::new(events);

        let command = WorkflowCommand::ScheduleTask {
            sequence_number: 1,
            task_type: "MyTask".to_string(),
            task_execution_id: Uuid::new_v4(),
            input: json!({}),
            priority_seconds: None,
        };

        let result = validator.validate_command(&command);
        assert_eq!(result, DeterminismValidationResult::Valid);
    }

    #[test]
    fn validate_task_type_mismatch() {
        let events = vec![event(1, EventType::TaskScheduled, json!({"taskType": "TaskA"}))];
        let validator = DeterminismValidator::new(events);

        let command = WorkflowCommand::ScheduleTask {
            sequence_number: 1,
            task_type: "TaskB".to_string(),
            ..Default::default()
        };

        let result = validator.validate_command(&command);
        assert!(matches!(result, DeterminismValidationResult::TaskTypeMismatch { .. }));
    }

    // ... Similar tests for all command types
}
```

---

## Layer 3: TCK Corpus Tests

JSON-based replay scenarios for cross-language validation (Rust SDK <-> Kotlin server).

### New Corpus Files: `sdk/tests/shared/replay-corpus/`

#### `determinism-task-loop.json`
```json
{
  "name": "determinism-task-loop",
  "description": "Validates sequence-based matching for tasks in a loop",
  "workflow_execution_id": "wf-det-001",
  "org_id": "test-org",
  "workflow_kind": "loop-task-workflow",
  "input": {"items": [1, 2, 3]},
  "events": [
    {"sequence_number": 1, "event_type": "WorkflowStarted", "data": {}},
    {"sequence_number": 2, "event_type": "TaskScheduled", "data": {"taskType": "ProcessItem", "taskExecutionId": "t1"}},
    {"sequence_number": 3, "event_type": "TaskCompleted", "data": {"taskExecutionId": "t1", "result": "r1"}},
    {"sequence_number": 4, "event_type": "TaskScheduled", "data": {"taskType": "ProcessItem", "taskExecutionId": "t2"}},
    {"sequence_number": 5, "event_type": "TaskCompleted", "data": {"taskExecutionId": "t2", "result": "r2"}},
    {"sequence_number": 6, "event_type": "TaskScheduled", "data": {"taskType": "ProcessItem", "taskExecutionId": "t3"}},
    {"sequence_number": 7, "event_type": "TaskCompleted", "data": {"taskExecutionId": "t3", "result": "r3"}},
    {"sequence_number": 8, "event_type": "WorkflowCompleted", "data": {"output": ["r1", "r2", "r3"]}}
  ],
  "expected_commands": [
    {"type": "ScheduleTask", "sequence_number": 2, "taskType": "ProcessItem"},
    {"type": "ScheduleTask", "sequence_number": 4, "taskType": "ProcessItem"},
    {"type": "ScheduleTask", "sequence_number": 6, "taskType": "ProcessItem"},
    {"type": "CompleteWorkflow", "sequence_number": 8}
  ],
  "expected_behavior": "replay_success"
}
```

#### `determinism-violation-task-type.json`
```json
{
  "name": "determinism-violation-task-type",
  "description": "Should raise determinism violation when task type changes",
  "workflow_execution_id": "wf-det-002",
  "org_id": "test-org",
  "workflow_kind": "changed-task-workflow",
  "input": {},
  "events": [
    {"sequence_number": 1, "event_type": "WorkflowStarted", "data": {}},
    {"sequence_number": 2, "event_type": "TaskScheduled", "data": {"taskType": "TaskA", "taskExecutionId": "t1"}}
  ],
  "workflow_code_schedules": "TaskB",
  "expected_behavior": "determinism_violation",
  "expected_error": "Task type mismatch at sequence 2: expected 'TaskA', got 'TaskB'"
}
```

#### `determinism-violation-child-workflow.json`
```json
{
  "name": "determinism-violation-child-workflow-name",
  "description": "Should raise determinism violation when child workflow name changes",
  "workflow_execution_id": "wf-det-003",
  "org_id": "test-org",
  "workflow_kind": "parent-workflow",
  "input": {},
  "events": [
    {"sequence_number": 1, "event_type": "WorkflowStarted", "data": {}},
    {"sequence_number": 2, "event_type": "ChildWorkflowInitiated", "data": {
      "childExecutionName": "child-1",
      "childWorkflowKind": "ChildProcessor"
    }}
  ],
  "workflow_code_child_name": "child-2",
  "expected_behavior": "determinism_violation",
  "expected_error": "Child workflow name mismatch at sequence 2"
}
```

### Test Runner
```rust
// sdk/tests/tck/determinism_corpus.rs

#[test]
fn run_determinism_corpus() {
    let scenarios = load_determinism_corpus();

    for scenario in scenarios {
        match scenario.expected_behavior.as_str() {
            "replay_success" => {
                let result = replay_scenario(&scenario);
                assert!(result.is_ok(), "Scenario {} should succeed", scenario.name);
            }
            "determinism_violation" => {
                let result = replay_scenario(&scenario);
                assert!(matches!(result, Err(FlovynError::DeterminismViolation { .. })));
                if let Some(expected_error) = &scenario.expected_error {
                    assert!(result.unwrap_err().to_string().contains(expected_error));
                }
            }
            _ => panic!("Unknown expected_behavior: {}", scenario.expected_behavior),
        }
    }
}
```

---

## Layer 4: E2E Replay Tests

Real workflow execution + replay with actual server.

### Test File: `sdk/tests/e2e/replay_tests.rs`

```rust
/// Test the complete replay flow with real server
#[tokio::test]
#[ignore] // Requires server
async fn test_e2e_task_loop_replay() {
    let harness = TestHarness::new().await;

    // 1. Execute workflow that schedules 3 tasks in a loop
    let result = harness.execute_workflow::<TaskLoopWorkflow>(json!({"count": 3})).await;
    assert!(result.is_ok());

    // 2. Get the event history from server
    let events = harness.get_workflow_events(&result.workflow_execution_id).await;
    assert!(events.iter().filter(|e| e.event_type == "TaskScheduled").count() == 3);

    // 3. Replay the same workflow code against recorded history
    let replay_result = harness.replay_workflow::<TaskLoopWorkflow>(events).await;
    assert!(replay_result.is_ok(), "Replay should succeed for identical code");
}

#[tokio::test]
#[ignore]
async fn test_e2e_determinism_violation_on_code_change() {
    let harness = TestHarness::new().await;

    // 1. Execute OriginalWorkflow
    let result = harness.execute_workflow::<OriginalWorkflow>(json!({})).await;
    let events = harness.get_workflow_events(&result.workflow_execution_id).await;

    // 2. Replay with ChangedWorkflow (different task type)
    let replay_result = harness.replay_workflow::<ChangedWorkflow>(events).await;
    assert!(matches!(replay_result, Err(FlovynError::DeterminismViolation { .. })));
}

#[tokio::test]
#[ignore]
async fn test_e2e_server_reports_determinism_failure() {
    let harness = TestHarness::new().await;

    // Execute workflow that will fail determinism on server-side replay
    // (simulate by forcing worker restart mid-execution)

    // Verify server marks workflow as FAILED with DETERMINISM_VIOLATION type
    let status = harness.get_workflow_status(&workflow_id).await;
    assert_eq!(status, WorkflowStatus::Failed);
    assert_eq!(status.failure_type, Some("DETERMINISM_VIOLATION"));
}
```

---

## Layer 5: Chaos/Mutation Tests

Detect subtle regression paths by mutating workflow code.

### Approach: Property-Based Testing with `proptest`

```rust
// sdk/tests/unit/state_machine_props.rs

use proptest::prelude::*;

/// Generate random event histories
fn arb_event_history() -> impl Strategy<Value = Vec<ReplayEvent>> {
    prop::collection::vec(arb_event(), 1..20)
        .prop_filter("must start with WorkflowStarted", |events| {
            events.first().map(|e| e.event_type()) == Some(EventType::WorkflowStarted)
        })
}

fn arb_event() -> impl Strategy<Value = ReplayEvent> {
    // Generate valid events with proper sequence numbers
}

proptest! {
    /// Property: Replaying commands in same order always matches events
    #[test]
    fn replay_same_order_is_deterministic(
        events in arb_event_history(),
        commands in arb_commands_from_events(&events)
    ) {
        let ctx = context_with_events(events.clone());

        for cmd in commands {
            let result = execute_command(&ctx, &cmd);
            // Should either succeed or return proper Suspended
            assert!(!matches!(result, Err(FlovynError::DeterminismViolation { .. })));
        }
    }

    /// Property: Swapping command order causes determinism violation
    #[test]
    fn swapped_order_causes_violation(
        events in arb_event_history_with_multiple_commands()
    ) {
        let ctx = context_with_events(events.clone());

        // Execute commands in reverse order
        let reversed_commands = derive_commands_reversed(&events);

        for (i, cmd) in reversed_commands.iter().enumerate() {
            if i > 0 {
                let result = execute_command(&ctx, &cmd);
                // Should fail because order is wrong
                prop_assert!(matches!(result, Err(FlovynError::DeterminismViolation { .. })));
                break;
            }
        }
    }
}
```

### Mutation Testing with `cargo-mutants`

```bash
# Install
cargo install cargo-mutants

# Run mutation testing on determinism module
cargo mutants -p flovyn-sdk -- --lib determinism
```

Expected mutations that MUST be caught:
- Removing sequence number comparison
- Changing `==` to `!=` in type matching
- Removing field validation (taskType, childExecutionName)

---

## Test Matrix Summary

| Test Type | Location | Runs In CI | Purpose |
|-----------|----------|------------|---------|
| Context Unit Tests | `context_impl.rs` | Yes | Core matching logic |
| Validator Unit Tests | `determinism.rs` | Yes | Validation rules |
| TCK Corpus Tests | `tests/tck/` | Yes | Cross-language compat |
| E2E Replay Tests | `tests/e2e/replay_tests.rs` | Yes (with server) | Full integration |
| Property Tests | `state_machine_props.rs` | Yes | Edge case discovery |
| Mutation Tests | CI job | Nightly | Regression detection |

---

## Determinism Violation Error Messages

Critical: Error messages must be actionable. Include:

1. **Sequence number** where violation occurred
2. **Expected value** (from event)
3. **Actual value** (from command)
4. **Field name** that mismatched

Example:
```
DeterminismViolation: Task type mismatch at sequence 5
  Expected: "ProcessPayment" (from historical event)
  Actual: "SendNotification" (from workflow code)

This indicates the workflow code has changed in a non-deterministic way.
Check if the order of task scheduling has changed.
```

---

## CI Integration

```yaml
# .github/workflows/test.yml
jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --lib

  tck-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --test tck

  e2e-tests:
    runs-on: ubuntu-latest
    services:
      flovyn-server:
        image: flovyn-server:latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --test e2e -- --ignored

  mutation-tests:
    runs-on: ubuntu-latest
    if: github.event_name == 'schedule' # Nightly only
    steps:
      - uses: actions/checkout@v4
      - run: cargo mutants -p flovyn-sdk -- --lib determinism
      - run: |
          # Fail if survival rate > 10%
          if [ $(jq '.survival_rate' mutants.json) -gt 0.1 ]; then
            exit 1
          fi
```

---

## TODO: Implementation Checklist

- [ ] Add `get_event_at_sequence()` helper to WorkflowContextImpl
- [ ] Update `schedule_raw()` to use sequence-based matching
- [ ] Update `schedule_workflow_raw()` to use sequence-based matching
- [ ] Remove `consumed_task_execution_ids` HashSet
- [ ] Add determinism corpus JSON files
- [ ] Add TCK test runner for determinism scenarios
- [ ] Add E2E replay tests
- [ ] Add property-based tests
- [ ] Set up mutation testing in CI
- [ ] Update error messages with actionable details
