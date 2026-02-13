//! Test agent definitions for E2E tests

#![allow(dead_code)] // Fixtures will be used when tests are implemented

use async_trait::async_trait;
use flovyn_worker_sdk::agent::context::ScheduleAgentTaskOptions;
use flovyn_worker_sdk::agent::context::{AgentContext, CancelTaskResult, EntryRole};
use flovyn_worker_sdk::agent::definition::{DynamicAgent, DynamicAgentInput, DynamicAgentOutput};
use flovyn_worker_sdk::error::Result;
use serde_json::json;
use std::time::Duration;

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

        // Schedule the task and wait for result (using lazy API)
        let future = ctx.schedule_raw(task_kind, task_input);
        let results = ctx.join_all(vec![future]).await?;
        let task_result = results.into_iter().next().unwrap();

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
            .map(|item| ctx.schedule_raw(task_kind, json!({"message": item})))
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
        let fallback = ctx.schedule_raw(
            "slow-task",
            json!({
                "source": "fallback",
                "delay_ms": fallback_delay
            }),
        );

        let primary = ctx.schedule_raw(
            "slow-task",
            json!({
                "source": "primary",
                "delay_ms": primary_delay
            }),
        );

        // Race: fallback is index 0, primary is index 1
        let (result, remaining) = ctx.select_ok(vec![fallback, primary]).await?;

        // Determine winner from result's source field
        let winner_name = result
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Determine winner index based on source (fallback=0, primary=1)
        let winner_index = match winner_name {
            "fallback" => 0,
            "primary" => 1,
            _ => -1,
        };

        ctx.checkpoint(&json!({"phase": "post-race"})).await?;

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("{} task won the race", winner_name)}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("winner".to_string(), json!(winner_name));
        output.insert("winnerIndex".to_string(), json!(winner_index));
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
                        let should_fail = t
                            .get("shouldFail")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
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
                ctx.schedule_raw(
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

        let f1 = ctx.schedule_raw("echo-task", json!({"message": "phase1-a"}));
        let f2 = ctx.schedule_raw("echo-task", json!({"message": "phase1-b"}));

        let phase1_results = ctx.join_all(vec![f1, f2]).await?;
        output.insert(
            "phase1".to_string(),
            serde_json::Value::Array(phase1_results),
        );

        ctx.checkpoint(&json!({"completedPhase": 1})).await?;

        // Phase 2: Single sequential task (using lazy API)
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Phase 2: Starting sequential task"}),
        )
        .await?;

        let f_seq = ctx.schedule_raw("echo-task", json!({"message": "phase2-sequential"}));
        let phase2_results = ctx.join_all(vec![f_seq]).await?;
        let phase2_result = phase2_results.into_iter().next().unwrap();
        output.insert("phase2".to_string(), phase2_result);

        ctx.checkpoint(&json!({"completedPhase": 2})).await?;

        // Phase 3: Three parallel tasks using new lazy API
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Phase 3: Starting three parallel tasks"}),
        )
        .await?;

        let f3 = ctx.schedule_raw("echo-task", json!({"message": "phase3-a"}));
        let f4 = ctx.schedule_raw("echo-task", json!({"message": "phase3-b"}));
        let f5 = ctx.schedule_raw("echo-task", json!({"message": "phase3-c"}));

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
                ctx.schedule_with_options_raw(
                    "slow-task",
                    json!({
                        "source": format!("task-{}", i),
                        "delay_ms": task_delay_ms
                    }),
                    opts.clone(),
                )
            })
            .collect();

        // Use join_all_settled to handle partial completions gracefully
        let result = ctx.join_all_settled(futures).await?;

        // Count timed out tasks (failures with "timed out" error)
        let timed_out_count = result
            .failed
            .iter()
            .filter(|(_, err)| err.to_lowercase().contains("timed out"))
            .count();

        // Checkpoint after tasks
        ctx.checkpoint(&json!({
            "phase": "post-timeout",
            "completedCount": result.completed.len(),
            "failedCount": result.failed.len(),
            "timedOutCount": timed_out_count
        }))
        .await?;

        let succeeded = result.all_succeeded();
        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": if succeeded { "All tasks completed" } else { "Some tasks failed/timed out" }}),
        )
        .await?;

        // Return output
        let mut output = DynamicAgentOutput::new();
        output.insert("totalTasks".to_string(), json!(task_count));
        output.insert("completedCount".to_string(), json!(result.completed.len()));
        output.insert("failedCount".to_string(), json!(result.failed.len()));
        output.insert("timedOutCount".to_string(), json!(timed_out_count));
        output.insert("cancelledCount".to_string(), json!(result.cancelled.len()));
        output.insert("timedOut".to_string(), json!(timed_out_count > 0));
        output.insert("succeeded".to_string(), json!(succeeded));

        Ok(output)
    }
}

// =============================================================================
// Task Cancellation Test Agents
// =============================================================================

/// Agent that races multiple tasks and cancels losers.
/// Uses `select_ok_with_cancel` to race tasks and auto-cancel remaining.
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
        let fast_delay = input
            .get("fastDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);
        let medium_delay = input
            .get("mediumDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);
        let slow_delay = input
            .get("slowDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000);

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Racing 3 tasks: fast={}ms, medium={}ms, slow={}ms", fast_delay, medium_delay, slow_delay)}),
        )
        .await?;

        // Schedule 3 tasks with different delays
        let task_configs = vec![
            ("fast", fast_delay),
            ("medium", medium_delay),
            ("slow", slow_delay),
        ];

        let futures: Vec<_> = task_configs
            .iter()
            .map(|(name, delay)| {
                ctx.schedule_raw("slow-task", json!({"sleepMs": delay, "source": *name}))
            })
            .collect();

        // Checkpoint before racing
        ctx.checkpoint(&json!({"phase": "pre-race", "taskCount": futures.len()}))
            .await?;

        // Race tasks and cancel losers
        let (result, cancelled_ids) = ctx.select_ok_with_cancel(futures).await?;

        // Determine winner (fast should win with shortest delay)
        let winner = result
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let winner_index = task_configs
            .iter()
            .position(|(name, _)| *name == winner)
            .unwrap_or(0);

        // Build cancel results info
        let cancel_results: Vec<_> = cancelled_ids
            .iter()
            .map(|id| {
                json!({
                    "taskId": id.to_string(),
                    "cancelled": true
                })
            })
            .collect();

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Winner: {} (index {}), cancelled {} tasks", winner, winner_index, cancel_results.len())}),
        )
        .await?;

        let mut output = DynamicAgentOutput::new();
        output.insert("winnerIndex".to_string(), json!(winner_index));
        output.insert("winner".to_string(), json!(winner));
        output.insert("result".to_string(), result);
        output.insert("cancelResults".to_string(), json!(cancel_results));
        Ok(output)
    }
}

/// Agent that schedules parallel tasks where some may fail.
/// Uses `join_all_settled` to collect both successes and failures.
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
        // Input: {tasks: [{name, shouldFail}, ...]}
        let tasks: Vec<serde_json::Value> = input
            .get("tasks")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Scheduling {} parallel tasks with potential failures", tasks.len())}),
        )
        .await?;

        // Schedule all tasks
        let futures: Vec<_> = tasks
            .iter()
            .map(|task_config| ctx.schedule_raw("conditional-fail-task", task_config.clone()))
            .collect();

        ctx.checkpoint(&json!({"phase": "pre-tasks", "taskCount": tasks.len()}))
            .await?;

        // Wait for all tasks, collecting both successes and failures
        let result = ctx.join_all_settled(futures).await?;

        // Build output
        let completed_results: Vec<_> = result
            .completed
            .iter()
            .map(|(task_id, output)| json!({"taskId": task_id, "output": output}))
            .collect();

        let failed_errors: Vec<_> = result
            .failed
            .iter()
            .enumerate()
            .map(|(idx, (task_id, error))| {
                // Find original index in input tasks
                json!({
                    "index": tasks.iter().position(|t| {
                        // Match by task name if available
                        t.get("name").and_then(|n| n.as_str())
                            .map(|name| error.contains(name))
                            .unwrap_or(false)
                    }).unwrap_or(idx),
                    "taskId": task_id,
                    "error": error
                })
            })
            .collect();

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Completed: {}, Failed: {}", result.completed.len(), result.failed.len())}),
        )
        .await?;

        let mut output = DynamicAgentOutput::new();
        output.insert("totalTasks".to_string(), json!(tasks.len()));
        output.insert("completedCount".to_string(), json!(result.completed.len()));
        output.insert("failedCount".to_string(), json!(result.failed.len()));
        output.insert("completedResults".to_string(), json!(completed_results));
        output.insert("failedErrors".to_string(), json!(failed_errors));
        Ok(output)
    }
}

/// Agent that uses batch scheduling API to schedule multiple tasks at once.
/// Demonstrates the lazy scheduling pattern with batch commit.
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
        // Input: {items: [...], taskKind}
        let items: Vec<String> = input
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let task_kind = input
            .get("taskKind")
            .and_then(|v| v.as_str())
            .unwrap_or("echo-task");

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Batch scheduling {} {} tasks", items.len(), task_kind)}),
        )
        .await?;

        ctx.checkpoint(&json!({"phase": "pre-batch", "itemCount": items.len()}))
            .await?;

        // Schedule all tasks lazily (no RPC yet - this is the batch pattern)
        let futures: Vec<_> = items
            .iter()
            .map(|item| ctx.schedule_raw(task_kind, json!({"message": item})))
            .collect();

        // Wait for all tasks using join_all (commits batch, then waits)
        let results = ctx.join_all(futures).await?;

        ctx.checkpoint(&json!({"phase": "post-batch", "resultCount": results.len()}))
            .await?;

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("All {} batch tasks completed", results.len())}),
        )
        .await?;

        let mut output = DynamicAgentOutput::new();
        output.insert("itemCount".to_string(), json!(items.len()));
        output.insert("usedBatchApi".to_string(), json!(true));
        output.insert("results".to_string(), serde_json::Value::Array(results));
        Ok(output)
    }
}

/// Agent that cancels a slow task and replaces it with a faster one.
/// Demonstrates the cancel-and-replace pattern.
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
        let slow_delay = input
            .get("slowDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(10000);
        let fast_delay = input
            .get("fastDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);
        let wait_before_cancel = input
            .get("waitBeforeCancelMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(200);

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Starting slow task ({}ms), will cancel after {}ms", slow_delay, wait_before_cancel)}),
        )
        .await?;

        // Schedule the slow task
        let slow_task = ctx.schedule_raw(
            "slow-task",
            json!({"sleepMs": slow_delay, "source": "slow-original"}),
        );
        let slow_task_id = slow_task.task_id;

        // Commit the slow task
        ctx.checkpoint(
            &json!({"phase": "slow-task-scheduled", "taskId": slow_task_id.to_string()}),
        )
        .await?;

        // Wait a bit before cancelling
        tokio::time::sleep(std::time::Duration::from_millis(wait_before_cancel)).await;

        // Try to cancel the slow task
        let cancel_result = ctx.cancel_task(slow_task_id).await?;
        let slow_cancelled = matches!(cancel_result, CancelTaskResult::Cancelled);
        let slow_status = match cancel_result {
            CancelTaskResult::Cancelled => "CANCELLED",
            CancelTaskResult::AlreadyCompleted => "COMPLETED",
            CancelTaskResult::AlreadyFailed => "FAILED",
            CancelTaskResult::AlreadyCancelled => "CANCELLED",
        };

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Slow task cancel result: {} (status: {})", slow_cancelled, slow_status)}),
        )
        .await?;

        // Schedule the fast replacement task
        let fast_task = ctx.schedule_raw(
            "slow-task",
            json!({"sleepMs": fast_delay, "source": "fast-replacement"}),
        );

        // Wait for replacement task
        let results = ctx.join_all(vec![fast_task]).await?;
        let replacement_result = results
            .into_iter()
            .next()
            .unwrap_or(serde_json::Value::Null);

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": "Replacement task completed successfully"}),
        )
        .await?;

        let mut output = DynamicAgentOutput::new();
        output.insert("slowTaskCancelled".to_string(), json!(slow_cancelled));
        output.insert("slowTaskStatus".to_string(), json!(slow_status));
        output.insert("success".to_string(), json!(true));
        output.insert("replacementResult".to_string(), replacement_result);
        Ok(output)
    }
}

/// Agent that tests cancel idempotency by cancelling the same task multiple times.
/// Multiple cancels on the same task should be safe (first succeeds, rest report already cancelled).
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
        let task_delay = input
            .get("taskDelayMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(5000);

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Testing cancel idempotency: {} cancel attempts on task with {}ms delay", cancel_count, task_delay)}),
        )
        .await?;

        // Schedule a slow task
        let task = ctx.schedule_raw(
            "slow-task",
            json!({"sleepMs": task_delay, "source": "idempotency-test"}),
        );
        let task_id = task.task_id;

        // Commit the task
        ctx.checkpoint(&json!({"phase": "task-scheduled", "taskId": task_id.to_string()}))
            .await?;

        // Cancel the task multiple times
        let mut cancel_results = Vec::new();
        let mut first_cancel_succeeded = false;

        for i in 0..cancel_count {
            let result = ctx.cancel_task(task_id).await?;
            let (cancelled, status) = match result {
                CancelTaskResult::Cancelled => {
                    if i == 0 {
                        first_cancel_succeeded = true;
                    }
                    (true, "CANCELLED")
                }
                CancelTaskResult::AlreadyCompleted => (false, "COMPLETED"),
                CancelTaskResult::AlreadyFailed => (false, "FAILED"),
                CancelTaskResult::AlreadyCancelled => (false, "CANCELLED"),
            };
            cancel_results.push(json!({
                "attempt": i + 1,
                "cancelled": cancelled,
                "status": status
            }));
        }

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Cancel attempts complete: first succeeded = {}", first_cancel_succeeded)}),
        )
        .await?;

        let mut output = DynamicAgentOutput::new();
        output.insert("cancelCount".to_string(), json!(cancel_count));
        output.insert(
            "firstCancelSucceeded".to_string(),
            json!(first_cancel_succeeded),
        );
        output.insert("success".to_string(), json!(true));
        output.insert("cancelResults".to_string(), json!(cancel_results));
        Ok(output)
    }
}

/// Agent that tests join_all behavior when one handle is pre-cancelled.
/// Tests both regular join_all (which fails) and join_all_settled (which handles gracefully).
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

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Testing join_all with cancelled handle (useOutcomes: {})", use_outcomes)}),
        )
        .await?;

        // Schedule 2 tasks
        let task1 = ctx.schedule_raw(
            "slow-task",
            json!({"sleepMs": 5000, "source": "task1-to-cancel"}),
        );
        let task2 = ctx.schedule_raw(
            "slow-task",
            json!({"sleepMs": 100, "source": "task2-completes"}),
        );
        let task1_id = task1.task_id;

        // Commit tasks
        ctx.checkpoint(&json!({"phase": "tasks-scheduled"})).await?;

        // Cancel task1
        let cancel_result = ctx.cancel_task(task1_id).await?;
        let task1_cancelled = matches!(cancel_result, CancelTaskResult::Cancelled);

        ctx.append_entry(
            EntryRole::Assistant,
            &json!({"text": format!("Task1 cancelled: {}", task1_cancelled)}),
        )
        .await?;

        if use_outcomes {
            // Use join_all_settled which handles cancellations gracefully
            let result = ctx.join_all_settled(vec![task1, task2]).await?;

            let mut output = DynamicAgentOutput::new();
            output.insert("usedOutcomes".to_string(), json!(true));
            output.insert("completedCount".to_string(), json!(result.completed.len()));
            output.insert("cancelledCount".to_string(), json!(result.cancelled.len()));
            output.insert("failedCount".to_string(), json!(result.failed.len()));
            output.insert("success".to_string(), json!(true));
            Ok(output)
        } else {
            // Use regular join_all which will fail on cancelled task
            let join_result = ctx.join_all(vec![task1, task2]).await;
            let join_all_succeeded = join_result.is_ok();

            let mut output = DynamicAgentOutput::new();
            output.insert("task1Cancelled".to_string(), json!(task1_cancelled));
            output.insert("joinAllSucceeded".to_string(), json!(join_all_succeeded));
            output.insert("success".to_string(), json!(true)); // Test succeeded (we expected failure)
            if let Err(e) = join_result {
                output.insert("error".to_string(), json!(e.to_string()));
            }
            Ok(output)
        }
    }
}
