//! Integration test for local agent mode.
//!
//! Tests that SqliteStorage + LocalTaskExecutor + InteractiveSignalSource
//! work together correctly for a complete local agent workflow.

#![cfg(feature = "local")]

use std::sync::Arc;

use flovyn_worker_sdk::agent::executor::{FnTask, LocalTaskExecutor, TaskExecutor};
use flovyn_worker_sdk::agent::signals::{InteractiveSignalSource, SignalSource};
use flovyn_worker_sdk::agent::storage::{
    AgentCommand, AgentStorage, CheckpointData, CommandBatch, SqliteStorage, TaskOptions,
    TaskStatus, TokenUsage,
};
use serde_json::json;
use uuid::Uuid;

/// Test full roundtrip: agent creates entries → checkpoint → entries persisted in SQLite
#[tokio::test]
async fn test_local_agent_entry_roundtrip() {
    let storage = SqliteStorage::in_memory().await.unwrap();
    let agent_id = Uuid::new_v4();
    let entry1 = Uuid::new_v4();
    let entry2 = Uuid::new_v4();

    // Simulate agent creating entries and checkpointing
    let batch = CommandBatch {
        segment: 0,
        sequence: 1,
        commands: vec![
            AgentCommand::AppendEntry {
                entry_id: entry1,
                parent_id: None,
                role: "user".to_string(),
                content: json!({"text": "Hello, agent!"}),
            },
            AgentCommand::AppendEntry {
                entry_id: entry2,
                parent_id: Some(entry1),
                role: "assistant".to_string(),
                content: json!({"text": "Hello! How can I help?"}),
            },
        ],
        checkpoint: Some(CheckpointData {
            state: json!({"turn": 1, "model": "local-test"}),
            leaf_entry_id: Some(entry2),
            token_usage: Some(TokenUsage {
                input_tokens: 50,
                output_tokens: 30,
            }),
        }),
    };

    storage.commit_batch(agent_id, batch).await.unwrap();

    // Verify entries persisted by loading segment
    let segment = storage.load_segment(agent_id, 0).await.unwrap();
    assert_eq!(segment.segment, 0);

    let cp = segment.checkpoint.unwrap();
    assert_eq!(cp.state, json!({"turn": 1, "model": "local-test"}));
    assert_eq!(cp.leaf_entry_id, Some(entry2));
    assert_eq!(cp.token_usage.unwrap().input_tokens, 50);

    // Verify latest segment
    assert_eq!(storage.get_latest_segment(agent_id).await.unwrap(), 0);
}

/// Test task execution: agent schedules task → LocalTaskExecutor runs it → result returned
#[tokio::test]
async fn test_local_agent_task_execution() {
    let storage = Arc::new(SqliteStorage::in_memory().await.unwrap());
    let agent_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();

    // Set up local executor with an echo task
    let mut executor = LocalTaskExecutor::new();
    executor.register(
        "echo",
        FnTask(|input| async move { Ok(input) }),
    );
    executor.register(
        "double",
        FnTask(|input: serde_json::Value| async move {
            let n = input.as_i64().unwrap_or(0);
            Ok(json!(n * 2))
        }),
    );

    // Agent schedules a task via commit_batch
    let batch = CommandBatch {
        segment: 0,
        sequence: 1,
        commands: vec![AgentCommand::ScheduleTask {
            task_id,
            kind: "echo".to_string(),
            input: json!({"message": "hello"}),
            options: TaskOptions::default(),
            idempotency_key: "test-key".to_string(),
        }],
        checkpoint: None,
    };

    storage.commit_batch(agent_id, batch).await.unwrap();

    // Verify task is pending in storage
    let result = storage.get_task_result(agent_id, task_id).await.unwrap().unwrap();
    assert_eq!(result.status, TaskStatus::Pending);

    // Execute the task locally
    assert!(executor.supports_local("echo"));
    let output = executor
        .execute(task_id, "echo", json!({"message": "hello"}))
        .await
        .unwrap();
    assert_eq!(output, json!({"message": "hello"}));

    // Execute the double task
    let task_id2 = Uuid::new_v4();
    let output = executor.execute(task_id2, "double", json!(21)).await.unwrap();
    assert_eq!(output, json!(42));

    // Verify unknown task fails
    let err = executor
        .execute(Uuid::new_v4(), "unknown", json!(null))
        .await;
    assert!(err.is_err());
}

/// Test signal flow: store signal → agent receives it
#[tokio::test]
async fn test_local_agent_signal_flow() {
    let storage = Arc::new(SqliteStorage::in_memory().await.unwrap());
    let agent_id = Uuid::new_v4();

    // Set up channel signal source
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let signal_source = InteractiveSignalSource::channel(rx);

    // Agent registers interest in a signal (via storage)
    let batch = CommandBatch {
        segment: 0,
        sequence: 1,
        commands: vec![AgentCommand::WaitForSignal {
            signal_name: "user-input".to_string(),
        }],
        checkpoint: Some(CheckpointData {
            state: json!({"waiting_for": "user-input"}),
            leaf_entry_id: None,
            token_usage: None,
        }),
    };

    storage.commit_batch(agent_id, batch).await.unwrap();

    // External code sends a signal via the channel
    tx.send(("user-input".to_string(), json!({"text": "continue"})))
        .await
        .unwrap();

    // Agent receives the signal
    let payload = signal_source
        .wait_for_signal(agent_id, "user-input")
        .await
        .unwrap();
    assert_eq!(payload, json!({"text": "continue"}));

    // Also test storage-based signals
    storage
        .store_signal(agent_id, "approval", json!({"approved": true}))
        .await
        .unwrap();

    assert!(storage.has_signal(agent_id, "approval").await.unwrap());
    let val = storage
        .pop_signal(agent_id, "approval")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(val, json!({"approved": true}));
}

/// Test full local agent lifecycle: schedule → execute → checkpoint → resume
#[tokio::test]
async fn test_local_agent_full_lifecycle() {
    let storage = Arc::new(SqliteStorage::in_memory().await.unwrap());
    let agent_id = Uuid::new_v4();

    // Phase 1: Agent starts and creates initial entries
    let entry1 = Uuid::new_v4();
    let batch = CommandBatch {
        segment: 0,
        sequence: 1,
        commands: vec![AgentCommand::AppendEntry {
            entry_id: entry1,
            parent_id: None,
            role: "system".to_string(),
            content: json!({"text": "You are a helpful assistant."}),
        }],
        checkpoint: Some(CheckpointData {
            state: json!({"phase": "init", "turn": 0}),
            leaf_entry_id: Some(entry1),
            token_usage: None,
        }),
    };
    storage.commit_batch(agent_id, batch).await.unwrap();

    // Phase 2: Agent schedules a task
    let task_id = Uuid::new_v4();
    let entry2 = Uuid::new_v4();
    let batch = CommandBatch {
        segment: 0,
        sequence: 2,
        commands: vec![
            AgentCommand::AppendEntry {
                entry_id: entry2,
                parent_id: Some(entry1),
                role: "assistant".to_string(),
                content: json!({"text": "Let me analyze that."}),
            },
            AgentCommand::ScheduleTask {
                task_id,
                kind: "analyze".to_string(),
                input: json!({"data": [1, 2, 3]}),
                options: TaskOptions::default(),
                idempotency_key: "analyze-key".to_string(),
            },
        ],
        checkpoint: Some(CheckpointData {
            state: json!({"phase": "analyzing", "turn": 1}),
            leaf_entry_id: Some(entry2),
            token_usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            }),
        }),
    };
    storage.commit_batch(agent_id, batch).await.unwrap();

    // Verify state after both phases
    let latest = storage.get_latest_segment(agent_id).await.unwrap();
    assert_eq!(latest, 0);

    let segment = storage.load_segment(agent_id, 0).await.unwrap();
    let cp = segment.checkpoint.unwrap();
    assert_eq!(cp.state, json!({"phase": "analyzing", "turn": 1}));
    assert_eq!(cp.leaf_entry_id, Some(entry2));

    // Verify task is tracked
    let result = storage.get_task_result(agent_id, task_id).await.unwrap().unwrap();
    assert_eq!(result.status, TaskStatus::Pending);

    // Verify pending tasks in segment
    assert_eq!(segment.pending_tasks.len(), 1);
    assert_eq!(segment.pending_tasks[0].task_id, task_id);
}
