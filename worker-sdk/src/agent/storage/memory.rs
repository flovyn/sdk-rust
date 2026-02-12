//! In-memory storage implementation for testing and ephemeral agents.
//!
//! This module provides an `InMemoryStorage` implementation of `AgentStorage`
//! that stores all data in memory using thread-safe HashMaps. It's useful for:
//!
//! - Unit and integration testing
//! - Ephemeral agents that don't need persistence
//! - Local development without a server connection
//!
//! # Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::storage::InMemoryStorage;
//!
//! let storage = InMemoryStorage::new();
//!
//! // Inject test data
//! storage.set_task_result(task_id, TaskResult {
//!     task_id,
//!     status: TaskStatus::Completed,
//!     output: Some(json!({"result": "success"})),
//!     error: None,
//! });
//!
//! // Send a signal to the agent
//! storage.send_signal(agent_id, "userInput", json!({"text": "hello"}));
//! ```

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use super::{
    AgentCommand, AgentStorage, CommandBatch, PendingTask, SegmentState, StorageResult, TaskResult,
    TaskStatus,
};
#[cfg(test)]
use super::CheckpointData;

/// In-memory storage backend for testing and ephemeral agents.
///
/// All data is stored in memory using thread-safe HashMaps. Data is lost
/// when the storage instance is dropped.
///
/// This implementation provides additional test helper methods for injecting
/// task results and signals, which is useful for testing agent behavior
/// without actually executing tasks or waiting for external signals.
pub struct InMemoryStorage {
    /// Checkpoints stored by agent_id -> segment -> batch
    checkpoints: RwLock<HashMap<Uuid, HashMap<u64, CommandBatch>>>,

    /// Task results by task_id
    task_results: RwLock<HashMap<Uuid, TaskResult>>,

    /// Signals stored by agent_id -> signal_name -> payloads (FIFO queue)
    signals: RwLock<HashMap<Uuid, HashMap<String, Vec<Value>>>>,

    /// Latest segment number by agent_id
    latest_segments: RwLock<HashMap<Uuid, u64>>,
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryStorage {
    /// Create a new empty in-memory storage.
    pub fn new() -> Self {
        Self {
            checkpoints: RwLock::new(HashMap::new()),
            task_results: RwLock::new(HashMap::new()),
            signals: RwLock::new(HashMap::new()),
            latest_segments: RwLock::new(HashMap::new()),
        }
    }

    // =========================================================================
    // Test Helpers
    // =========================================================================

    /// Set a task result for testing.
    ///
    /// This allows tests to inject task results without actually executing tasks.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// storage.set_task_result(task_id, TaskResult {
    ///     task_id,
    ///     status: TaskStatus::Completed,
    ///     output: Some(json!({"answer": 42})),
    ///     error: None,
    /// });
    /// ```
    pub fn set_task_result(&self, task_id: Uuid, result: TaskResult) {
        let mut results = self.task_results.write().unwrap();
        results.insert(task_id, result);
    }

    /// Send a signal to an agent for testing.
    ///
    /// Signals are stored in a FIFO queue and can be consumed by calling
    /// `pop_signal` on the storage.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// storage.send_signal(agent_id, "userInput", json!({"text": "hello"}));
    /// ```
    pub fn send_signal(&self, agent_id: Uuid, signal_name: &str, payload: Value) {
        let mut signals = self.signals.write().unwrap();
        let agent_signals = signals.entry(agent_id).or_default();
        let queue = agent_signals.entry(signal_name.to_string()).or_default();
        queue.push(payload);
    }

    /// Get all batches for an agent (for testing inspection).
    pub fn get_batches(&self, agent_id: Uuid) -> Vec<CommandBatch> {
        let checkpoints = self.checkpoints.read().unwrap();
        checkpoints
            .get(&agent_id)
            .map(|segments| {
                let mut batches: Vec<_> = segments.values().cloned().collect();
                batches.sort_by_key(|b| (b.segment, b.sequence));
                batches
            })
            .unwrap_or_default()
    }

    /// Clear all data (for testing reset).
    pub fn clear(&self) {
        self.checkpoints.write().unwrap().clear();
        self.task_results.write().unwrap().clear();
        self.signals.write().unwrap().clear();
        self.latest_segments.write().unwrap().clear();
    }
}

#[async_trait]
impl AgentStorage for InMemoryStorage {
    async fn commit_batch(&self, agent_id: Uuid, batch: CommandBatch) -> StorageResult<()> {
        let segment = batch.segment;

        // Store the batch
        {
            let mut checkpoints = self.checkpoints.write().unwrap();
            let agent_batches = checkpoints.entry(agent_id).or_default();
            agent_batches.insert(segment, batch);
        }

        // Update latest segment if this is newer
        {
            let mut latest = self.latest_segments.write().unwrap();
            let current = latest.entry(agent_id).or_insert(0);
            if segment > *current {
                *current = segment;
            }
        }

        Ok(())
    }

    async fn load_segment(&self, agent_id: Uuid, segment: u64) -> StorageResult<SegmentState> {
        let checkpoints = self.checkpoints.read().unwrap();

        let batch = checkpoints
            .get(&agent_id)
            .and_then(|segments| segments.get(&segment));

        let checkpoint = batch.and_then(|b| b.checkpoint.clone());

        // Extract pending tasks from the batch commands
        let pending_tasks = batch
            .map(|b| {
                b.commands
                    .iter()
                    .filter_map(|cmd| match cmd {
                        AgentCommand::ScheduleTask {
                            task_id,
                            kind,
                            input: _,
                            options: _,
                        } => {
                            // Check if we have a result for this task
                            let results = self.task_results.read().unwrap();
                            let result = results.get(task_id).cloned();
                            let status = result
                                .as_ref()
                                .map(|r| r.status)
                                .unwrap_or(TaskStatus::Pending);

                            Some(PendingTask {
                                task_id: *task_id,
                                kind: kind.clone(),
                                status,
                                result,
                            })
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Extract pending signals from the batch commands
        let pending_signals = batch
            .map(|b| {
                b.commands
                    .iter()
                    .filter_map(|cmd| match cmd {
                        AgentCommand::WaitForSignal { signal_name } => Some(signal_name.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(SegmentState {
            segment,
            checkpoint,
            pending_tasks,
            pending_signals,
        })
    }

    async fn get_latest_segment(&self, agent_id: Uuid) -> StorageResult<u64> {
        let latest = self.latest_segments.read().unwrap();
        Ok(*latest.get(&agent_id).unwrap_or(&0))
    }

    async fn get_task_result(&self, task_id: Uuid) -> StorageResult<Option<TaskResult>> {
        let results = self.task_results.read().unwrap();
        Ok(results.get(&task_id).cloned())
    }

    async fn get_task_results(&self, task_ids: &[Uuid]) -> StorageResult<Vec<TaskResult>> {
        let results = self.task_results.read().unwrap();
        Ok(task_ids
            .iter()
            .filter_map(|id| results.get(id).cloned())
            .collect())
    }

    async fn store_signal(
        &self,
        agent_id: Uuid,
        signal_name: &str,
        payload: Value,
    ) -> StorageResult<()> {
        self.send_signal(agent_id, signal_name, payload);
        Ok(())
    }

    async fn pop_signal(&self, agent_id: Uuid, signal_name: &str) -> StorageResult<Option<Value>> {
        let mut signals = self.signals.write().unwrap();

        let payload = signals
            .get_mut(&agent_id)
            .and_then(|agent_signals| agent_signals.get_mut(signal_name))
            .and_then(|queue| {
                if queue.is_empty() {
                    None
                } else {
                    Some(queue.remove(0)) // FIFO: remove from front
                }
            });

        Ok(payload)
    }

    async fn has_signal(&self, agent_id: Uuid, signal_name: &str) -> StorageResult<bool> {
        let signals = self.signals.read().unwrap();

        let has = signals
            .get(&agent_id)
            .and_then(|agent_signals| agent_signals.get(signal_name))
            .map(|queue| !queue.is_empty())
            .unwrap_or(false);

        Ok(has)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_commit_and_load_batch() {
        let storage = InMemoryStorage::new();
        let agent_id = Uuid::new_v4();
        let entry_id = Uuid::new_v4();

        // Commit a batch with checkpoint
        let batch = CommandBatch {
            segment: 1,
            sequence: 0,
            commands: vec![AgentCommand::AppendEntry {
                entry_id,
                parent_id: None,
                role: "user".to_string(),
                content: json!({"text": "Hello"}),
            }],
            checkpoint: Some(CheckpointData {
                state: json!({"turn": 1}),
                leaf_entry_id: Some(entry_id),
                token_usage: None,
            }),
        };

        storage.commit_batch(agent_id, batch).await.unwrap();

        // Verify latest segment is updated
        let latest = storage.get_latest_segment(agent_id).await.unwrap();
        assert_eq!(latest, 1);

        // Load the segment and verify checkpoint
        let state = storage.load_segment(agent_id, 1).await.unwrap();
        assert_eq!(state.segment, 1);
        assert!(state.checkpoint.is_some());

        let checkpoint = state.checkpoint.unwrap();
        assert_eq!(checkpoint.state, json!({"turn": 1}));
        assert_eq!(checkpoint.leaf_entry_id, Some(entry_id));
    }

    #[tokio::test]
    async fn test_signal_fifo() {
        let storage = InMemoryStorage::new();
        let agent_id = Uuid::new_v4();

        // Send multiple signals in order
        storage.send_signal(agent_id, "userInput", json!({"index": 1}));
        storage.send_signal(agent_id, "userInput", json!({"index": 2}));
        storage.send_signal(agent_id, "userInput", json!({"index": 3}));

        // Verify has_signal
        assert!(storage.has_signal(agent_id, "userInput").await.unwrap());
        assert!(!storage.has_signal(agent_id, "otherSignal").await.unwrap());

        // Pop signals and verify FIFO order
        let first = storage.pop_signal(agent_id, "userInput").await.unwrap();
        assert_eq!(first, Some(json!({"index": 1})));

        let second = storage.pop_signal(agent_id, "userInput").await.unwrap();
        assert_eq!(second, Some(json!({"index": 2})));

        let third = storage.pop_signal(agent_id, "userInput").await.unwrap();
        assert_eq!(third, Some(json!({"index": 3})));

        // Queue should be empty now
        let empty = storage.pop_signal(agent_id, "userInput").await.unwrap();
        assert_eq!(empty, None);

        assert!(!storage.has_signal(agent_id, "userInput").await.unwrap());
    }

    #[tokio::test]
    async fn test_task_results() {
        let storage = InMemoryStorage::new();
        let task_id_1 = Uuid::new_v4();
        let task_id_2 = Uuid::new_v4();
        let task_id_3 = Uuid::new_v4();

        // Set task results
        storage.set_task_result(
            task_id_1,
            TaskResult {
                task_id: task_id_1,
                status: TaskStatus::Completed,
                output: Some(json!({"answer": 42})),
                error: None,
            },
        );

        storage.set_task_result(
            task_id_2,
            TaskResult {
                task_id: task_id_2,
                status: TaskStatus::Failed,
                output: None,
                error: Some("Task failed".to_string()),
            },
        );

        // Get single task result
        let result_1 = storage.get_task_result(task_id_1).await.unwrap();
        assert!(result_1.is_some());
        let r1 = result_1.unwrap();
        assert_eq!(r1.status, TaskStatus::Completed);
        assert_eq!(r1.output, Some(json!({"answer": 42})));

        // Get non-existent task result
        let result_3 = storage.get_task_result(task_id_3).await.unwrap();
        assert!(result_3.is_none());

        // Get multiple task results
        let results = storage
            .get_task_results(&[task_id_1, task_id_2, task_id_3])
            .await
            .unwrap();
        assert_eq!(results.len(), 2); // Only 2 have results
        assert!(results.iter().any(|r| r.task_id == task_id_1));
        assert!(results.iter().any(|r| r.task_id == task_id_2));
    }

    #[tokio::test]
    async fn test_store_signal_via_trait() {
        let storage = InMemoryStorage::new();
        let agent_id = Uuid::new_v4();

        // Use the trait method instead of the test helper
        storage
            .store_signal(agent_id, "notification", json!({"type": "alert"}))
            .await
            .unwrap();

        // Verify signal was stored
        assert!(storage.has_signal(agent_id, "notification").await.unwrap());

        let payload = storage.pop_signal(agent_id, "notification").await.unwrap();
        assert_eq!(payload, Some(json!({"type": "alert"})));
    }

    #[tokio::test]
    async fn test_multiple_segments() {
        let storage = InMemoryStorage::new();
        let agent_id = Uuid::new_v4();

        // Commit batches for segments 1, 2, 3
        for segment in 1..=3 {
            let batch = CommandBatch {
                segment,
                sequence: 0,
                commands: vec![],
                checkpoint: Some(CheckpointData {
                    state: json!({"segment": segment}),
                    leaf_entry_id: None,
                    token_usage: None,
                }),
            };
            storage.commit_batch(agent_id, batch).await.unwrap();
        }

        // Latest segment should be 3
        let latest = storage.get_latest_segment(agent_id).await.unwrap();
        assert_eq!(latest, 3);

        // Each segment should have its own checkpoint
        for segment in 1..=3 {
            let state = storage.load_segment(agent_id, segment).await.unwrap();
            let checkpoint = state.checkpoint.unwrap();
            assert_eq!(checkpoint.state, json!({"segment": segment}));
        }
    }

    #[tokio::test]
    async fn test_pending_tasks_in_segment() {
        let storage = InMemoryStorage::new();
        let agent_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        // Commit a batch with a scheduled task
        let batch = CommandBatch {
            segment: 1,
            sequence: 0,
            commands: vec![AgentCommand::ScheduleTask {
                task_id,
                kind: "analyze".to_string(),
                input: json!({"data": [1, 2, 3]}),
                options: Default::default(),
            }],
            checkpoint: None,
        };
        storage.commit_batch(agent_id, batch).await.unwrap();

        // Load segment - task should be pending
        let state = storage.load_segment(agent_id, 1).await.unwrap();
        assert_eq!(state.pending_tasks.len(), 1);
        assert_eq!(state.pending_tasks[0].task_id, task_id);
        assert_eq!(state.pending_tasks[0].status, TaskStatus::Pending);

        // Set task result
        storage.set_task_result(
            task_id,
            TaskResult {
                task_id,
                status: TaskStatus::Completed,
                output: Some(json!({"result": "done"})),
                error: None,
            },
        );

        // Load segment again - task should now be completed
        let state = storage.load_segment(agent_id, 1).await.unwrap();
        assert_eq!(state.pending_tasks.len(), 1);
        assert_eq!(state.pending_tasks[0].status, TaskStatus::Completed);
        assert!(state.pending_tasks[0].result.is_some());
    }

    #[tokio::test]
    async fn test_pending_signals_in_segment() {
        let storage = InMemoryStorage::new();
        let agent_id = Uuid::new_v4();

        // Commit a batch waiting for signals
        let batch = CommandBatch {
            segment: 1,
            sequence: 0,
            commands: vec![
                AgentCommand::WaitForSignal {
                    signal_name: "userInput".to_string(),
                },
                AgentCommand::WaitForSignal {
                    signal_name: "approval".to_string(),
                },
            ],
            checkpoint: None,
        };
        storage.commit_batch(agent_id, batch).await.unwrap();

        // Load segment - should have pending signals
        let state = storage.load_segment(agent_id, 1).await.unwrap();
        assert_eq!(state.pending_signals.len(), 2);
        assert!(state.pending_signals.contains(&"userInput".to_string()));
        assert!(state.pending_signals.contains(&"approval".to_string()));
    }

    #[test]
    fn test_clear() {
        let storage = InMemoryStorage::new();
        let agent_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        // Add some data
        storage.send_signal(agent_id, "test", json!(null));
        storage.set_task_result(
            task_id,
            TaskResult {
                task_id,
                status: TaskStatus::Completed,
                output: None,
                error: None,
            },
        );

        // Clear all data
        storage.clear();

        // Verify data is cleared (need runtime for async)
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            assert!(!storage.has_signal(agent_id, "test").await.unwrap());
            assert!(storage.get_task_result(task_id).await.unwrap().is_none());
        });
    }

    #[test]
    fn test_default() {
        let storage = InMemoryStorage::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Should work with empty storage
            let latest = storage.get_latest_segment(Uuid::new_v4()).await.unwrap();
            assert_eq!(latest, 0);
        });
    }
}
