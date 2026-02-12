//! Test agent definitions for E2E tests

#![allow(dead_code)] // Fixtures will be used when tests are implemented

use async_trait::async_trait;
use flovyn_worker_sdk::agent::combinators::{
    agent_join_all, agent_join_all_outcomes, agent_join_all_with_timeout, agent_select,
    agent_select_with_cancel, TaskOutcome,
};
use flovyn_worker_sdk::agent::context::{AgentContext, EntryRole};
use flovyn_worker_sdk::agent::definition::{DynamicAgent, DynamicAgentInput, DynamicAgentOutput};
use flovyn_worker_sdk::error::Result;
use serde_json::json;

/// Simple agent that echoes its input.
/// Used to test basic agent execution cycle.
pub struct EchoAgent;

#[async_trait]
impl DynamicAgent for EchoAgent {
    fn kind(&self) -> &str {
        "echo-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Extract message from input
        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("no message");

        // Append user message entry
        ctx.append_entry(EntryRole::User, &json!({"text": message}))
            .await?;

        // Create checkpoint
        ctx.checkpoint(&json!({"processed": true})).await?;

        // Append assistant response
        let response = format!("Echo: {}", message);
        ctx.append_entry(EntryRole::Assistant, &json!({"text": &response}))
            .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("response".to_string(), json!(response));
        output.insert("inputMessage".to_string(), json!(message));
        Ok(output)
    }
}

/// Agent that waits for a signal, simulating multi-turn conversation.
/// Used to test signal-based user interaction.
pub struct MultiTurnAgent;

#[async_trait]
impl DynamicAgent for MultiTurnAgent {
    fn kind(&self) -> &str {
        "multi-turn-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get initial prompt
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Hello");

        // Append system message
        ctx.append_entry(EntryRole::System, &json!({"text": "You are a helpful assistant."}))
            .await?;

        // Append user message
        ctx.append_entry(EntryRole::User, &json!({"text": prompt}))
            .await?;

        // Checkpoint after first turn
        ctx.checkpoint(&json!({"turn": 1})).await?;

        // Respond to first turn
        let first_response = format!("Acknowledged: {}", prompt);
        ctx.append_entry(EntryRole::Assistant, &json!({"text": &first_response}))
            .await?;

        // Stream progress for observability
        ctx.stream_progress(0.5, Some("Waiting for follow-up"))
            .await?;

        // Wait for follow-up signal (this will suspend the agent)
        let follow_up: serde_json::Value = ctx.wait_for_signal_raw("followUp").await?;
        let follow_up_text = follow_up
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("no follow-up");

        // Append follow-up user message
        ctx.append_entry(EntryRole::User, &json!({"text": follow_up_text}))
            .await?;

        // Checkpoint after second turn
        ctx.checkpoint(&json!({"turn": 2})).await?;

        // Final response
        let final_response = format!("Follow-up received: {}", follow_up_text);
        ctx.append_entry(EntryRole::Assistant, &json!({"text": &final_response}))
            .await?;

        ctx.stream_progress(1.0, Some("Complete")).await?;

        // Return output with conversation summary
        let mut output = DynamicAgentOutput::new();
        output.insert("firstResponse".to_string(), json!(first_response));
        output.insert("followUpReceived".to_string(), json!(follow_up_text));
        output.insert("finalResponse".to_string(), json!(final_response));
        output.insert("turns".to_string(), json!(2));
        Ok(output)
    }
}

/// Agent that schedules a task and waits for the result.
/// Used to test agent-task integration.
pub struct TaskSchedulingAgent;

#[async_trait]
impl DynamicAgent for TaskSchedulingAgent {
    fn kind(&self) -> &str {
        "task-scheduling-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get task input from agent input
        let task_input = input
            .get("taskInput")
            .cloned()
            .unwrap_or(json!({"default": true}));
        let task_kind = input
            .get("taskKind")
            .and_then(|v| v.as_str())
            .unwrap_or("echo-task");

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Scheduling task: {}", task_kind)}),
        )
        .await?;

        // Checkpoint before task
        ctx.checkpoint(&json!({"phase": "pre-task"})).await?;

        // Append tool call entry
        ctx.append_tool_call(task_kind, &task_input).await?;

        // Schedule the task and wait for result
        let task_result = ctx.schedule_task_raw(task_kind, task_input).await?;

        // Append tool result entry
        ctx.append_tool_result(task_kind, &task_result).await?;

        // Checkpoint after task
        ctx.checkpoint(&json!({"phase": "post-task", "taskCompleted": true}))
            .await?;

        // Log completion
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Task completed successfully"}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("taskResult".to_string(), task_result);
        output.insert("taskKind".to_string(), json!(task_kind));
        Ok(output)
    }
}

/// Agent that streams tokens during execution.
/// Used to test streaming functionality.
pub struct StreamingAgent;

#[async_trait]
impl DynamicAgent for StreamingAgent {
    fn kind(&self) -> &str {
        "streaming-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        let tokens: Vec<&str> = input
            .get("tokens")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| vec!["Hello", ", ", "streaming", " ", "world", "!"]);

        // Stream progress at start
        ctx.stream_progress(0.0, Some("Starting")).await?;

        // Append user message
        ctx.append_entry(EntryRole::User, &json!({"text": "Stream tokens"}))
            .await?;

        // Stream each token
        let mut streamed_count = 0;
        for token in &tokens {
            ctx.stream_token(token).await?;
            streamed_count += 1;
            // Small delay between tokens
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Checkpoint with streaming complete
        ctx.checkpoint(&json!({"streamed": streamed_count})).await?;

        // Append response with full text
        let full_text: String = tokens.iter().copied().collect();
        ctx.append_entry(EntryRole::Assistant, &json!({"text": &full_text}))
            .await?;

        // Stream completion progress
        ctx.stream_progress(1.0, Some("Complete")).await?;

        let mut output = DynamicAgentOutput::new();
        output.insert("fullText".to_string(), json!(full_text));
        output.insert("tokenCount".to_string(), json!(streamed_count));
        Ok(output)
    }
}

/// Agent that demonstrates checkpoint-based recovery.
/// Simulates work that can be resumed from a checkpoint.
pub struct CheckpointAgent;

#[async_trait]
impl DynamicAgent for CheckpointAgent {
    fn kind(&self) -> &str {
        "checkpoint-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        let steps = input
            .get("steps")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as i32;

        // Get starting point from checkpoint state (for resume)
        let start_step = ctx
            .state()
            .and_then(|s| s.get("completedStep"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or(0);

        let mut results = Vec::new();

        // Resume from checkpoint
        if start_step > 0 {
            ctx.append_entry(
                EntryRole::System,
                &json!({"text": format!("Resuming from step {}", start_step)}),
            )
            .await?;
        }

        for step in (start_step + 1)..=steps {
            // Progress
            let progress = step as f64 / steps as f64;
            ctx.stream_progress(progress, Some(&format!("Step {}/{}", step, steps)))
                .await?;

            // Append entry for this step
            ctx.append_entry(
                EntryRole::Assistant,
                &json!({"text": format!("Processing step {}", step)}),
            )
            .await?;

            // Checkpoint after each step
            ctx.checkpoint(&json!({
                "completedStep": step,
                "progress": progress
            }))
            .await?;

            results.push(step);

            // Simulate work
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "All steps completed"}),
        )
        .await?;

        let mut output = DynamicAgentOutput::new();
        output.insert("completedSteps".to_string(), json!(results));
        output.insert("totalSteps".to_string(), json!(steps));
        output.insert("resumedFrom".to_string(), json!(start_step));
        Ok(output)
    }
}

/// Agent that schedules multiple tasks in parallel using agent_join_all.
/// Used to test parallel task execution.
pub struct ParallelTasksAgent;

#[async_trait]
impl DynamicAgent for ParallelTasksAgent {
    fn kind(&self) -> &str {
        "parallel-tasks-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get items to process in parallel from input
        let items: Vec<String> = input
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["a".into(), "b".into(), "c".into()]);

        let task_kind = input
            .get("taskKind")
            .and_then(|v| v.as_str())
            .unwrap_or("echo-task");

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Scheduling {} parallel tasks", items.len())}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({"phase": "pre-tasks", "itemCount": items.len()}))
            .await?;

        // Schedule all tasks without waiting (returns handles)
        let mut handles = Vec::new();
        for item in &items {
            let handle = ctx
                .schedule_task_handle(task_kind, json!({"message": item}))
                .await?;
            handles.push(handle);
        }

        // Wait for all tasks using agent_join_all
        let results = agent_join_all(ctx, handles).await?;

        // Checkpoint after tasks complete
        ctx.checkpoint(&json!({"phase": "post-tasks", "resultCount": results.len()}))
            .await?;

        // Log completion
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("All {} tasks completed", results.len())}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("itemCount".to_string(), json!(items.len()));
        output.insert("results".to_string(), serde_json::Value::Array(results));
        Ok(output)
    }
}

/// Agent that races multiple tasks using agent_select.
/// Used to test select (first-to-complete) pattern.
pub struct RacingTasksAgent;

#[async_trait]
impl DynamicAgent for RacingTasksAgent {
    fn kind(&self) -> &str {
        "racing-tasks-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get delays for tasks to simulate racing
        let primary_delay = input
            .get("primaryDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000);
        let fallback_delay = input
            .get("fallbackDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(500);

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Racing primary and fallback tasks"}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({
            "phase": "pre-race",
            "primaryDelayMs": primary_delay,
            "fallbackDelayMs": fallback_delay
        }))
        .await?;

        // Schedule both tasks
        let primary = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "primary",
                    "delay_ms": primary_delay
                }),
            )
            .await?;

        let fallback = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "fallback",
                    "delay_ms": fallback_delay
                }),
            )
            .await?;

        // Race - first to complete wins
        let (winner_index, result) = agent_select(ctx, vec![primary, fallback]).await?;

        // Checkpoint after race
        ctx.checkpoint(&json!({"phase": "post-race", "winnerIndex": winner_index}))
            .await?;

        // Log winner
        let winner_name = if winner_index == 0 { "primary" } else { "fallback" };
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("{} task won the race", winner_name)}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("winnerIndex".to_string(), json!(winner_index));
        output.insert("winner".to_string(), json!(winner_name));
        output.insert("result".to_string(), result);
        Ok(output)
    }
}

/// Agent that races multiple tasks and cancels losers using agent_select_with_cancel.
/// Used to test select-with-cancellation pattern.
pub struct RacingTasksWithCancelAgent;

#[async_trait]
impl DynamicAgent for RacingTasksWithCancelAgent {
    fn kind(&self) -> &str {
        "racing-tasks-with-cancel-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get delays for tasks to simulate racing
        let fast_delay = input
            .get("fastDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);
        let medium_delay = input
            .get("mediumDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(500);
        let slow_delay = input
            .get("slowDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000);

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Racing three tasks with auto-cancellation of losers"}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({
            "phase": "pre-race",
            "fastDelayMs": fast_delay,
            "mediumDelayMs": medium_delay,
            "slowDelayMs": slow_delay
        }))
        .await?;

        // Schedule three tasks with different delays
        let fast = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "fast",
                    "delay_ms": fast_delay
                }),
            )
            .await?;

        let medium = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "medium",
                    "delay_ms": medium_delay
                }),
            )
            .await?;

        let slow = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "slow",
                    "delay_ms": slow_delay
                }),
            )
            .await?;

        // Race - first to complete wins, others are cancelled
        let race_result =
            agent_select_with_cancel(ctx, vec![fast, medium, slow], Some("Race completed")).await?;

        // Checkpoint after race
        ctx.checkpoint(&json!({
            "phase": "post-race",
            "winnerIndex": race_result.winner_index,
            "cancelledCount": race_result.cancel_results.iter().filter(|r| r.cancelled).count()
        }))
        .await?;

        // Log winner
        let winner_names = ["fast", "medium", "slow"];
        let winner_name = winner_names[race_result.winner_index];
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("{} task won the race, cancelled {} others", winner_name, race_result.cancel_results.len())}),
        )
        .await?;

        // Build cancellation info
        let cancel_info: Vec<serde_json::Value> = race_result
            .cancel_results
            .iter()
            .map(|r| {
                json!({
                    "index": r.index,
                    "taskId": r.task_id.to_string(),
                    "cancelled": r.cancelled,
                    "reason": r.reason
                })
            })
            .collect();

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("winnerIndex".to_string(), json!(race_result.winner_index));
        output.insert("winner".to_string(), json!(winner_name));
        output.insert("result".to_string(), race_result.result);
        output.insert("cancelResults".to_string(), json!(cancel_info));
        Ok(output)
    }
}

/// Agent that schedules parallel tasks where some may fail.
/// Uses agent_join_all_outcomes to collect all results including failures.
pub struct ParallelWithFailuresAgent;

#[async_trait]
impl DynamicAgent for ParallelWithFailuresAgent {
    fn kind(&self) -> &str {
        "parallel-with-failures-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get task configurations from input
        // Each task can have: name, shouldFail, delayMs
        let tasks: Vec<serde_json::Value> = input
            .get("tasks")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| {
                vec![
                    json!({"name": "task1", "shouldFail": false}),
                    json!({"name": "task2", "shouldFail": true}),
                    json!({"name": "task3", "shouldFail": false}),
                ]
            });

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Scheduling {} parallel tasks (some may fail)", tasks.len())}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({"phase": "pre-tasks", "taskCount": tasks.len()}))
            .await?;

        // Schedule all tasks
        let mut handles = Vec::new();
        for task_config in &tasks {
            let handle = ctx
                .schedule_task_handle("conditional-fail-task", task_config.clone())
                .await?;
            handles.push(handle);
        }

        // Wait for all tasks and collect outcomes (no fail-fast)
        let outcomes = agent_join_all_outcomes(ctx, handles).await?;

        // Checkpoint after tasks complete
        ctx.checkpoint(&json!({"phase": "post-tasks"})).await?;

        // Categorize outcomes
        let mut completed_count = 0;
        let mut failed_count = 0;
        let mut cancelled_count = 0;
        let mut completed_results = Vec::new();
        let mut failed_errors = Vec::new();

        for (i, outcome) in outcomes.iter().enumerate() {
            match outcome {
                TaskOutcome::Completed(value) => {
                    completed_count += 1;
                    completed_results.push(json!({
                        "index": i,
                        "result": value
                    }));
                }
                TaskOutcome::Failed(error) => {
                    failed_count += 1;
                    failed_errors.push(json!({
                        "index": i,
                        "error": error
                    }));
                }
                TaskOutcome::Cancelled => {
                    cancelled_count += 1;
                }
                TaskOutcome::Pending => {
                    // Shouldn't happen after join_all_outcomes
                }
            }
        }

        // Log summary
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!(
                "Tasks completed: {} succeeded, {} failed, {} cancelled",
                completed_count, failed_count, cancelled_count
            )}),
        )
        .await?;

        // Return output with detailed results
        let mut output = DynamicAgentOutput::new();
        output.insert("totalTasks".to_string(), json!(tasks.len()));
        output.insert("completedCount".to_string(), json!(completed_count));
        output.insert("failedCount".to_string(), json!(failed_count));
        output.insert("cancelledCount".to_string(), json!(cancelled_count));
        output.insert("completedResults".to_string(), json!(completed_results));
        output.insert("failedErrors".to_string(), json!(failed_errors));
        Ok(output)
    }
}

/// Agent that schedules slow tasks with a timeout.
/// Uses agent_join_all_with_timeout to cancel tasks that don't complete in time.
pub struct TimeoutTasksAgent;

#[async_trait]
impl DynamicAgent for TimeoutTasksAgent {
    fn kind(&self) -> &str {
        "timeout-tasks-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get configuration
        let task_count = input
            .get("taskCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;
        let task_delay_ms = input
            .get("taskDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000);
        let timeout_ms = input
            .get("timeoutMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!(
                "Scheduling {} tasks with {}ms delay, timeout {}ms",
                task_count, task_delay_ms, timeout_ms
            )}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({
            "phase": "pre-tasks",
            "taskCount": task_count,
            "taskDelayMs": task_delay_ms,
            "timeoutMs": timeout_ms
        }))
        .await?;

        // Schedule slow tasks
        let mut handles = Vec::new();
        for i in 0..task_count {
            let handle = ctx
                .schedule_task_handle(
                    "slow-task",
                    json!({
                        "source": format!("task-{}", i),
                        "delay_ms": task_delay_ms
                    }),
                )
                .await?;
            handles.push(handle);
        }

        // Wait with timeout
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let result = agent_join_all_with_timeout(ctx, handles, timeout, Some("Timeout exceeded"))
            .await?;

        // Checkpoint after timeout
        ctx.checkpoint(&json!({
            "phase": "post-timeout",
            "timedOut": result.timed_out,
            "completedCount": result.completed.len(),
            "cancelledCount": result.cancelled.len()
        }))
        .await?;

        // Log result
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!(
                "Timeout result: {} completed, {} failed, {} cancelled, timed_out={}",
                result.completed.len(),
                result.failed.len(),
                result.cancelled.len(),
                result.timed_out
            )}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("totalTasks".to_string(), json!(task_count));
        output.insert("completedCount".to_string(), json!(result.completed.len()));
        output.insert("failedCount".to_string(), json!(result.failed.len()));
        output.insert("cancelledCount".to_string(), json!(result.cancelled.len()));
        output.insert("timedOut".to_string(), json!(result.timed_out));

        // Include completed task indices
        let completed_indices: Vec<usize> = result.completed.iter().map(|(i, _)| *i).collect();
        output.insert("completedIndices".to_string(), json!(completed_indices));

        // Include cancelled task indices
        let cancelled_indices: Vec<usize> = result.cancelled.iter().map(|(i, _)| *i).collect();
        output.insert("cancelledIndices".to_string(), json!(cancelled_indices));

        Ok(output)
    }
}

/// Agent that uses batch scheduling API to schedule multiple tasks at once.
/// Used to test the batch scheduling optimization.
pub struct BatchSchedulingAgent;

#[async_trait]
impl DynamicAgent for BatchSchedulingAgent {
    fn kind(&self) -> &str {
        "batch-scheduling-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get items to process
        let items: Vec<String> = input
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()]);

        let task_kind = input
            .get("taskKind")
            .and_then(|v| v.as_str())
            .unwrap_or("echo-task");

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Batch scheduling {} tasks", items.len())}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({"phase": "pre-batch", "itemCount": items.len()}))
            .await?;

        // Build batch of tasks
        let tasks: Vec<(&str, serde_json::Value)> = items
            .iter()
            .map(|item| (task_kind, json!({"message": item})))
            .collect();

        // Schedule all tasks in a single batch RPC
        let handles = ctx.schedule_tasks_batch(tasks).await?;

        // Wait for all tasks
        let results = agent_join_all(ctx, handles).await?;

        // Checkpoint after tasks
        ctx.checkpoint(&json!({"phase": "post-batch", "resultCount": results.len()}))
            .await?;

        // Log completion
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Batch completed: {} results", results.len())}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("itemCount".to_string(), json!(items.len()));
        output.insert("results".to_string(), serde_json::Value::Array(results));
        output.insert("usedBatchApi".to_string(), json!(true));
        Ok(output)
    }
}

/// Agent that cancels a slow task and schedules a replacement.
/// Used to test the cancel-and-replace pattern.
pub struct CancelAndReplaceAgent;

#[async_trait]
impl DynamicAgent for CancelAndReplaceAgent {
    fn kind(&self) -> &str {
        "cancel-and-replace-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        let slow_delay_ms = input
            .get("slowDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(10000); // 10 seconds
        let fast_delay_ms = input
            .get("fastDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(100); // 100ms
        let wait_before_cancel_ms = input
            .get("waitBeforeCancelMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(200); // 200ms

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Starting slow task, will cancel and replace if too slow"}),
        )
        .await?;

        // Checkpoint before task
        ctx.checkpoint(&json!({"phase": "pre-task"})).await?;

        // Schedule slow task
        let slow_task = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "slow",
                    "delay_ms": slow_delay_ms
                }),
            )
            .await?;

        // Wait a bit then cancel
        tokio::time::sleep(std::time::Duration::from_millis(wait_before_cancel_ms)).await;

        // Cancel the slow task
        let cancel_result = ctx.cancel_task(&slow_task, Some("Replacing with faster task")).await?;

        // Schedule faster replacement
        let fast_task = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "fast-replacement",
                    "delay_ms": fast_delay_ms
                }),
            )
            .await?;

        // Wait for replacement to complete
        let results = agent_join_all(ctx, vec![fast_task]).await?;
        let result = results.into_iter().next().unwrap_or(json!(null));

        // Checkpoint after replacement completes
        ctx.checkpoint(&json!({"phase": "post-replacement"})).await?;

        // Log completion
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Replacement task completed"}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("slowTaskCancelled".to_string(), json!(cancel_result.cancelled));
        output.insert("slowTaskStatus".to_string(), json!(cancel_result.status));
        output.insert("replacementResult".to_string(), result);
        output.insert("success".to_string(), json!(true));
        Ok(output)
    }
}

/// Agent that tests cancel idempotency by cancelling the same task multiple times.
/// Used to verify multiple cancel calls are safe.
pub struct CancelIdempotencyAgent;

#[async_trait]
impl DynamicAgent for CancelIdempotencyAgent {
    fn kind(&self) -> &str {
        "cancel-idempotency-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        let cancel_count = input
            .get("cancelCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;
        let task_delay_ms = input
            .get("taskDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000); // 5 seconds

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Testing cancel idempotency with {} cancel calls", cancel_count)}),
        )
        .await?;

        // Checkpoint before task
        ctx.checkpoint(&json!({"phase": "pre-task"})).await?;

        // Schedule a slow task
        let task = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "target",
                    "delay_ms": task_delay_ms
                }),
            )
            .await?;

        // Cancel the task multiple times
        let mut cancel_results = Vec::new();
        for i in 0..cancel_count {
            let result = ctx.cancel_task(&task, None).await?;
            cancel_results.push(json!({
                "attempt": i + 1,
                "cancelled": result.cancelled,
                "status": result.status
            }));
        }

        // Checkpoint after cancellations
        ctx.checkpoint(&json!({"phase": "post-cancel"})).await?;

        // Log completion
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Completed {} cancel attempts", cancel_count)}),
        )
        .await?;

        // Check results
        let first_cancelled = cancel_results
            .first()
            .and_then(|r| r.get("cancelled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("cancelCount".to_string(), json!(cancel_count));
        output.insert("cancelResults".to_string(), json!(cancel_results));
        output.insert("firstCancelSucceeded".to_string(), json!(first_cancelled));
        output.insert("success".to_string(), json!(true));
        Ok(output)
    }
}

/// Agent that tests join_all behavior with a pre-cancelled handle.
/// Used to verify proper error handling when joining cancelled tasks.
pub struct JoinAllWithCancelledHandleAgent;

#[async_trait]
impl DynamicAgent for JoinAllWithCancelledHandleAgent {
    fn kind(&self) -> &str {
        "join-all-cancelled-handle-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        let use_outcomes = input
            .get("useOutcomes")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Testing join_all with pre-cancelled handle"}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({"phase": "pre-tasks"})).await?;

        // Schedule two tasks
        let task1 = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "task1",
                    "delay_ms": 5000  // 5 seconds - won't complete
                }),
            )
            .await?;

        let task2 = ctx
            .schedule_task_handle(
                "slow-task",
                json!({
                    "source": "task2",
                    "delay_ms": 100  // Fast
                }),
            )
            .await?;

        // Cancel task1 before joining
        let cancel_result = ctx.cancel_task(&task1, Some("Testing cancelled handle")).await?;

        // Now try to join both - task1 is already cancelled
        let mut output = DynamicAgentOutput::new();
        output.insert("task1Cancelled".to_string(), json!(cancel_result.cancelled));

        if use_outcomes {
            // Use agent_join_all_outcomes which handles cancelled tasks gracefully
            let outcomes = agent_join_all_outcomes(ctx, vec![task1, task2]).await?;

            let mut completed_count = 0;
            let mut cancelled_count = 0;
            let mut results = Vec::new();

            for (i, outcome) in outcomes.iter().enumerate() {
                match outcome {
                    TaskOutcome::Completed(v) => {
                        completed_count += 1;
                        results.push(json!({"index": i, "status": "completed", "value": v}));
                    }
                    TaskOutcome::Cancelled => {
                        cancelled_count += 1;
                        results.push(json!({"index": i, "status": "cancelled"}));
                    }
                    TaskOutcome::Failed(e) => {
                        results.push(json!({"index": i, "status": "failed", "error": e}));
                    }
                    TaskOutcome::Pending => {
                        results.push(json!({"index": i, "status": "pending"}));
                    }
                }
            }

            output.insert("usedOutcomes".to_string(), json!(true));
            output.insert("completedCount".to_string(), json!(completed_count));
            output.insert("cancelledCount".to_string(), json!(cancelled_count));
            output.insert("results".to_string(), json!(results));
            output.insert("success".to_string(), json!(true));
        } else {
            // Use regular agent_join_all - should fail because task1 is cancelled
            match agent_join_all(ctx, vec![task1, task2]).await {
                Ok(_results) => {
                    // Unexpected success
                    output.insert("joinAllSucceeded".to_string(), json!(true));
                    output.insert("success".to_string(), json!(false));
                    output.insert("error".to_string(), json!("Expected join_all to fail with cancelled task"));
                }
                Err(e) => {
                    // Expected failure
                    output.insert("joinAllSucceeded".to_string(), json!(false));
                    output.insert("joinAllError".to_string(), json!(e.to_string()));
                    output.insert("success".to_string(), json!(true));
                }
            }
        }

        // Checkpoint after test
        ctx.checkpoint(&json!({"phase": "post-test"})).await?;

        Ok(output)
    }
}

/// Agent that mixes parallel and sequential task execution.
/// Demonstrates combining different patterns in a single agent.
pub struct MixedParallelSequentialAgent;

#[async_trait]
impl DynamicAgent for MixedParallelSequentialAgent {
    fn kind(&self) -> &str {
        "mixed-parallel-sequential-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        _input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        let mut output = DynamicAgentOutput::new();

        // Phase 1: Two parallel tasks
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Phase 1: Starting two parallel tasks"}),
        )
        .await?;

        let task1 = ctx
            .schedule_task_handle("echo-task", json!({"message": "phase1-a"}))
            .await?;
        let task2 = ctx
            .schedule_task_handle("echo-task", json!({"message": "phase1-b"}))
            .await?;

        let phase1_results = agent_join_all(ctx, vec![task1, task2]).await?;
        output.insert("phase1".to_string(), serde_json::Value::Array(phase1_results));

        ctx.checkpoint(&json!({"completedPhase": 1})).await?;

        // Phase 2: Single sequential task (using schedule_task_raw)
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Phase 2: Starting sequential task"}),
        )
        .await?;

        let phase2_result = ctx
            .schedule_task_raw("echo-task", json!({"message": "phase2-sequential"}))
            .await?;
        output.insert("phase2".to_string(), phase2_result);

        ctx.checkpoint(&json!({"completedPhase": 2})).await?;

        // Phase 3: Three parallel tasks
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Phase 3: Starting three parallel tasks"}),
        )
        .await?;

        let task3 = ctx
            .schedule_task_handle("echo-task", json!({"message": "phase3-a"}))
            .await?;
        let task4 = ctx
            .schedule_task_handle("echo-task", json!({"message": "phase3-b"}))
            .await?;
        let task5 = ctx
            .schedule_task_handle("echo-task", json!({"message": "phase3-c"}))
            .await?;

        let phase3_results = agent_join_all(ctx, vec![task3, task4, task5]).await?;
        output.insert("phase3".to_string(), serde_json::Value::Array(phase3_results));

        ctx.checkpoint(&json!({"completedPhase": 3})).await?;

        // Summary
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "All phases complete: 2 parallel + 1 sequential + 3 parallel = 6 tasks"}),
        )
        .await?;

        output.insert("totalTasks".to_string(), json!(6));
        output.insert("phases".to_string(), json!(3));
        output.insert("success".to_string(), json!(true));
        Ok(output)
    }
}
