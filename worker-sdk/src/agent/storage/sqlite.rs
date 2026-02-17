//! SQLite storage implementation for local agent execution.
//!
//! This module implements `AgentStorage` using a local SQLite database.
//! All operations use SQLite transactions for atomic batch commits.
//!
//! # Schema
//!
//! Four tables: `checkpoints`, `entries`, `tasks`, `signals`.
//! Schema is managed via sqlx migrations in `migrations/`.
//!
//! # Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::storage::SqliteStorage;
//!
//! let storage = SqliteStorage::open("./agent.db").await?;
//! storage.commit_batch(agent_id, batch).await?;
//! ```

use std::path::Path;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::{
    AgentCommand, AgentStorage, CheckpointData, CommandBatch, SegmentState, StorageResult,
    TaskResult, TaskStatus, TokenUsage,
};
use crate::error::FlovynError;

/// SQLite-backed storage for local agent execution.
///
/// Uses a single SQLite file to store all agent state. Batch commits
/// are atomic via SQLite transactions.
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Open or create a SQLite database at the given path.
    ///
    /// Runs migrations automatically to ensure the schema is up to date.
    pub async fn open(path: impl AsRef<Path>) -> StorageResult<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| FlovynError::Other(format!("SQLite connection failed: {e}")))?;

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| FlovynError::Other(format!("SQLite migration failed: {e}")))?;

        Ok(Self { pool })
    }

    /// Open an in-memory SQLite database (for testing).
    pub async fn in_memory() -> StorageResult<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .map_err(|e| FlovynError::Other(format!("SQLite connection failed: {e}")))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| FlovynError::Other(format!("SQLite migration failed: {e}")))?;

        Ok(Self { pool })
    }

    /// Parse a task status string from the database.
    fn parse_status(s: &str) -> TaskStatus {
        match s {
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "cancelled" => TaskStatus::Cancelled,
            "running" => TaskStatus::Running,
            _ => TaskStatus::Pending,
        }
    }

    /// Convert a TaskStatus to its database string representation.
    fn status_str(status: TaskStatus) -> &'static str {
        match status {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
}

#[async_trait]
impl AgentStorage for SqliteStorage {
    async fn commit_batch(&self, agent_id: Uuid, batch: CommandBatch) -> StorageResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| FlovynError::Other(format!("SQLite begin failed: {e}")))?;

        let agent_id_str = agent_id.to_string();
        let now = Utc::now().to_rfc3339();

        for cmd in &batch.commands {
            match cmd {
                AgentCommand::AppendEntry {
                    entry_id,
                    parent_id,
                    role,
                    content,
                } => {
                    let entry_id_str = entry_id.to_string();
                    let parent_id_str = parent_id.map(|id| id.to_string());
                    let content_str = serde_json::to_string(content)
                        .map_err(|e| FlovynError::Other(format!("JSON serialize failed: {e}")))?;

                    sqlx::query(
                        "INSERT INTO entries (entry_id, agent_id, parent_id, segment, role, content, created_at)
                         VALUES (?, ?, ?, ?, ?, ?, ?)"
                    )
                    .bind(&entry_id_str)
                    .bind(&agent_id_str)
                    .bind(&parent_id_str)
                    .bind(batch.segment as i64)
                    .bind(role)
                    .bind(&content_str)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| FlovynError::Other(format!("SQLite insert entry failed: {e}")))?;
                }
                AgentCommand::ScheduleTask {
                    task_id,
                    kind,
                    input,
                    options: _,
                    idempotency_key: _,
                } => {
                    let task_id_str = task_id.to_string();
                    let input_str = serde_json::to_string(input)
                        .map_err(|e| FlovynError::Other(format!("JSON serialize failed: {e}")))?;

                    sqlx::query(
                        "INSERT OR IGNORE INTO tasks (task_id, agent_id, kind, input, status, created_at)
                         VALUES (?, ?, ?, ?, 'pending', ?)"
                    )
                    .bind(&task_id_str)
                    .bind(&agent_id_str)
                    .bind(kind)
                    .bind(&input_str)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| FlovynError::Other(format!("SQLite insert task failed: {e}")))?;
                }
                AgentCommand::WaitForSignal { .. } => {
                    // No-op for storage; signal delivery handled by SignalSource
                }
            }
        }

        if let Some(checkpoint) = &batch.checkpoint {
            let state_str = serde_json::to_string(&checkpoint.state)
                .map_err(|e| FlovynError::Other(format!("JSON serialize failed: {e}")))?;
            let leaf_entry_str = checkpoint.leaf_entry_id.map(|id| id.to_string());
            let token_usage_str = checkpoint
                .token_usage
                .as_ref()
                .map(|u| serde_json::to_string(u).unwrap_or_default());

            sqlx::query(
                "INSERT OR REPLACE INTO checkpoints (agent_id, segment, state, leaf_entry_id, token_usage, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(&agent_id_str)
            .bind(batch.segment as i64)
            .bind(&state_str)
            .bind(&leaf_entry_str)
            .bind(&token_usage_str)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| FlovynError::Other(format!("SQLite insert checkpoint failed: {e}")))?;
        }

        tx.commit()
            .await
            .map_err(|e| FlovynError::Other(format!("SQLite commit failed: {e}")))?;

        Ok(())
    }

    async fn load_segment(&self, agent_id: Uuid, segment: u64) -> StorageResult<SegmentState> {
        let agent_id_str = agent_id.to_string();

        // Load checkpoint for this segment
        let checkpoint = sqlx::query(
            "SELECT state, leaf_entry_id, token_usage FROM checkpoints
             WHERE agent_id = ? AND segment = ?"
        )
        .bind(&agent_id_str)
        .bind(segment as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| FlovynError::Other(format!("SQLite query failed: {e}")))?;

        let checkpoint_data = checkpoint
            .map(|row| {
                let state_str: String = row.get("state");
                let leaf_entry_str: Option<String> = row.get("leaf_entry_id");
                let token_usage_str: Option<String> = row.get("token_usage");

                CheckpointData {
                    state: serde_json::from_str(&state_str).unwrap_or(Value::Null),
                    leaf_entry_id: leaf_entry_str.and_then(|s| Uuid::parse_str(&s).ok()),
                    token_usage: token_usage_str
                        .and_then(|s| serde_json::from_str::<TokenUsage>(&s).ok()),
                }
            });

        // Load pending tasks
        let pending_tasks = sqlx::query(
            "SELECT task_id, kind, status, output, error FROM tasks
             WHERE agent_id = ? AND status IN ('pending', 'running')"
        )
        .bind(&agent_id_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| FlovynError::Other(format!("SQLite query failed: {e}")))?;

        let pending_tasks = pending_tasks
            .into_iter()
            .map(|row| {
                let task_id_str: String = row.get("task_id");
                let kind: String = row.get("kind");
                let status_str: String = row.get("status");
                super::PendingTask {
                    task_id: Uuid::parse_str(&task_id_str).unwrap_or_default(),
                    kind,
                    status: Self::parse_status(&status_str),
                    result: None,
                }
            })
            .collect();

        Ok(SegmentState {
            segment,
            checkpoint: checkpoint_data,
            pending_tasks,
            pending_signals: Vec::new(),
        })
    }

    async fn get_latest_segment(&self, agent_id: Uuid) -> StorageResult<u64> {
        let agent_id_str = agent_id.to_string();

        let result = sqlx::query(
            "SELECT MAX(segment) as max_seg FROM checkpoints WHERE agent_id = ?"
        )
        .bind(&agent_id_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| FlovynError::Other(format!("SQLite query failed: {e}")))?;

        let max_seg: Option<i64> = result.get("max_seg");
        Ok(max_seg.unwrap_or(0) as u64)
    }

    async fn get_task_result(
        &self,
        _agent_id: Uuid,
        task_id: Uuid,
    ) -> StorageResult<Option<TaskResult>> {
        let task_id_str = task_id.to_string();

        let row = sqlx::query(
            "SELECT task_id, status, output, error FROM tasks WHERE task_id = ?"
        )
        .bind(&task_id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| FlovynError::Other(format!("SQLite query failed: {e}")))?;

        Ok(row.map(|r| {
            let status_str: String = r.get("status");
            let output_str: Option<String> = r.get("output");
            let error: Option<String> = r.get("error");

            TaskResult {
                task_id,
                status: Self::parse_status(&status_str),
                output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
                error,
            }
        }))
    }

    async fn get_task_results(
        &self,
        _agent_id: Uuid,
        task_ids: &[Uuid],
    ) -> StorageResult<Vec<TaskResult>> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::with_capacity(task_ids.len());
        for &task_id in task_ids {
            if let Some(result) = self.get_task_result(_agent_id, task_id).await? {
                results.push(result);
            }
        }
        Ok(results)
    }

    async fn store_signal(
        &self,
        agent_id: Uuid,
        signal_name: &str,
        payload: Value,
    ) -> StorageResult<()> {
        let agent_id_str = agent_id.to_string();
        let payload_str = serde_json::to_string(&payload)
            .map_err(|e| FlovynError::Other(format!("JSON serialize failed: {e}")))?;
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO signals (agent_id, signal_name, payload, created_at)
             VALUES (?, ?, ?, ?)"
        )
        .bind(&agent_id_str)
        .bind(signal_name)
        .bind(&payload_str)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| FlovynError::Other(format!("SQLite insert signal failed: {e}")))?;

        Ok(())
    }

    async fn pop_signal(
        &self,
        agent_id: Uuid,
        signal_name: &str,
    ) -> StorageResult<Option<Value>> {
        let agent_id_str = agent_id.to_string();

        // Get the oldest signal with this name
        let row = sqlx::query(
            "SELECT id, payload FROM signals
             WHERE agent_id = ? AND signal_name = ?
             ORDER BY id ASC
             LIMIT 1"
        )
        .bind(&agent_id_str)
        .bind(signal_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| FlovynError::Other(format!("SQLite query failed: {e}")))?;

        match row {
            Some(r) => {
                let id: i64 = r.get("id");
                let payload_str: String = r.get("payload");

                // Delete the consumed signal
                sqlx::query("DELETE FROM signals WHERE id = ?")
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| FlovynError::Other(format!("SQLite delete signal failed: {e}")))?;

                let payload = serde_json::from_str(&payload_str).unwrap_or(Value::Null);
                Ok(Some(payload))
            }
            None => Ok(None),
        }
    }

    async fn has_signal(&self, agent_id: Uuid, signal_name: &str) -> StorageResult<bool> {
        let agent_id_str = agent_id.to_string();

        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM signals
             WHERE agent_id = ? AND signal_name = ?"
        )
        .bind(&agent_id_str)
        .bind(signal_name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| FlovynError::Other(format!("SQLite query failed: {e}")))?;

        let count: i64 = row.get("cnt");
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_sqlite_storage_creates_schema() {
        let storage = SqliteStorage::in_memory().await.unwrap();

        // Verify tables exist by running queries against them
        let result = sqlx::query("SELECT COUNT(*) as cnt FROM checkpoints")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        let count: i64 = result.get("cnt");
        assert_eq!(count, 0);

        let result = sqlx::query("SELECT COUNT(*) as cnt FROM entries")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        let count: i64 = result.get("cnt");
        assert_eq!(count, 0);

        let result = sqlx::query("SELECT COUNT(*) as cnt FROM tasks")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        let count: i64 = result.get("cnt");
        assert_eq!(count, 0);

        let result = sqlx::query("SELECT COUNT(*) as cnt FROM signals")
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        let count: i64 = result.get("cnt");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_sqlite_storage_migrations_idempotent() {
        // Create temp file for the database
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Open twice - should not error
        let _storage1 = SqliteStorage::open(&db_path).await.unwrap();
        let _storage2 = SqliteStorage::open(&db_path).await.unwrap();
    }
}
