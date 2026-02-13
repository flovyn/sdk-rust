//! Test agent definitions for E2E tests

#![allow(dead_code)] // Fixtures will be used when tests are implemented

use async_trait::async_trait;
use flovyn_worker_sdk::agent::context::ScheduleAgentTaskOptions;
use std::time::Duration;
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
        ctx.append_entry(
            EntryRole::System,
            &json!({"text": "You are a helpful assistant."}),
        )
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
        let steps = input.get("steps").and_then(|v| v.as_u64()).unwrap_or(3) as i32;

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

/// Agent that schedules multiple tasks in parallel using join_all.
/// Used to test parallel task execution with the new lazy API.
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

        // Schedule all tasks lazily (no RPC yet)
        let futures: Vec<_> = items
            .iter()
            .map(|item| ctx.schedule_task_lazy(task_kind, json!({"message": item})))
            .collect();

        // Wait for all tasks using join_all
        let results = ctx.join_all(futures).await?;

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

/// Agent that races multiple tasks using select_ok.
/// Used to test select (first-to-succeed) pattern with the new lazy API.
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

        // Schedule tasks lazily
        let fallback = ctx.schedule_task_lazy(
            "slow-task",
            json!({
                "source": "fallback",
                "delay_ms": fallback_delay
            }),
        );

        let primary = ctx.schedule_task_lazy(
            "slow-task",
            json!({
                "source": "primary",
                "delay_ms": primary_delay
            }),
        );

        // Race: fallback is index 0, primary is index 1
        let (result, remaining) = ctx.select_ok(vec![fallback, primary]).await?;

        // Determine winner based on which task completed first
        // With select_ok, we don't get the index directly, but we can infer from remaining
        let winner_name = if remaining.len() == 1 && remaining[0].kind() == "slow-task" {
            // The one that finished was whichever one is NOT in remaining
            // Since inputs are similar, we'll just report "first task"
            "first"
        } else {
            "unknown"
        };

        ctx.checkpoint(&json!({"phase": "post-race"}))
            .await?;

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("{} task won the race", winner_name)}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("winner".to_string(), json!(winner_name));
        output.insert("result".to_string(), result);
        output.insert("remainingTasks".to_string(), json!(remaining.len()));
        Ok(output)
    }
}

/// Agent that tests select_ok: returns first successful task, skipping failures.
/// Used to test the fallback pattern where multiple providers are tried.
pub struct SelectOkAgent;

#[async_trait]
impl DynamicAgent for SelectOkAgent {
    fn kind(&self) -> &str {
        "select-ok-agent"
    }

    async fn execute(
        &self,
        ctx: &dyn AgentContext,
        input: DynamicAgentInput,
    ) -> Result<DynamicAgentOutput> {
        // Get task configurations - array of {shouldFail, delayMs}
        let tasks: Vec<(bool, u64)> = input
            .get("tasks")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|t| {
                        let should_fail = t.get("shouldFail").and_then(|v| v.as_bool()).unwrap_or(false);
                        let delay_ms = t.get("delayMs").and_then(|v| v.as_u64()).unwrap_or(100);
                        (should_fail, delay_ms)
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![(true, 100), (true, 200), (false, 300)]);

        // Log intent
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!(
                "Scheduling {} tasks with select_ok (expecting first success)",
                tasks.len()
            )}),
        )
        .await?;

        // Checkpoint before tasks
        ctx.checkpoint(&json!({
            "phase": "pre-tasks",
            "taskCount": tasks.len(),
        }))
        .await?;

        // Schedule tasks lazily with conditional failure
        let futures: Vec<_> = tasks
            .iter()
            .enumerate()
            .map(|(i, (should_fail, delay_ms))| {
                ctx.schedule_task_lazy(
                    "conditional-fail-task",
                    json!({
                        "taskIndex": i,
                        "shouldFail": should_fail,
                        "delayMs": delay_ms
                    }),
                )
            })
            .collect();

        // Wait for first successful task
        let (result, remaining) = ctx.select_ok(futures).await?;

        // Checkpoint after select
        ctx.checkpoint(&json!({
            "phase": "post-select",
            "remainingCount": remaining.len(),
        }))
        .await?;

        // Log result
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "select_ok returned a winner"}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("result".to_string(), result);
        output.insert("remainingTasks".to_string(), json!(remaining.len()));

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

        // Phase 1: Two parallel tasks using new lazy API
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Phase 1: Starting two parallel tasks"}),
        )
        .await?;

        let f1 = ctx.schedule_task_lazy("echo-task", json!({"message": "phase1-a"}));
        let f2 = ctx.schedule_task_lazy("echo-task", json!({"message": "phase1-b"}));

        let phase1_results = ctx.join_all(vec![f1, f2]).await?;
        output.insert(
            "phase1".to_string(),
            serde_json::Value::Array(phase1_results),
        );

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

        // Phase 3: Three parallel tasks using new lazy API
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Phase 3: Starting three parallel tasks"}),
        )
        .await?;

        let f3 = ctx.schedule_task_lazy("echo-task", json!({"message": "phase3-a"}));
        let f4 = ctx.schedule_task_lazy("echo-task", json!({"message": "phase3-b"}));
        let f5 = ctx.schedule_task_lazy("echo-task", json!({"message": "phase3-c"}));

        let phase3_results = ctx.join_all(vec![f3, f4, f5]).await?;
        output.insert(
            "phase3".to_string(),
            serde_json::Value::Array(phase3_results),
        );

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

/// Agent that schedules parallel tasks with timeouts.
/// Uses task-level timeouts via ScheduleAgentTaskOptions::timeout().
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
        let task_count = input.get("taskCount").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
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
                "Scheduling {} tasks with {}ms delay, timeout {}ms (server-side)",
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

        // Schedule slow tasks with server-side timeout using lazy API
        let timeout = Duration::from_millis(timeout_ms);
        let opts = ScheduleAgentTaskOptions::default().timeout(timeout);

        let futures: Vec<_> = (0..task_count)
            .map(|i| {
                ctx.schedule_task_lazy_with_options(
                    "slow-task",
                    json!({
                        "source": format!("task-{}", i),
                        "delay_ms": task_delay_ms
                    }),
                    opts.clone(),
                )
            })
            .collect();

        // Try to wait for all - this may fail due to timeouts
        let result = ctx.join_all(futures).await;

        // Log result
        let (succeeded, error_msg) = match &result {
            Ok(results) => (true, format!("{} tasks completed", results.len())),
            Err(e) => (false, e.to_string()),
        };

        // Checkpoint after tasks
        ctx.checkpoint(&json!({
            "phase": "post-timeout",
            "succeeded": succeeded
        }))
        .await?;

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": if succeeded { "All tasks completed" } else { "Some tasks failed/timed out" }}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("totalTasks".to_string(), json!(task_count));
        output.insert("succeeded".to_string(), json!(succeeded));
        output.insert("message".to_string(), json!(error_msg));

        Ok(output)
    }
}
