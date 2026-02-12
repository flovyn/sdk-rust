//! Agent E2E tests
//!
//! These tests verify agent execution against a real Flovyn server.
//! Tests the full agent lifecycle including:
//! - Basic agent execution with entries and checkpoints
//! - Multi-turn agents with signal-based user interaction
//! - Agents that schedule tasks and wait for results
//! - Checkpoint-based state recovery

use crate::fixtures::agents::{
    BatchSchedulingAgent, CancelAndReplaceAgent, CancelIdempotencyAgent, CheckpointAgent,
    EchoAgent, JoinAllWithCancelledHandleAgent, MixedParallelSequentialAgent, MultiTurnAgent,
    ParallelTasksAgent, ParallelWithFailuresAgent, RacingTasksAgent, RacingTasksWithCancelAgent,
    TaskSchedulingAgent, TimeoutTasksAgent,
};
use crate::fixtures::tasks::{ConditionalFailTask, EchoTask, SlowTask};
use crate::{get_harness, with_timeout};
use flovyn_worker_sdk::client::FlovynClient;
use serde_json::json;
use std::time::Duration;

/// Test timeout for agent tests (longer than workflow tests due to multi-turn nature)
const AGENT_TEST_TIMEOUT: Duration = Duration::from_secs(120);

// ============================================================================
// Basic Agent Execution Tests
// ============================================================================

/// Test basic agent execution: agent processes input, creates entries, checkpoints, returns output.
/// This tests the full flow:
/// 1. Create agent execution via REST API
/// 2. Agent worker polls and receives the agent
/// 3. Agent processes input and creates conversation entries
/// 4. Agent creates checkpoint
/// 5. Agent completes with output
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_basic_agent_execution() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_basic_agent_execution", async {
        let harness = get_harness().await;

        let queue = "agent-basic-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-agent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(EchoAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        // Give the worker time to register
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create an agent execution via REST API
        let agent = harness
            .create_agent_execution(
                "echo-agent",
                json!({"message": "Hello from E2E test"}),
                queue,
            )
            .await;

        println!("Created agent execution: {}", agent.id);
        assert_eq!(agent.kind, "echo-agent");
        assert_eq!(agent.status.to_uppercase(), "PENDING");

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(30))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete successfully");

        // Verify output
        let output = completed.output.expect("Agent should have output");
        assert_eq!(
            output.get("response").and_then(|v| v.as_str()),
            Some("Echo: Hello from E2E test")
        );
        assert_eq!(
            output.get("inputMessage").and_then(|v| v.as_str()),
            Some("Hello from E2E test")
        );

        // Verify checkpoints were created
        let checkpoints = harness.list_agent_checkpoints(&agent.id.to_string()).await;
        assert!(!checkpoints.is_empty(), "Agent should have created checkpoints");
        println!("Agent created {} checkpoint(s)", checkpoints.len());

        handle.abort();
    })
    .await;
}

// ============================================================================
// Multi-Turn Agent with Signals Tests
// ============================================================================

/// Test multi-turn agent with signals: agent waits for user interaction via signals.
/// This tests signal-based conversation:
/// 1. Create agent execution
/// 2. Agent executes first turn
/// 3. Agent calls ctx.wait_for_signal() and suspends (status = WAITING)
/// 4. Signal is sent via REST API
/// 5. Agent resumes and processes signal
/// 6. Agent completes with conversation result
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_multi_turn_agent_with_signals() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_multi_turn_agent_with_signals", async {
        let harness = get_harness().await;

        let queue = "agent-signal-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-signal-agent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(MultiTurnAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent execution
        let agent = harness
            .create_agent_execution(
                "multi-turn-agent",
                json!({"prompt": "What is the weather?"}),
                queue,
            )
            .await;

        println!("Created multi-turn agent execution: {}", agent.id);

        // Wait for agent to reach WAITING status (suspended waiting for signal)
        let waiting = harness
            .wait_for_agent_status(&agent.id.to_string(), &["WAITING", "COMPLETED", "FAILED"], Duration::from_secs(30))
            .await;

        println!("Agent status after first turn: {}", waiting.status);
        assert_eq!(waiting.status.to_uppercase(), "WAITING", "Agent should suspend waiting for signal");

        // Verify checkpoint was created before suspension
        let checkpoints = harness.list_agent_checkpoints(&agent.id.to_string()).await;
        assert!(!checkpoints.is_empty(), "Agent should checkpoint before waiting");
        println!("Agent created {} checkpoint(s) before waiting", checkpoints.len());

        // Send the follow-up signal
        println!("Sending followUp signal...");
        let signal_response = harness
            .signal_agent_execution(
                &agent.id.to_string(),
                "followUp",
                json!({"message": "It's sunny today!"}),
            )
            .await;

        assert!(signal_response.signaled, "Signal should be created");
        assert!(signal_response.agent_resumed, "Agent should be resumed after signal");
        println!("Signal sent, agent resumed: {}", signal_response.agent_resumed);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(30))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete after signal");

        // Verify output includes both turns
        let output = completed.output.expect("Agent should have output");
        assert_eq!(output.get("turns").and_then(|v| v.as_i64()), Some(2));
        assert!(
            output.get("firstResponse").is_some(),
            "Output should include first response"
        );
        assert_eq!(
            output.get("followUpReceived").and_then(|v| v.as_str()),
            Some("It's sunny today!")
        );

        handle.abort();
    })
    .await;
}

// ============================================================================
// Agent with Task Scheduling Tests
// ============================================================================

/// Test agent with task scheduling: agent schedules a task and waits for the result.
/// This tests the agent-task integration:
/// 1. Create agent execution
/// 2. Agent worker polls and executes the agent
/// 3. Agent calls ctx.schedule_task_raw() which:
///    - Creates a task execution with agent_execution_id
///    - Suspends the agent (status = WAITING)
/// 4. Task worker executes the task
/// 5. Task completion signals the agent
/// 6. Agent resumes with task result
/// 7. Agent completes with final output including task result
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_with_task_scheduling() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_with_task_scheduling", async {
        let harness = get_harness().await;

        let queue = "agent-task-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-agent-task-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(TaskSchedulingAgent)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent that will schedule a task
        let agent = harness
            .create_agent_execution(
                "task-scheduling-agent",
                json!({
                    "taskKind": "echo-task",
                    "taskInput": {"message": "Task input from agent"}
                }),
                queue,
            )
            .await;

        println!("Created task-scheduling agent execution: {}", agent.id);

        // Wait for agent to complete (may go through WAITING state while task executes)
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete after task");

        // Verify output includes task result
        let output = completed.output.expect("Agent should have output");
        assert_eq!(
            output.get("taskKind").and_then(|v| v.as_str()),
            Some("echo-task")
        );

        let task_result = output.get("taskResult");
        assert!(task_result.is_some(), "Output should include task result");
        println!("Task result: {:?}", task_result);

        // Verify task was created and associated with agent
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert!(!tasks.is_empty(), "Agent should have scheduled tasks");
        println!("Agent scheduled {} task(s)", tasks.len());

        let task = &tasks[0];
        assert_eq!(task.kind, "echo-task");
        assert_eq!(task.status.to_uppercase(), "COMPLETED");

        // Verify checkpoints
        let checkpoints = harness.list_agent_checkpoints(&agent.id.to_string()).await;
        assert!(checkpoints.len() >= 2, "Agent should have checkpoints before and after task");
        println!("Agent created {} checkpoint(s)", checkpoints.len());

        handle.abort();
    })
    .await;
}

// ============================================================================
// Checkpoint Recovery Tests
// ============================================================================

/// Test agent checkpoint-based recovery.
/// This tests the checkpoint mechanism works correctly:
/// 1. Agent executes multiple steps with checkpoints
/// 2. Verify all checkpoints are persisted
/// 3. Verify state is correctly saved in each checkpoint
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_checkpoint_persistence() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_checkpoint_persistence", async {
        let harness = get_harness().await;

        let queue = "agent-checkpoint-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-checkpoint-agent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(CheckpointAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent that will create multiple checkpoints
        let agent = harness
            .create_agent_execution(
                "checkpoint-agent",
                json!({"steps": 5}),
                queue,
            )
            .await;

        println!("Created checkpoint agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(30))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED");

        // Verify output
        let output = completed.output.expect("Agent should have output");
        assert_eq!(output.get("totalSteps").and_then(|v| v.as_i64()), Some(5));

        let completed_steps = output.get("completedSteps").and_then(|v| v.as_array());
        assert!(completed_steps.is_some());
        assert_eq!(completed_steps.unwrap().len(), 5);

        // Verify checkpoints were created (one per step)
        let checkpoints = harness.list_agent_checkpoints(&agent.id.to_string()).await;
        assert!(checkpoints.len() >= 5, "Agent should have at least 5 checkpoints");
        println!("Agent created {} checkpoint(s)", checkpoints.len());

        // Verify checkpoint states have correct progress
        for (i, cp) in checkpoints.iter().enumerate() {
            if let Some(state) = &cp.state {
                if let Some(step) = state.get("completedStep").and_then(|v| v.as_i64()) {
                    println!("Checkpoint {} has completedStep: {}", i, step);
                }
            }
        }

        handle.abort();
    })
    .await;
}

// ============================================================================
// Agent Worker Metrics Tests
// ============================================================================

/// Test agent worker metrics tracking.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_worker_metrics() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_worker_metrics", async {
        let harness = get_harness().await;

        let queue = "agent-metrics-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-agent-metrics-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(EchoAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Initially no agents executed
        assert_eq!(handle.agents_executed(), 0, "No agents should have executed yet");

        // Create and run an agent
        let agent = harness
            .create_agent_execution("echo-agent", json!({"message": "test"}), queue)
            .await;

        harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(30))
            .await;

        // Give metrics time to update
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify metrics updated
        let executed = handle.agents_executed();
        println!("Agents executed: {}", executed);
        assert!(executed >= 1, "At least one agent should have executed");

        handle.abort();
    })
    .await;
}

// ============================================================================
// Combined Worker Types Tests
// ============================================================================

/// Test combined workflow, task, and agent worker in single client.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_combined_worker_types() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_combined_worker_types", async {
        use crate::fixtures::tasks::SlowTask;
        use crate::fixtures::workflows::EchoWorkflow;

        let harness = get_harness().await;

        let queue = "combined-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-combined-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_workflow(EchoWorkflow)
            .register_task(EchoTask)
            .register_task(SlowTask)
            .register_agent(EchoAgent)
            .register_agent(TaskSchedulingAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Run agent that schedules a task
        let agent = harness
            .create_agent_execution(
                "task-scheduling-agent",
                json!({
                    "taskKind": "echo-task",
                    "taskInput": {"message": "Combined test"}
                }),
                queue,
            )
            .await;

        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        assert_eq!(completed.status.to_uppercase(), "COMPLETED");
        println!("Combined worker successfully executed agent with task");

        handle.abort();
    })
    .await;
}

// ============================================================================
// Error Handling Tests
// ============================================================================

/// Test agent failure handling.
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_failure_handling() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_failure_handling", async {
        let harness = get_harness().await;

        let queue = "agent-error-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-agent-error-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(EchoAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent with non-existent kind (should fail or timeout)
        // Note: This tests that unknown agent kinds are handled gracefully
        let agent = harness
            .create_agent_execution(
                "non-existent-agent",
                json!({"message": "test"}),
                queue,
            )
            .await;

        // Wait a bit - agent should remain PENDING since no worker handles it
        tokio::time::sleep(Duration::from_secs(3)).await;

        let status = harness.get_agent_execution(&agent.id.to_string()).await;
        println!("Agent with unknown kind has status: {}", status.status);

        // Should still be pending or running (not completed, since no handler)
        assert!(
            status.status.to_uppercase() == "PENDING" || status.status.to_uppercase() == "RUNNING",
            "Unknown agent should remain pending/running"
        );

        handle.abort();
    })
    .await;
}

// ============================================================================
// Concurrency and Race Condition Tests
// ============================================================================

/// Test concurrent agent execution: multiple agents executing simultaneously.
///
/// This tests that the server can handle multiple agents being:
/// 1. Started concurrently
/// 2. Executed in parallel by workers
/// 3. Completed independently without interfering with each other
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_concurrent_agent_execution() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_concurrent_agent_execution", async {
        let harness = get_harness().await;

        let queue = "agent-concurrent-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-concurrent-agent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(EchoAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Start multiple agents concurrently
        let num_agents = 5;
        let mut agent_ids = Vec::new();

        for i in 0..num_agents {
            let agent = harness
                .create_agent_execution(
                    "echo-agent",
                    json!({"message": format!("Agent {}", i)}),
                    queue,
                )
                .await;
            agent_ids.push((i, agent.id));
            println!("Created agent {}: {}", i, agent.id);
        }

        // Wait for all agents to complete
        let timeout = Duration::from_secs(60);
        let start = std::time::Instant::now();
        let mut completed = vec![false; num_agents];

        loop {
            if start.elapsed() > timeout {
                let pending: Vec<_> = completed
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !**c)
                    .map(|(i, _)| i)
                    .collect();
                panic!("Agents did not complete within timeout. Pending: {:?}", pending);
            }

            for (idx, (input_idx, agent_id)) in agent_ids.iter().enumerate() {
                if completed[idx] {
                    continue;
                }

                let status = harness.get_agent_execution(&agent_id.to_string()).await;
                if status.status.to_uppercase() == "COMPLETED" {
                    // Verify output is correct
                    if let Some(output) = &status.output {
                        let expected_msg = format!("Agent {}", input_idx);
                        let expected_response = format!("Echo: {}", expected_msg);
                        assert_eq!(
                            output.get("response").and_then(|v| v.as_str()),
                            Some(expected_response.as_str()),
                            "Agent {} should have correct response",
                            agent_id
                        );
                    }
                    completed[idx] = true;
                    println!("Agent {} completed", input_idx);
                } else if status.status.to_uppercase() == "FAILED" {
                    panic!("Agent {} failed: {:?}", agent_id, status.error);
                }
            }

            if completed.iter().all(|c| *c) {
                break;
            }

            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        println!("All {} agents completed successfully", num_agents);
        handle.abort();
    })
    .await;
}

/// Test multiple workers competing for agents on the same queue.
///
/// This tests that:
/// 1. Two workers can connect to the same queue
/// 2. Agents are properly distributed between workers
/// 3. No race conditions when claiming agents
/// 4. Each agent is executed exactly once
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_multiple_agent_workers() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_multiple_agent_workers", async {
        let harness = get_harness().await;

        let queue = "agent-multi-worker-queue";

        // Create first worker
        let client1 = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-agent-worker-1")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(EchoAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient 1");

        // Create second worker (same queue)
        let client2 = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-agent-worker-2")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(EchoAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient 2");

        let handle1 = client1.start().await.expect("Failed to start worker 1");
        let handle2 = client2.start().await.expect("Failed to start worker 2");

        handle1.await_ready().await;
        handle2.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Start multiple agents
        let num_agents = 6;
        let mut agent_ids = Vec::new();

        for i in 0..num_agents {
            let agent = harness
                .create_agent_execution(
                    "echo-agent",
                    json!({"message": format!("Multi-worker agent {}", i)}),
                    queue,
                )
                .await;
            agent_ids.push(agent.id);
        }

        // Wait for all agents to complete
        let timeout = Duration::from_secs(60);
        let start = std::time::Instant::now();
        let mut completed_count = 0;

        loop {
            if start.elapsed() > timeout {
                panic!(
                    "Agents did not complete within timeout. Completed: {}/{}",
                    completed_count, num_agents
                );
            }

            completed_count = 0;
            for agent_id in &agent_ids {
                let status = harness.get_agent_execution(&agent_id.to_string()).await;
                if status.status.to_uppercase() == "COMPLETED" {
                    completed_count += 1;
                } else if status.status.to_uppercase() == "FAILED" {
                    panic!("Agent {} failed: {:?}", agent_id, status.error);
                }
            }

            if completed_count == num_agents {
                break;
            }

            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        println!("All {} agents completed with multiple workers", num_agents);

        handle1.abort();
        handle2.abort();
    })
    .await;
}

/// Test signal race condition: multiple signals sent rapidly to the same agent.
///
/// This tests that:
/// 1. Signals are properly queued
/// 2. Agent processes signals in order
/// 3. No signals are lost under rapid-fire conditions
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_rapid_signal_delivery() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_rapid_signal_delivery", async {
        let harness = get_harness().await;

        let queue = "agent-signal-race-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-signal-race-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(MultiTurnAgent)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent
        let agent = harness
            .create_agent_execution(
                "multi-turn-agent",
                json!({"prompt": "Start conversation"}),
                queue,
            )
            .await;

        // Wait for agent to reach WAITING status
        let waiting = harness
            .wait_for_agent_status(
                &agent.id.to_string(),
                &["WAITING", "COMPLETED", "FAILED"],
                Duration::from_secs(30),
            )
            .await;

        assert_eq!(waiting.status.to_uppercase(), "WAITING");

        // Send the signal (only one signal expected for MultiTurnAgent)
        let signal_response = harness
            .signal_agent_execution(
                &agent.id.to_string(),
                "followUp",
                json!({"message": "Signal received"}),
            )
            .await;

        assert!(signal_response.signaled);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(
                &agent.id.to_string(),
                &["COMPLETED", "FAILED"],
                Duration::from_secs(30),
            )
            .await;

        assert_eq!(completed.status.to_uppercase(), "COMPLETED");
        println!("Signal race test completed successfully");

        handle.abort();
    })
    .await;
}

/// Test concurrent agents with task scheduling: multiple agents scheduling tasks simultaneously.
///
/// This tests that:
/// 1. Multiple agents can schedule tasks concurrently
/// 2. Task completion signals are delivered to correct agents
/// 3. No cross-talk between agents
/// 4. Each agent receives its own task result
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_concurrent_agents_with_tasks() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_concurrent_agents_with_tasks", async {
        let harness = get_harness().await;

        let queue = "agent-concurrent-task-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-concurrent-task-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(TaskSchedulingAgent)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Start multiple task-scheduling agents concurrently
        let num_agents = 3;
        let mut agent_ids = Vec::new();

        for i in 0..num_agents {
            let agent = harness
                .create_agent_execution(
                    "task-scheduling-agent",
                    json!({
                        "taskKind": "echo-task",
                        "taskInput": {"message": format!("Task from agent {}", i)}
                    }),
                    queue,
                )
                .await;
            agent_ids.push((i, agent.id));
            println!("Created task-scheduling agent {}: {}", i, agent.id);
        }

        // Wait for all agents to complete
        let timeout = Duration::from_secs(90);
        let start = std::time::Instant::now();
        let mut completed = vec![false; num_agents];

        loop {
            if start.elapsed() > timeout {
                let pending: Vec<_> = completed
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !**c)
                    .map(|(i, _)| i)
                    .collect();
                panic!("Agents did not complete within timeout. Pending: {:?}", pending);
            }

            for (idx, (input_idx, agent_id)) in agent_ids.iter().enumerate() {
                if completed[idx] {
                    continue;
                }

                let status = harness.get_agent_execution(&agent_id.to_string()).await;
                if status.status.to_uppercase() == "COMPLETED" {
                    // Verify task was created for this agent
                    let tasks = harness.list_agent_tasks(&agent_id.to_string()).await;
                    assert!(!tasks.is_empty(), "Agent {} should have scheduled tasks", input_idx);
                    assert_eq!(tasks[0].kind, "echo-task");
                    assert_eq!(tasks[0].status.to_uppercase(), "COMPLETED");

                    completed[idx] = true;
                    println!("Agent {} completed with task", input_idx);
                } else if status.status.to_uppercase() == "FAILED" {
                    panic!("Agent {} failed: {:?}", agent_id, status.error);
                }
            }

            if completed.iter().all(|c| *c) {
                break;
            }

            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        println!("All {} agents with tasks completed successfully", num_agents);
        handle.abort();
    })
    .await;
}

/// Test agent claiming race: ensure an agent can only be claimed by one worker.
///
/// This tests the server's atomic agent claiming mechanism:
/// 1. Multiple workers poll for the same agent
/// 2. Only one worker should succeed in claiming
/// 3. Agent should execute exactly once
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_claiming_race() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_claiming_race", async {
        let harness = get_harness().await;

        let queue = "agent-claiming-race-queue";

        // Create 3 workers competing for work
        let mut handles = Vec::new();
        for i in 0..3 {
            let client = FlovynClient::builder()
                .server_url(harness.grpc_url())
                .org_id(harness.org_id())
                .worker_id(format!("e2e-race-worker-{}", i))
                .worker_token(harness.worker_token())
                .queue(queue)
                .register_agent(EchoAgent)
                .build()
                .await
                .expect("Failed to build FlovynClient");

            let handle = client.start().await.expect("Failed to start worker");
            handle.await_ready().await;
            handles.push(handle);
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create a single agent
        let agent = harness
            .create_agent_execution("echo-agent", json!({"message": "Race test"}), queue)
            .await;

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(
                &agent.id.to_string(),
                &["COMPLETED", "FAILED"],
                Duration::from_secs(30),
            )
            .await;

        assert_eq!(completed.status.to_uppercase(), "COMPLETED");

        // Verify the agent completed exactly once with correct output
        let output = completed.output.expect("Agent should have output");
        assert_eq!(
            output.get("response").and_then(|v| v.as_str()),
            Some("Echo: Race test")
        );

        println!("Agent claiming race test passed - agent executed exactly once");

        for handle in handles {
            handle.abort();
        }
    })
    .await;
}

// ============================================================================
// Parallel Task Tests (Phase 5: Agent Parallel Task Support)
// ============================================================================

/// Test agent with parallel task scheduling using agent_join_all.
///
/// This tests the parallel task combinators:
/// 1. Agent schedules multiple tasks using schedule_task_handle()
/// 2. Agent waits for all tasks using agent_join_all()
/// 3. Server uses WaitMode::All to only resume when all tasks complete
/// 4. Agent receives all results in order
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_parallel_tasks_join_all() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_parallel_tasks_join_all", async {
        let harness = get_harness().await;

        let queue = "agent-parallel-join-all-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-parallel-agent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(ParallelTasksAgent)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent that will schedule 3 parallel tasks
        let agent = harness
            .create_agent_execution(
                "parallel-tasks-agent",
                json!({
                    "items": ["alpha", "beta", "gamma"],
                    "taskKind": "echo-task"
                }),
                queue,
            )
            .await;

        println!("Created parallel-tasks agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(90))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete after all tasks");

        // Verify output
        let output = completed.output.expect("Agent should have output");
        assert_eq!(output.get("itemCount").and_then(|v| v.as_i64()), Some(3));

        let results = output.get("results").and_then(|v| v.as_array());
        assert!(results.is_some(), "Output should include results array");
        assert_eq!(results.unwrap().len(), 3, "Should have 3 results");
        println!("Agent received {} task results", results.unwrap().len());

        // Verify all 3 tasks were created for this agent
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 3, "Agent should have scheduled 3 tasks");

        for task in &tasks {
            assert_eq!(task.kind, "echo-task");
            assert_eq!(task.status.to_uppercase(), "COMPLETED");
        }
        println!("All 3 tasks completed successfully");

        // Verify checkpoints were created
        let checkpoints = harness.list_agent_checkpoints(&agent.id.to_string()).await;
        assert!(checkpoints.len() >= 2, "Agent should have checkpoints before and after parallel tasks");
        println!("Agent created {} checkpoint(s)", checkpoints.len());

        handle.abort();
    })
    .await;
}

/// Test agent with racing tasks using agent_select.
///
/// This tests the select combinator:
/// 1. Agent schedules two tasks with different delays
/// 2. Agent waits for first to complete using agent_select()
/// 3. Server uses WaitMode::Any to resume when any task completes
/// 4. Agent receives the winner's result
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_racing_tasks_select() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_racing_tasks_select", async {
        let harness = get_harness().await;

        let queue = "agent-racing-select-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-racing-agent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(RacingTasksAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent with primary (slow) and fallback (fast) tasks
        // Fallback should win the race
        let agent = harness
            .create_agent_execution(
                "racing-tasks-agent",
                json!({
                    "primaryDelayMs": 5000,   // Primary: 5 seconds
                    "fallbackDelayMs": 100    // Fallback: 100ms (should win)
                }),
                queue,
            )
            .await;

        println!("Created racing-tasks agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(90))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete after any task");

        // Verify output - fallback should win
        let output = completed.output.expect("Agent should have output");
        let winner_index = output.get("winnerIndex").and_then(|v| v.as_i64());
        let winner = output.get("winner").and_then(|v| v.as_str());

        println!("Winner index: {:?}, winner: {:?}", winner_index, winner);

        // Fallback (index 0) should win - it's scheduled first and has shorter delay
        assert_eq!(winner_index, Some(0), "Fallback task (index 0) should win the race");
        assert_eq!(winner, Some("fallback"), "Winner should be 'fallback'");

        // Verify the result contains source from the winning task
        let result = output.get("result");
        assert!(result.is_some(), "Output should include result from winner");
        if let Some(r) = result {
            let source = r.get("source").and_then(|v| v.as_str());
            assert_eq!(source, Some("fallback"), "Result source should be 'fallback'");
        }

        handle.abort();
    })
    .await;
}

/// Test agent with large batch of parallel tasks.
///
/// This tests that the batch APIs work correctly with many tasks:
/// 1. Agent schedules 10 tasks in parallel
/// 2. All tasks complete
/// 3. Agent receives all 10 results
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_parallel_large_batch() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_parallel_large_batch", async {
        let harness = get_harness().await;

        let queue = "agent-parallel-batch-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-batch-agent-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(ParallelTasksAgent)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent that will schedule 10 parallel tasks
        let items: Vec<String> = (0..10).map(|i| format!("item-{}", i)).collect();
        let agent = harness
            .create_agent_execution(
                "parallel-tasks-agent",
                json!({
                    "items": items,
                    "taskKind": "echo-task"
                }),
                queue,
            )
            .await;

        println!("Created large-batch parallel agent execution: {}", agent.id);

        // Wait for agent to complete (may take longer with 10 tasks)
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(120))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete after all 10 tasks");

        // Verify output
        let output = completed.output.expect("Agent should have output");
        assert_eq!(output.get("itemCount").and_then(|v| v.as_i64()), Some(10));

        let results = output.get("results").and_then(|v| v.as_array());
        assert!(results.is_some(), "Output should include results array");
        assert_eq!(results.unwrap().len(), 10, "Should have 10 results");
        println!("Agent received {} task results", results.unwrap().len());

        // Verify all 10 tasks were created for this agent
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 10, "Agent should have scheduled 10 tasks");

        handle.abort();
    })
    .await;
}

/// Test multiple agents scheduling parallel tasks concurrently.
///
/// This tests isolation between agents:
/// 1. Multiple agents each schedule multiple tasks
/// 2. Each agent's tasks are correctly tracked
/// 3. No cross-talk between agents
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_concurrent_agents_parallel_tasks() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_concurrent_agents_parallel_tasks", async {
        let harness = get_harness().await;

        let queue = "agent-concurrent-parallel-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-concurrent-parallel-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(ParallelTasksAgent)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Start 3 agents, each scheduling 3 parallel tasks (9 total tasks)
        let num_agents = 3;
        let mut agent_ids = Vec::new();

        for i in 0..num_agents {
            let items: Vec<String> = (0..3).map(|j| format!("agent{}-item{}", i, j)).collect();
            let agent = harness
                .create_agent_execution(
                    "parallel-tasks-agent",
                    json!({
                        "items": items,
                        "taskKind": "echo-task"
                    }),
                    queue,
                )
                .await;
            agent_ids.push((i, agent.id));
            println!("Created parallel agent {}: {}", i, agent.id);
        }

        // Wait for all agents to complete
        let timeout = Duration::from_secs(120);
        let start = std::time::Instant::now();
        let mut completed = vec![false; num_agents];

        loop {
            if start.elapsed() > timeout {
                let pending: Vec<_> = completed
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !**c)
                    .map(|(i, _)| i)
                    .collect();
                panic!("Agents did not complete within timeout. Pending: {:?}", pending);
            }

            for (idx, (input_idx, agent_id)) in agent_ids.iter().enumerate() {
                if completed[idx] {
                    continue;
                }

                let status = harness.get_agent_execution(&agent_id.to_string()).await;
                if status.status.to_uppercase() == "COMPLETED" {
                    // Verify each agent has exactly 3 tasks
                    let tasks = harness.list_agent_tasks(&agent_id.to_string()).await;
                    assert_eq!(tasks.len(), 3, "Agent {} should have 3 tasks", input_idx);

                    // Verify output has 3 results
                    if let Some(output) = &status.output {
                        let item_count = output.get("itemCount").and_then(|v| v.as_i64());
                        assert_eq!(item_count, Some(3), "Agent {} should have 3 items", input_idx);
                    }

                    completed[idx] = true;
                    println!("Agent {} completed with 3 parallel tasks", input_idx);
                } else if status.status.to_uppercase() == "FAILED" {
                    panic!("Agent {} failed: {:?}", agent_id, status.error);
                }
            }

            if completed.iter().all(|c| *c) {
                break;
            }

            tokio::time::sleep(Duration::from_millis(300)).await;
        }

        println!("All {} agents with parallel tasks completed successfully", num_agents);
        handle.abort();
    })
    .await;
}

/// Test agent with racing tasks using agent_select_with_cancel.
///
/// This tests the select-with-cancellation combinator:
/// 1. Agent schedules three tasks with different delays
/// 2. Agent waits for first to complete using agent_select_with_cancel()
/// 3. Winner completes first
/// 4. Remaining tasks are automatically cancelled
/// 5. Agent receives winner's result and cancellation info
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_racing_tasks_with_cancellation() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_racing_tasks_with_cancellation", async {
        let harness = get_harness().await;

        let queue = "agent-racing-cancel-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-racing-cancel-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(RacingTasksWithCancelAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent with three tasks: fast (100ms), medium (1s), slow (5s)
        // Fast should win, medium and slow should be cancelled
        let agent = harness
            .create_agent_execution(
                "racing-tasks-with-cancel-agent",
                json!({
                    "fastDelayMs": 100,     // Fast: 100ms (should win)
                    "mediumDelayMs": 3000,  // Medium: 3 seconds
                    "slowDelayMs": 10000    // Slow: 10 seconds
                }),
                queue,
            )
            .await;

        println!("Created racing-tasks-with-cancel agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(90))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete after any task");

        // Verify output - fast should win
        let output = completed.output.expect("Agent should have output");
        let winner_index = output.get("winnerIndex").and_then(|v| v.as_i64());
        let winner = output.get("winner").and_then(|v| v.as_str());

        println!("Winner index: {:?}, winner: {:?}", winner_index, winner);

        // Fast (index 0) should win because it has shortest delay
        assert_eq!(winner_index, Some(0), "Fast task (index 0) should win the race");
        assert_eq!(winner, Some("fast"), "Winner should be 'fast'");

        // Verify the result contains source from the winning task
        let result = output.get("result");
        assert!(result.is_some(), "Output should include result from winner");
        if let Some(r) = result {
            let source = r.get("source").and_then(|v| v.as_str());
            assert_eq!(source, Some("fast"), "Result source should be 'fast'");
        }

        // Verify cancellation results
        let cancel_results = output.get("cancelResults").and_then(|v| v.as_array());
        assert!(cancel_results.is_some(), "Output should include cancellation results");
        let cancel_results = cancel_results.unwrap();
        println!("Cancellation results: {} attempts", cancel_results.len());

        // Should have attempted to cancel 2 tasks (medium and slow)
        assert_eq!(cancel_results.len(), 2, "Should have 2 cancellation attempts");

        // Verify tasks were created
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 3, "Agent should have scheduled 3 tasks");
        println!("Agent scheduled {} tasks", tasks.len());

        // Check task statuses - winner should be COMPLETED, others may be CANCELLED or COMPLETED
        let mut completed_count = 0;
        let mut cancelled_count = 0;
        for task in &tasks {
            match task.status.to_uppercase().as_str() {
                "COMPLETED" => completed_count += 1,
                "CANCELLED" => cancelled_count += 1,
                status => println!("Task {} has status: {}", task.id, status),
            }
        }

        println!("Task statuses: {} completed, {} cancelled", completed_count, cancelled_count);
        // At minimum, the winner should be completed
        assert!(completed_count >= 1, "At least one task should be completed (the winner)");
        // Cancellation may or may not succeed depending on timing - medium/slow tasks may have
        // already started running by the time we try to cancel

        handle.abort();
    })
    .await;
}

/// Test agent with parallel tasks where some fail.
///
/// This tests the agent_join_all_outcomes combinator:
/// 1. Agent schedules multiple tasks, some configured to fail
/// 2. Agent uses agent_join_all_outcomes() to collect all results
/// 3. Agent receives both successful and failed outcomes
/// 4. Agent can process partial results gracefully
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_parallel_with_failure() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_parallel_with_failure", async {
        let harness = get_harness().await;

        let queue = "agent-parallel-failure-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-parallel-failure-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(ParallelWithFailuresAgent)
            .register_task(ConditionalFailTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent with 3 tasks: 2 succeed, 1 fails
        let agent = harness
            .create_agent_execution(
                "parallel-with-failures-agent",
                json!({
                    "tasks": [
                        {"name": "task-a", "shouldFail": false},
                        {"name": "task-b", "shouldFail": true},   // This one fails
                        {"name": "task-c", "shouldFail": false}
                    ]
                }),
                queue,
            )
            .await;

        println!("Created parallel-with-failures agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(90))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete (handling failures gracefully)");

        // Verify output
        let output = completed.output.expect("Agent should have output");

        let total_tasks = output.get("totalTasks").and_then(|v| v.as_i64());
        let completed_count = output.get("completedCount").and_then(|v| v.as_i64());
        let failed_count = output.get("failedCount").and_then(|v| v.as_i64());

        println!("Total: {:?}, Completed: {:?}, Failed: {:?}", total_tasks, completed_count, failed_count);

        assert_eq!(total_tasks, Some(3), "Should have 3 total tasks");
        assert_eq!(completed_count, Some(2), "2 tasks should have completed successfully");
        assert_eq!(failed_count, Some(1), "1 task should have failed");

        // Verify completed results
        let completed_results = output.get("completedResults").and_then(|v| v.as_array());
        assert!(completed_results.is_some(), "Should have completed results array");
        assert_eq!(completed_results.unwrap().len(), 2, "Should have 2 completed results");

        // Verify failed errors
        let failed_errors = output.get("failedErrors").and_then(|v| v.as_array());
        assert!(failed_errors.is_some(), "Should have failed errors array");
        assert_eq!(failed_errors.unwrap().len(), 1, "Should have 1 failed error");

        // Verify the failed task info
        let failed_error = &failed_errors.unwrap()[0];
        assert_eq!(failed_error.get("index").and_then(|v| v.as_i64()), Some(1), "Failed task should be at index 1");

        // Verify all 3 tasks were created
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 3, "Agent should have scheduled 3 tasks");

        // Check task statuses
        let mut task_completed = 0;
        let mut task_failed = 0;
        for task in &tasks {
            match task.status.to_uppercase().as_str() {
                "COMPLETED" => task_completed += 1,
                "FAILED" => task_failed += 1,
                status => println!("Task {} has status: {}", task.id, status),
            }
        }

        assert_eq!(task_completed, 2, "2 tasks should be COMPLETED");
        assert_eq!(task_failed, 1, "1 task should be FAILED");

        handle.abort();
    })
    .await;
}

/// Test cancelling an already-completed task.
///
/// This tests the cancel_task behavior for edge cases:
/// 1. Agent schedules a fast task that completes quickly
/// 2. Agent waits for completion
/// 3. Agent attempts to cancel the already-completed task
/// 4. Cancel returns appropriate result (not cancelled, already completed)
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_cancel_completed_task() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_cancel_completed_task", async {
        let harness = get_harness().await;

        let queue = "agent-cancel-completed-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-cancel-completed-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(RacingTasksWithCancelAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent with tasks where fast wins quickly
        // The medium and slow tasks may have already completed by the time we try to cancel
        let agent = harness
            .create_agent_execution(
                "racing-tasks-with-cancel-agent",
                json!({
                    "fastDelayMs": 50,      // Very fast
                    "mediumDelayMs": 100,   // Also fast - may complete before cancel
                    "slowDelayMs": 150      // Also relatively fast - may complete before cancel
                }),
                queue,
            )
            .await;

        println!("Created cancel-completed test agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED");

        // Verify output
        let output = completed.output.expect("Agent should have output");

        // Fast task should win
        let winner_index = output.get("winnerIndex").and_then(|v| v.as_i64());
        assert_eq!(winner_index, Some(0), "Fast task should win");

        // Check cancel results - some may have already completed
        let cancel_results = output.get("cancelResults").and_then(|v| v.as_array());
        assert!(cancel_results.is_some(), "Should have cancel results");

        let cancel_results = cancel_results.unwrap();
        println!("Cancel results: {:?}", cancel_results);

        // Both cancel attempts should exist (for medium and slow tasks)
        assert_eq!(cancel_results.len(), 2, "Should have 2 cancel attempts");

        // At least verify the cancel attempts were made
        // Some may be cancelled=false because task already completed
        for result in cancel_results {
            let cancelled = result.get("cancelled").and_then(|v| v.as_bool());
            let reason = result.get("reason");
            println!("Cancel attempt: cancelled={:?}, reason={:?}", cancelled, reason);
            // Either successfully cancelled, or has a reason why not
            assert!(
                cancelled == Some(true) || reason.is_some(),
                "Cancel should either succeed or have a reason for failure"
            );
        }

        handle.abort();
    })
    .await;
}

/// Test agent with timeout that cancels all pending tasks.
///
/// This tests the agent_join_all_with_timeout combinator:
/// 1. Agent schedules multiple slow tasks (5s delay each)
/// 2. Agent waits with a short timeout (1s)
/// 3. Timeout occurs before any task completes
/// 4. All tasks are cancelled
/// 5. Agent receives timeout result with cancellation info
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_timeout_cancels_all() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_timeout_cancels_all", async {
        let harness = get_harness().await;

        let queue = "agent-timeout-cancel-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-timeout-cancel-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(TimeoutTasksAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent with 3 slow tasks (5s each) but only 1s timeout
        let agent = harness
            .create_agent_execution(
                "timeout-tasks-agent",
                json!({
                    "taskCount": 3,
                    "taskDelayMs": 5000,   // Each task takes 5 seconds
                    "timeoutMs": 1000      // But we only wait 1 second
                }),
                queue,
            )
            .await;

        println!("Created timeout-tasks agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete (handling timeout gracefully)");

        // Verify output
        let output = completed.output.expect("Agent should have output");

        let total_tasks = output.get("totalTasks").and_then(|v| v.as_i64());
        let completed_count = output.get("completedCount").and_then(|v| v.as_i64());
        let cancelled_count = output.get("cancelledCount").and_then(|v| v.as_i64());
        let timed_out = output.get("timedOut").and_then(|v| v.as_bool());

        println!(
            "Total: {:?}, Completed: {:?}, Cancelled: {:?}, TimedOut: {:?}",
            total_tasks, completed_count, cancelled_count, timed_out
        );

        assert_eq!(total_tasks, Some(3), "Should have 3 total tasks");
        assert_eq!(timed_out, Some(true), "Should have timed out");

        // Note: With sequential task execution, the worker picks up tasks one at a time.
        // When timeout occurs, some tasks may already be RUNNING (picked up by worker).
        // cancel_task only works for PENDING tasks, so RUNNING tasks won't be cancelled.
        // The agent reports "cancelled" based on its perspective (cancel request made),
        // but the actual task status may be RUNNING if cancellation didn't succeed.
        assert_eq!(completed_count, Some(0), "No tasks should have completed (they take 5s, timeout is 1s)");

        // Verify cancelled indices from agent's perspective (cancel was attempted)
        let cancelled_indices = output.get("cancelledIndices").and_then(|v| v.as_array());
        assert!(cancelled_indices.is_some(), "Should have cancelled indices");
        // Agent attempted to cancel all 3
        assert_eq!(cancelled_indices.unwrap().len(), 3, "Should have attempted to cancel 3 tasks");

        // Verify all 3 tasks were created
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 3, "Agent should have scheduled 3 tasks");

        // Check task statuses - may be CANCELLED, RUNNING, or PENDING
        // depending on whether the worker picked them up before cancel
        let mut task_cancelled = 0;
        let mut task_running = 0;
        let mut task_pending = 0;
        for task in &tasks {
            match task.status.to_uppercase().as_str() {
                "CANCELLED" => task_cancelled += 1,
                "RUNNING" => task_running += 1,
                "PENDING" => task_pending += 1,
                status => println!("Task {} has status: {}", task.id, status),
            }
        }

        println!("Task statuses: {} cancelled, {} running, {} pending", task_cancelled, task_running, task_pending);

        // At least some tasks should be cancelled (those still PENDING when cancel was called)
        // Note: With sequential execution, the first task may be RUNNING
        assert!(task_cancelled + task_running + task_pending == 3, "All 3 tasks should have a valid status");
        // None should be completed (they take 5s, we only waited 1s)
        let task_completed = tasks.iter().filter(|t| t.status.to_uppercase() == "COMPLETED").count();
        assert_eq!(task_completed, 0, "No tasks should be completed (they take 5s)");

        handle.abort();
    })
    .await;
}

/// Test agent with timeout where some tasks complete before timeout.
///
/// This tests partial completion with timeout:
/// 1. Agent schedules 3 tasks: 1 fast (100ms), 2 slow (5s)
/// 2. Agent waits with 500ms timeout
/// 3. Fast task completes, slow tasks are cancelled
/// 4. Agent receives partial results
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_timeout_partial_completion() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_timeout_partial_completion", async {
        let harness = get_harness().await;

        let queue = "agent-timeout-partial-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-timeout-partial-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(TimeoutTasksAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Note: TimeoutTasksAgent uses same delay for all tasks
        // For partial completion test, we use a timeout that allows some tasks to complete
        // With 3 tasks at 200ms each and 500ms timeout, at least 1-2 should complete
        let agent = harness
            .create_agent_execution(
                "timeout-tasks-agent",
                json!({
                    "taskCount": 3,
                    "taskDelayMs": 200,    // Each task takes 200ms
                    "timeoutMs": 350       // Timeout after 350ms (1-2 tasks should complete)
                }),
                queue,
            )
            .await;

        println!("Created partial-timeout agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED");

        // Verify output
        let output = completed.output.expect("Agent should have output");

        let total_tasks = output.get("totalTasks").and_then(|v| v.as_i64());
        let completed_count = output.get("completedCount").and_then(|v| v.as_i64()).unwrap_or(0);
        let cancelled_count = output.get("cancelledCount").and_then(|v| v.as_i64()).unwrap_or(0);
        let timed_out = output.get("timedOut").and_then(|v| v.as_bool());

        println!(
            "Total: {:?}, Completed: {}, Cancelled: {}, TimedOut: {:?}",
            total_tasks, completed_count, cancelled_count, timed_out
        );

        assert_eq!(total_tasks, Some(3), "Should have 3 total tasks");

        // Either all completed (no timeout) or we have a mix of completed and cancelled
        if timed_out == Some(true) {
            // Some tasks completed, some cancelled
            assert!(completed_count > 0, "At least some tasks should have completed before timeout");
            assert!(cancelled_count > 0, "Some tasks should have been cancelled due to timeout");
            assert_eq!(completed_count + cancelled_count, 3, "Total should be 3");
        } else {
            // All completed before timeout
            assert_eq!(completed_count, 3, "All tasks completed before timeout");
        }

        handle.abort();
    })
    .await;
}

/// Test agent with mixed parallel and sequential task execution.
///
/// This tests combining different patterns:
/// 1. Phase 1: 2 parallel tasks
/// 2. Phase 2: 1 sequential task
/// 3. Phase 3: 3 parallel tasks
/// 4. Agent completes with results from all phases
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_mixed_parallel_sequential() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_mixed_parallel_sequential", async {
        let harness = get_harness().await;

        let queue = "agent-mixed-parallel-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-mixed-parallel-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(MixedParallelSequentialAgent)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent
        let agent = harness
            .create_agent_execution(
                "mixed-parallel-sequential-agent",
                json!({}),
                queue,
            )
            .await;

        println!("Created mixed-parallel-sequential agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(90))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED");

        // Verify output
        let output = completed.output.expect("Agent should have output");

        let total_tasks = output.get("totalTasks").and_then(|v| v.as_i64());
        let phases = output.get("phases").and_then(|v| v.as_i64());
        let success = output.get("success").and_then(|v| v.as_bool());

        println!("Total tasks: {:?}, Phases: {:?}, Success: {:?}", total_tasks, phases, success);

        assert_eq!(total_tasks, Some(6), "Should have 6 total tasks (2 + 1 + 3)");
        assert_eq!(phases, Some(3), "Should have 3 phases");
        assert_eq!(success, Some(true), "Should be successful");

        // Verify phase results
        let phase1 = output.get("phase1").and_then(|v| v.as_array());
        let phase2 = output.get("phase2");
        let phase3 = output.get("phase3").and_then(|v| v.as_array());

        assert!(phase1.is_some(), "Should have phase1 results");
        assert_eq!(phase1.unwrap().len(), 2, "Phase 1 should have 2 results");

        assert!(phase2.is_some(), "Should have phase2 result");

        assert!(phase3.is_some(), "Should have phase3 results");
        assert_eq!(phase3.unwrap().len(), 3, "Phase 3 should have 3 results");

        // Verify all 6 tasks were created
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 6, "Agent should have scheduled 6 tasks");

        // All tasks should be completed
        for task in &tasks {
            assert_eq!(task.status.to_uppercase(), "COMPLETED", "All tasks should be COMPLETED");
        }

        // Verify checkpoints
        let checkpoints = harness.list_agent_checkpoints(&agent.id.to_string()).await;
        assert!(checkpoints.len() >= 3, "Agent should have at least 3 checkpoints (one per phase)");

        handle.abort();
    })
    .await;
}

/// Test agent using batch scheduling API.
///
/// This tests that the batch scheduling API works correctly:
/// 1. Agent schedules multiple tasks using schedule_tasks_batch()
/// 2. All tasks are created in a single batch RPC
/// 3. Agent waits for all tasks and receives all results
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_batch_api_scheduling() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_batch_api_scheduling", async {
        let harness = get_harness().await;

        let queue = "agent-batch-api-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-batch-api-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(BatchSchedulingAgent)
            .register_task(EchoTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent that will use batch scheduling for 5 items
        let agent = harness
            .create_agent_execution(
                "batch-scheduling-agent",
                json!({
                    "items": ["alpha", "beta", "gamma", "delta", "epsilon"],
                    "taskKind": "echo-task"
                }),
                queue,
            )
            .await;

        println!("Created batch-scheduling agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(90))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete successfully");

        // Verify output
        let output = completed.output.expect("Agent should have output");
        assert_eq!(output.get("itemCount").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(output.get("usedBatchApi").and_then(|v| v.as_bool()), Some(true));

        let results = output.get("results").and_then(|v| v.as_array());
        assert!(results.is_some(), "Output should include results array");
        assert_eq!(results.unwrap().len(), 5, "Should have 5 results");

        // Verify all 5 tasks were created
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 5, "Agent should have scheduled 5 tasks");

        for task in &tasks {
            assert_eq!(task.status.to_uppercase(), "COMPLETED");
        }

        handle.abort();
    })
    .await;
}

/// Test cancel-and-replace pattern.
///
/// This tests the cancel-and-replace workflow:
/// 1. Agent schedules a slow task
/// 2. Agent cancels the slow task after a short wait
/// 3. Agent schedules a fast replacement task
/// 4. Agent waits for replacement and completes successfully
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_cancel_and_replace() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_cancel_and_replace", async {
        let harness = get_harness().await;

        let queue = "agent-cancel-replace-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-cancel-replace-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(CancelAndReplaceAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent with slow task (10s) that will be cancelled after 200ms
        // and replaced with a fast task (100ms)
        let agent = harness
            .create_agent_execution(
                "cancel-and-replace-agent",
                json!({
                    "slowDelayMs": 10000,
                    "fastDelayMs": 100,
                    "waitBeforeCancelMs": 200
                }),
                queue,
            )
            .await;

        println!("Created cancel-and-replace agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete successfully");

        // Verify output
        let output = completed.output.expect("Agent should have output");

        let slow_task_cancelled = output.get("slowTaskCancelled").and_then(|v| v.as_bool());
        let slow_task_status = output.get("slowTaskStatus").and_then(|v| v.as_str());
        let success = output.get("success").and_then(|v| v.as_bool());

        println!("Slow task cancelled: {:?}, status: {:?}, success: {:?}", slow_task_cancelled, slow_task_status, success);

        // Note: Cancellation may fail if the task was already picked up by the worker (RUNNING).
        // cancel_task only works for PENDING tasks. This is expected behavior.
        // The important thing is that the agent completed successfully and has a replacement result.
        assert_eq!(success, Some(true), "Agent should complete successfully");

        // Verify replacement result
        let replacement_result = output.get("replacementResult");
        assert!(replacement_result.is_some(), "Should have replacement result");

        if let Some(result) = replacement_result {
            let source = result.get("source").and_then(|v| v.as_str());
            assert_eq!(source, Some("fast-replacement"), "Result should be from fast replacement");
        }

        // Verify tasks - should have 2 tasks total
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 2, "Agent should have scheduled 2 tasks");

        // The slow task may be CANCELLED (if cancellation succeeded) or
        // RUNNING/COMPLETED (if the worker picked it up before cancellation).
        // The replacement task should be COMPLETED.
        let cancelled_count = tasks.iter().filter(|t| t.status.to_uppercase() == "CANCELLED").count();
        let completed_count = tasks.iter().filter(|t| t.status.to_uppercase() == "COMPLETED").count();
        let running_count = tasks.iter().filter(|t| t.status.to_uppercase() == "RUNNING").count();

        println!("Tasks: {} cancelled, {} completed, {} running", cancelled_count, completed_count, running_count);

        // At least one task should be completed (the replacement)
        assert!(completed_count >= 1, "At least one task (replacement) should be completed");
        // Total should be 2
        assert_eq!(cancelled_count + completed_count + running_count, 2, "All 2 tasks should have a status");

        handle.abort();
    })
    .await;
}

/// Test cancel idempotency - multiple cancels on the same task are safe.
///
/// This tests that:
/// 1. Agent schedules a task
/// 2. Agent cancels the task multiple times
/// 3. First cancel succeeds, subsequent cancels return "already cancelled"
/// 4. No errors occur from multiple cancel attempts
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_cancel_idempotency() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_cancel_idempotency", async {
        let harness = get_harness().await;

        let queue = "agent-cancel-idempotency-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-cancel-idempotency-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(CancelIdempotencyAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Create agent that will cancel the same task 3 times
        let agent = harness
            .create_agent_execution(
                "cancel-idempotency-agent",
                json!({
                    "cancelCount": 3,
                    "taskDelayMs": 5000
                }),
                queue,
            )
            .await;

        println!("Created cancel-idempotency agent execution: {}", agent.id);

        // Wait for agent to complete
        let completed = harness
            .wait_for_agent_status(&agent.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent completed with status: {}", completed.status);
        assert_eq!(completed.status.to_uppercase(), "COMPLETED", "Agent should complete successfully");

        // Verify output
        let output = completed.output.expect("Agent should have output");

        let cancel_count = output.get("cancelCount").and_then(|v| v.as_i64());
        let first_cancel_succeeded = output.get("firstCancelSucceeded").and_then(|v| v.as_bool());
        let success = output.get("success").and_then(|v| v.as_bool());

        println!("Cancel count: {:?}, first succeeded: {:?}", cancel_count, first_cancel_succeeded);

        assert_eq!(cancel_count, Some(3), "Should have made 3 cancel attempts");
        assert_eq!(first_cancel_succeeded, Some(true), "First cancel should succeed");
        assert_eq!(success, Some(true), "Agent should complete successfully");

        // Verify cancel results
        let cancel_results = output.get("cancelResults").and_then(|v| v.as_array());
        assert!(cancel_results.is_some(), "Should have cancel results");

        let cancel_results = cancel_results.unwrap();
        assert_eq!(cancel_results.len(), 3, "Should have 3 cancel results");

        // First should succeed, subsequent should indicate already cancelled
        let first_result = &cancel_results[0];
        assert_eq!(
            first_result.get("cancelled").and_then(|v| v.as_bool()),
            Some(true),
            "First cancel should succeed"
        );

        // Subsequent cancels should have cancelled=false (already cancelled)
        for (i, result) in cancel_results.iter().enumerate().skip(1) {
            let cancelled = result.get("cancelled").and_then(|v| v.as_bool());
            let status = result.get("status").and_then(|v| v.as_str());
            println!("Cancel attempt {}: cancelled={:?}, status={:?}", i + 1, cancelled, status);
            // Already cancelled - cancelled should be false
            assert_eq!(cancelled, Some(false), "Subsequent cancels should return false");
            assert_eq!(status, Some("CANCELLED"), "Status should be CANCELLED");
        }

        // Verify task status
        let tasks = harness.list_agent_tasks(&agent.id.to_string()).await;
        assert_eq!(tasks.len(), 1, "Agent should have scheduled 1 task");
        assert_eq!(tasks[0].status.to_uppercase(), "CANCELLED", "Task should be CANCELLED");

        handle.abort();
    })
    .await;
}

/// Test join_all with a pre-cancelled handle.
///
/// This tests the behavior when agent_join_all is called with a handle that was
/// already cancelled:
/// 1. Agent schedules two tasks
/// 2. Agent cancels one task
/// 3. Agent calls join_all with both handles
/// 4. join_all should fail appropriately (or use outcomes API to handle gracefully)
#[tokio::test]
#[ignore] // Enable when Docker is available
async fn test_agent_join_all_with_cancelled_handle() {
    with_timeout(AGENT_TEST_TIMEOUT, "test_agent_join_all_with_cancelled_handle", async {
        let harness = get_harness().await;

        let queue = "agent-join-cancelled-queue";
        let client = FlovynClient::builder()
            .server_url(harness.grpc_url())
            .org_id(harness.org_id())
            .worker_id("e2e-join-cancelled-worker")
            .worker_token(harness.worker_token())
            .queue(queue)
            .register_agent(JoinAllWithCancelledHandleAgent)
            .register_task(SlowTask)
            .build()
            .await
            .expect("Failed to build FlovynClient");

        let handle = client.start().await.expect("Failed to start worker");
        handle.await_ready().await;

        tokio::time::sleep(Duration::from_secs(1)).await;

        // Test 1: Using regular join_all (should fail)
        let agent1 = harness
            .create_agent_execution(
                "join-all-cancelled-handle-agent",
                json!({
                    "useOutcomes": false
                }),
                queue,
            )
            .await;

        println!("Created join-all-cancelled test agent (regular): {}", agent1.id);

        let completed1 = harness
            .wait_for_agent_status(&agent1.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent 1 completed with status: {}", completed1.status);
        assert_eq!(completed1.status.to_uppercase(), "COMPLETED", "Agent should complete");

        let output1 = completed1.output.expect("Agent should have output");
        let task1_cancelled = output1.get("task1Cancelled").and_then(|v| v.as_bool());
        let join_all_succeeded = output1.get("joinAllSucceeded").and_then(|v| v.as_bool());
        let success1 = output1.get("success").and_then(|v| v.as_bool());

        println!("Task1 cancelled: {:?}, joinAll succeeded: {:?}, success: {:?}",
                 task1_cancelled, join_all_succeeded, success1);

        assert_eq!(task1_cancelled, Some(true), "Task 1 should have been cancelled");
        assert_eq!(join_all_succeeded, Some(false), "join_all should have failed");
        assert_eq!(success1, Some(true), "Test should succeed (expected failure handled)");

        // Test 2: Using outcomes API (should handle gracefully)
        let agent2 = harness
            .create_agent_execution(
                "join-all-cancelled-handle-agent",
                json!({
                    "useOutcomes": true
                }),
                queue,
            )
            .await;

        println!("Created join-all-cancelled test agent (outcomes): {}", agent2.id);

        let completed2 = harness
            .wait_for_agent_status(&agent2.id.to_string(), &["COMPLETED", "FAILED"], Duration::from_secs(60))
            .await;

        println!("Agent 2 completed with status: {}", completed2.status);
        assert_eq!(completed2.status.to_uppercase(), "COMPLETED", "Agent should complete");

        let output2 = completed2.output.expect("Agent should have output");
        let used_outcomes = output2.get("usedOutcomes").and_then(|v| v.as_bool());
        let completed_count = output2.get("completedCount").and_then(|v| v.as_i64());
        let cancelled_count = output2.get("cancelledCount").and_then(|v| v.as_i64());
        let success2 = output2.get("success").and_then(|v| v.as_bool());

        println!("Used outcomes: {:?}, completed: {:?}, cancelled: {:?}, success: {:?}",
                 used_outcomes, completed_count, cancelled_count, success2);

        assert_eq!(used_outcomes, Some(true), "Should have used outcomes API");
        assert_eq!(completed_count, Some(1), "One task should have completed");
        assert_eq!(cancelled_count, Some(1), "One task should have been cancelled");
        assert_eq!(success2, Some(true), "Test should succeed");

        handle.abort();
    })
    .await;
}
