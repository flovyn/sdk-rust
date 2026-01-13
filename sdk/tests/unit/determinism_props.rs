//! Workflow Determinism Property Tests
//!
//! These tests verify the core determinism property:
//! **Same workflow code + same events = same commands**
//!
//! This is critical for replay correctness. If a workflow produces different
//! commands on replay than it did originally, the replay will fail.
//!
//! ## What We Test
//!
//! 1. Running the same workflow twice with identical inputs produces identical actions
//! 2. Workflows using deterministic context methods (time, random) are reproducible
//! 3. Parallel execution patterns are deterministic
//!
//! ## How It Works
//!
//! For each test:
//! 1. Create a MockWorkflowContext with fixed configuration
//! 2. Run the workflow and capture actions (scheduled tasks, operations, etc.)
//! 3. Reset and run again with identical configuration
//! 4. Assert the captured actions are identical

// Note: This module requires the "testing" feature
#![cfg(feature = "testing")]

use async_trait::async_trait;
use flovyn_sdk::error::Result;
use flovyn_sdk::testing::MockWorkflowContext;
use flovyn_sdk::workflow::context::WorkflowContext;
use flovyn_sdk::workflow::definition::WorkflowDefinition;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Captured actions from a workflow execution for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedActions {
    /// Task types scheduled in order
    scheduled_task_types: Vec<String>,
    /// Operation names recorded in order
    recorded_operations: Vec<String>,
    /// Child workflow kinds scheduled in order
    scheduled_workflow_kinds: Vec<String>,
    /// Promise names created in order
    created_promises: Vec<String>,
}

impl CapturedActions {
    fn from_context(ctx: &MockWorkflowContext) -> Self {
        Self {
            scheduled_task_types: ctx
                .scheduled_tasks()
                .iter()
                .map(|t| t.task_type.clone())
                .collect(),
            recorded_operations: ctx
                .recorded_operations()
                .iter()
                .map(|o| o.name.clone())
                .collect(),
            scheduled_workflow_kinds: ctx
                .scheduled_workflows()
                .iter()
                .map(|w| w.kind.clone())
                .collect(),
            created_promises: ctx.created_promises(),
        }
    }
}

/// Helper to run a workflow and capture its actions.
async fn run_and_capture<W>(
    workflow: &W,
    input: W::Input,
    ctx_builder: impl Fn() -> MockWorkflowContext,
) -> (Result<W::Output>, CapturedActions)
where
    W: WorkflowDefinition,
{
    let ctx = Arc::new(ctx_builder());
    let result = workflow.execute(ctx.as_ref(), input).await;
    let actions = CapturedActions::from_context(&ctx);
    (result, actions)
}

/// Verify that a workflow is deterministic by running it twice.
async fn assert_deterministic<W>(
    workflow: &W,
    input: W::Input,
    ctx_builder: impl Fn() -> MockWorkflowContext,
) where
    W: WorkflowDefinition,
    W::Input: Clone,
    W::Output: PartialEq + std::fmt::Debug,
{
    // Run 1
    let (result1, actions1) = run_and_capture(workflow, input.clone(), &ctx_builder).await;

    // Run 2
    let (result2, actions2) = run_and_capture(workflow, input, &ctx_builder).await;

    // Compare actions
    assert_eq!(
        actions1, actions2,
        "Workflow produced different actions on second run!\nRun 1: {:?}\nRun 2: {:?}",
        actions1, actions2
    );

    // Compare results
    assert_eq!(
        result1.is_ok(),
        result2.is_ok(),
        "Workflow result status differs between runs"
    );

    if let (Ok(output1), Ok(output2)) = (&result1, &result2) {
        assert_eq!(
            output1, output2,
            "Workflow produced different output on second run"
        );
    }
}

// ============================================================================
// Test Workflows
// ============================================================================

/// Simple workflow that schedules a single task.
struct SingleTaskWorkflow;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SingleTaskInput {
    value: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct SingleTaskOutput {
    result: Value,
}

#[async_trait]
impl WorkflowDefinition for SingleTaskWorkflow {
    type Input = SingleTaskInput;
    type Output = SingleTaskOutput;

    fn kind(&self) -> &str {
        "single-task-workflow"
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let result = ctx
            .schedule_raw("process-item", json!({"item": input.value}))
            .await?;

        Ok(SingleTaskOutput { result })
    }
}

/// Workflow that schedules multiple tasks sequentially.
struct SequentialTasksWorkflow;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SequentialInput {
    items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct SequentialOutput {
    results: Vec<Value>,
}

#[async_trait]
impl WorkflowDefinition for SequentialTasksWorkflow {
    type Input = SequentialInput;
    type Output = SequentialOutput;

    fn kind(&self) -> &str {
        "sequential-tasks-workflow"
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let mut results = Vec::new();
        for item in &input.items {
            let result = ctx
                .schedule_raw("process-item", json!({"item": item}))
                .await?;
            results.push(result);
        }

        Ok(SequentialOutput { results })
    }
}

/// Workflow that uses deterministic time.
struct TimeBasedWorkflow;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TimeInput {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct TimeOutput {
    timestamp: i64,
    decision: String,
}

#[async_trait]
impl WorkflowDefinition for TimeBasedWorkflow {
    type Input = TimeInput;
    type Output = TimeOutput;

    fn kind(&self) -> &str {
        "time-based-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: Self::Input,
    ) -> Result<Self::Output> {
        let timestamp = ctx.current_time_millis();

        // Make a decision based on time
        let decision = if timestamp % 2 == 0 {
            "even".to_string()
        } else {
            "odd".to_string()
        };

        // Record the decision
        ctx.run_raw("record-decision", json!({"decision": &decision}))
            .await?;

        Ok(TimeOutput {
            timestamp,
            decision,
        })
    }
}

/// Workflow that uses deterministic random.
struct RandomBasedWorkflow;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RandomInput {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct RandomOutput {
    uuid: String,
    random_value: bool,
}

#[async_trait]
impl WorkflowDefinition for RandomBasedWorkflow {
    type Input = RandomInput;
    type Output = RandomOutput;

    fn kind(&self) -> &str {
        "random-based-workflow"
    }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        _input: Self::Input,
    ) -> Result<Self::Output> {
        let uuid = ctx.random_uuid();
        let random_value = ctx.random().next_bool();

        // Schedule task based on random decision
        if random_value {
            ctx.schedule_raw("path-a", json!({})).await?;
        } else {
            ctx.schedule_raw("path-b", json!({})).await?;
        }

        Ok(RandomOutput {
            uuid: uuid.to_string(),
            random_value,
        })
    }
}

/// Workflow with conditional logic.
struct ConditionalWorkflow;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ConditionalInput {
    amount: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct ConditionalOutput {
    approved: bool,
    path: String,
}

#[async_trait]
impl WorkflowDefinition for ConditionalWorkflow {
    type Input = ConditionalInput;
    type Output = ConditionalOutput;

    fn kind(&self) -> &str {
        "conditional-workflow"
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        if input.amount > 100 {
            // High value path - needs approval
            ctx.schedule_raw("request-approval", json!({"amount": input.amount}))
                .await?;
            ctx.run_raw("log-high-value", json!({})).await?;
            Ok(ConditionalOutput {
                approved: true,
                path: "high-value".to_string(),
            })
        } else {
            // Low value path - auto-approve
            ctx.run_raw("log-auto-approve", json!({})).await?;
            Ok(ConditionalOutput {
                approved: true,
                path: "low-value".to_string(),
            })
        }
    }
}

/// Workflow using parallel execution (join_all pattern).
struct ParallelWorkflow;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ParallelInput {
    items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
struct ParallelOutput {
    count: usize,
}

#[async_trait]
impl WorkflowDefinition for ParallelWorkflow {
    type Input = ParallelInput;
    type Output = ParallelOutput;

    fn kind(&self) -> &str {
        "parallel-workflow"
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        // Schedule all tasks (simulating join_all pattern)
        let mut futures = Vec::new();
        for item in &input.items {
            futures.push(ctx.schedule_raw("process-item", json!({"item": item})));
        }

        // Wait for all
        let mut count = 0;
        for future in futures {
            future.await?;
            count += 1;
        }

        Ok(ParallelOutput { count })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_single_task_is_deterministic() {
    let workflow = SingleTaskWorkflow;

    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .task_result("process-item", json!({"processed": true}))
            .build()
    };

    assert_deterministic(&workflow, SingleTaskInput { value: 42 }, ctx_builder).await;
}

#[tokio::test]
async fn test_sequential_tasks_are_deterministic() {
    let workflow = SequentialTasksWorkflow;

    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .task_result("process-item", json!({"processed": true}))
            .build()
    };

    assert_deterministic(
        &workflow,
        SequentialInput {
            items: vec!["a".into(), "b".into(), "c".into()],
        },
        ctx_builder,
    )
    .await;
}

#[tokio::test]
async fn test_time_based_workflow_is_deterministic() {
    let workflow = TimeBasedWorkflow;

    // With fixed time, the workflow should always make the same decision
    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000) // Even timestamp
            .rng_seed(42)
            .build()
    };

    assert_deterministic(&workflow, TimeInput {}, ctx_builder).await;
}

#[tokio::test]
async fn test_random_based_workflow_is_deterministic() {
    let workflow = RandomBasedWorkflow;

    // With fixed RNG seed, random values should be reproducible
    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .task_result("path-a", json!({}))
            .task_result("path-b", json!({}))
            .build()
    };

    assert_deterministic(&workflow, RandomInput {}, ctx_builder).await;
}

#[tokio::test]
async fn test_conditional_high_value_is_deterministic() {
    let workflow = ConditionalWorkflow;

    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .task_result("request-approval", json!({"approved": true}))
            .build()
    };

    // High value - should go through approval path
    assert_deterministic(&workflow, ConditionalInput { amount: 150 }, ctx_builder).await;
}

#[tokio::test]
async fn test_conditional_low_value_is_deterministic() {
    let workflow = ConditionalWorkflow;

    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .build()
    };

    // Low value - should auto-approve
    assert_deterministic(&workflow, ConditionalInput { amount: 50 }, ctx_builder).await;
}

#[tokio::test]
async fn test_parallel_workflow_is_deterministic() {
    let workflow = ParallelWorkflow;

    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .task_result("process-item", json!({"processed": true}))
            .build()
    };

    assert_deterministic(
        &workflow,
        ParallelInput {
            items: vec!["x".into(), "y".into(), "z".into()],
        },
        ctx_builder,
    )
    .await;
}

/// Test that changing input produces different actions (sanity check).
#[tokio::test]
async fn test_different_input_produces_different_actions() {
    let workflow = SequentialTasksWorkflow;

    let ctx_builder = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .task_result("process-item", json!({"processed": true}))
            .build()
    };

    // Run with 2 items
    let (_, actions1) = run_and_capture(
        &workflow,
        SequentialInput {
            items: vec!["a".into(), "b".into()],
        },
        &ctx_builder,
    )
    .await;

    // Run with 3 items
    let (_, actions2) = run_and_capture(
        &workflow,
        SequentialInput {
            items: vec!["a".into(), "b".into(), "c".into()],
        },
        &ctx_builder,
    )
    .await;

    // Should be different
    assert_ne!(
        actions1.scheduled_task_types.len(),
        actions2.scheduled_task_types.len(),
        "Different inputs should produce different number of scheduled tasks"
    );
}

/// Test that conditional paths produce expected actions.
#[tokio::test]
async fn test_conditional_paths_are_distinct() {
    let workflow = ConditionalWorkflow;

    // High value context
    let high_ctx = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .task_result("request-approval", json!({"approved": true}))
            .build()
    };

    // Low value context
    let low_ctx = || {
        MockWorkflowContext::builder()
            .workflow_execution_id(Uuid::from_u128(1))
            .org_id(Uuid::from_u128(2))
            .initial_time_millis(1000)
            .rng_seed(42)
            .build()
    };

    let (_, high_actions) =
        run_and_capture(&workflow, ConditionalInput { amount: 150 }, &high_ctx).await;

    let (_, low_actions) =
        run_and_capture(&workflow, ConditionalInput { amount: 50 }, &low_ctx).await;

    // High value should schedule a task
    assert_eq!(high_actions.scheduled_task_types, vec!["request-approval"]);
    assert_eq!(high_actions.recorded_operations, vec!["log-high-value"]);

    // Low value should not schedule a task
    assert!(low_actions.scheduled_task_types.is_empty());
    assert_eq!(low_actions.recorded_operations, vec!["log-auto-approve"]);
}
