//! Remote storage implementation using gRPC client.
//!
//! This module implements `AgentStorage` using the existing `AgentDispatch` gRPC client.
//! Commands are executed individually for now; a future batch RPC will be added to
//! optimize network efficiency.

use async_trait::async_trait;
use flovyn_worker_core::client::{AgentDispatch, AgentTokenUsage};
use serde_json::Value;
use uuid::Uuid;

use super::{
    AgentCommand, AgentStorage, CheckpointData, CommandBatch, SegmentState, StorageResult,
    TaskResult, TaskStatus, TokenUsage,
};
use crate::error::FlovynError;

/// Remote storage backend using gRPC.
///
/// Wraps the `AgentDispatch` gRPC client to implement `AgentStorage`.
/// This is the production storage backend that communicates with the
/// Flovyn server.
///
/// # Current Implementation
///
/// Commands in a batch are executed individually using separate gRPC calls.
/// This is a transitional implementation; a future batch RPC will be added
/// to commit all commands atomically in a single round-trip.
///
/// # Example
///
/// ```rust,ignore
/// use flovyn_worker_sdk::agent::storage::RemoteStorage;
///
/// let storage = RemoteStorage::new(agent_dispatch_client, org_id);
/// storage.commit_batch(agent_id, batch).await?;
/// ```
pub struct RemoteStorage {
    /// The gRPC client for agent dispatch operations
    client: AgentDispatch,
    /// Organization ID for the agent
    ///
    /// Currently unused but will be needed for batch RPCs in Phase 2.
    #[allow(dead_code)]
    org_id: Uuid,
}

impl RemoteStorage {
    /// Create a new RemoteStorage instance.
    ///
    /// # Arguments
    ///
    /// * `client` - The AgentDispatch gRPC client
    /// * `org_id` - Organization ID for scoping operations
    pub fn new(client: AgentDispatch, org_id: Uuid) -> Self {
        Self { client, org_id }
    }

    /// Convert storage TokenUsage to gRPC TokenUsage
    fn convert_token_usage(usage: &TokenUsage) -> AgentTokenUsage {
        AgentTokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    }

    /// Convert gRPC task status string to TaskStatus enum
    fn parse_task_status(status: &str) -> TaskStatus {
        match status {
            "PENDING" => TaskStatus::Pending,
            "RUNNING" => TaskStatus::Running,
            "COMPLETED" => TaskStatus::Completed,
            "FAILED" => TaskStatus::Failed,
            "CANCELLED" => TaskStatus::Cancelled,
            _ => TaskStatus::Pending,
        }
    }
}

#[async_trait]
impl AgentStorage for RemoteStorage {
    /// Commit a batch of commands by executing them individually.
    ///
    /// Currently executes commands one at a time via gRPC. This will be
    /// replaced with a batch RPC in Phase 2.
    async fn commit_batch(&self, agent_id: Uuid, batch: CommandBatch) -> StorageResult<()> {
        let mut client = self.client.clone();

        // Execute each command individually
        for command in &batch.commands {
            match command {
                AgentCommand::AppendEntry {
                    entry_id,
                    parent_id,
                    role,
                    content,
                } => {
                    // Map role to entry_type for the gRPC call
                    let entry_type = match role.as_str() {
                        "tool_result" => "TOOL_RESULT",
                        "assistant" => "MESSAGE",
                        "user" => "MESSAGE",
                        "system" => "MESSAGE",
                        _ => "MESSAGE",
                    };

                    client
                        .append_entry(
                            agent_id,
                            *parent_id,
                            entry_type,
                            Some(role.as_str()),
                            content,
                            None, // turn_id
                            None, // token_usage
                            Some(&entry_id.to_string()),
                        )
                        .await
                        .map_err(FlovynError::from)?;
                }
                AgentCommand::ScheduleTask {
                    task_id: _,
                    kind,
                    input,
                    options,
                    idempotency_key,
                } => {
                    // Use content-based idempotency_key for deduplication.
                    // Server will use this to detect duplicate task submissions.
                    client
                        .schedule_task(
                            agent_id,
                            kind,
                            input,
                            options.queue.as_deref(),
                            options.max_retries,
                            options.timeout_ms,
                            Some(idempotency_key),
                        )
                        .await
                        .map_err(FlovynError::from)?;
                }
                AgentCommand::WaitForSignal { signal_name: _ } => {
                    // Signal waiting is recorded in checkpoint state, no gRPC call needed
                }
            }
        }

        // Submit checkpoint if provided
        if let Some(checkpoint) = &batch.checkpoint {
            let token_usage = checkpoint
                .token_usage
                .as_ref()
                .map(Self::convert_token_usage);

            client
                .submit_checkpoint(
                    agent_id,
                    checkpoint.leaf_entry_id,
                    &checkpoint.state,
                    token_usage,
                )
                .await
                .map_err(FlovynError::from)?;
        }

        Ok(())
    }

    /// Load segment state for recovery.
    ///
    /// Uses the get_latest_checkpoint gRPC call to retrieve checkpoint data.
    async fn load_segment(&self, agent_id: Uuid, segment: u64) -> StorageResult<SegmentState> {
        let mut client = self.client.clone();

        // Get the latest checkpoint
        let checkpoint = client
            .get_latest_checkpoint(agent_id)
            .await
            .map_err(FlovynError::from)?;

        // Convert checkpoint to CheckpointData
        let checkpoint_data = checkpoint.map(|cp| CheckpointData {
            state: cp.state,
            leaf_entry_id: cp.leaf_entry_id,
            token_usage: None, // Checkpoints don't store cumulative token usage for recovery
        });

        // TODO: In Phase 2, we'll track pending tasks and signals in the checkpoint
        // For now, return empty lists since the current implementation doesn't persist this
        Ok(SegmentState {
            segment,
            checkpoint: checkpoint_data,
            pending_tasks: Vec::new(),
            pending_signals: Vec::new(),
        })
    }

    /// Get the latest segment number for an agent.
    ///
    /// Uses checkpoint sequence number as segment proxy.
    async fn get_latest_segment(&self, agent_id: Uuid) -> StorageResult<u64> {
        let mut client = self.client.clone();

        // Get the latest checkpoint to determine segment
        let checkpoint = client
            .get_latest_checkpoint(agent_id)
            .await
            .map_err(FlovynError::from)?;

        // Use checkpoint sequence as segment number
        Ok(checkpoint.map(|cp| cp.sequence as u64).unwrap_or(0))
    }

    /// Get the result of a completed task.
    async fn get_task_result(&self, task_id: Uuid) -> StorageResult<Option<TaskResult>> {
        // Note: get_task_result requires agent_id, but AgentStorage trait doesn't have it
        // For now, we'll use a workaround - the caller should use get_task_results instead
        // which is called from AgentContextImpl where agent_id is available
        //
        // This method is kept for trait compliance but may need refactoring
        // when we have a proper task lookup by task_id only.
        tracing::warn!(
            task_id = %task_id,
            "get_task_result called without agent_id context - returning None"
        );
        Ok(None)
    }

    /// Get results for multiple tasks in one call.
    async fn get_task_results(&self, task_ids: &[Uuid]) -> StorageResult<Vec<TaskResult>> {
        // Similar to get_task_result, this needs agent_id context
        // The actual implementation should be called from AgentContextImpl
        // which has access to agent_id
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }

        tracing::warn!(
            task_count = task_ids.len(),
            "get_task_results called without agent_id context - returning empty"
        );
        Ok(Vec::new())
    }

    /// Store a signal for an agent.
    async fn store_signal(
        &self,
        agent_id: Uuid,
        signal_name: &str,
        payload: Value,
    ) -> StorageResult<()> {
        let mut client = self.client.clone();

        client
            .signal_agent(agent_id, signal_name, &payload)
            .await
            .map_err(FlovynError::from)?;

        Ok(())
    }

    /// Get and remove the next pending signal.
    async fn pop_signal(&self, agent_id: Uuid, signal_name: &str) -> StorageResult<Option<Value>> {
        let mut client = self.client.clone();

        // consume_signals removes and returns signals
        let signals = client
            .consume_signals(agent_id, Some(signal_name))
            .await
            .map_err(FlovynError::from)?;

        // Return the first signal's value if any
        Ok(signals.into_iter().next().map(|s| s.signal_value))
    }

    /// Check if a signal is pending without consuming it.
    async fn has_signal(&self, agent_id: Uuid, signal_name: &str) -> StorageResult<bool> {
        let mut client = self.client.clone();

        client
            .has_signal(agent_id, signal_name)
            .await
            .map_err(FlovynError::from)
    }
}

/// Extension trait for RemoteStorage to access task results with agent context.
///
/// This trait provides methods that require agent_id context, which is not
/// available in the base AgentStorage trait.
impl RemoteStorage {
    /// Get task result with agent context.
    ///
    /// Unlike the trait method, this takes agent_id as a parameter.
    pub async fn get_task_result_for_agent(
        &self,
        agent_id: Uuid,
        task_id: Uuid,
    ) -> StorageResult<Option<TaskResult>> {
        let mut client = self.client.clone();

        let result = client
            .get_task_result(agent_id, task_id)
            .await
            .map_err(FlovynError::from)?;

        // Convert to our TaskResult type
        let status = Self::parse_task_status(&result.status);

        // Only return result for terminal states
        if result.is_terminal() {
            Ok(Some(TaskResult {
                task_id,
                status,
                output: result.output,
                error: result.error,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get multiple task results with agent context.
    pub async fn get_task_results_for_agent(
        &self,
        agent_id: Uuid,
        task_ids: &[Uuid],
    ) -> StorageResult<Vec<TaskResult>> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut client = self.client.clone();

        let results = client
            .get_task_results_batch(agent_id, task_ids)
            .await
            .map_err(FlovynError::from)?;

        // Convert to our TaskResult type, filtering for terminal states
        let task_results = results
            .into_iter()
            .filter(|r| r.is_terminal())
            .map(|r| TaskResult {
                task_id: r.task_execution_id,
                status: Self::parse_task_status(&r.status),
                output: r.output,
                error: r.error,
            })
            .collect();

        Ok(task_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_task_status() {
        assert_eq!(
            RemoteStorage::parse_task_status("PENDING"),
            TaskStatus::Pending
        );
        assert_eq!(
            RemoteStorage::parse_task_status("RUNNING"),
            TaskStatus::Running
        );
        assert_eq!(
            RemoteStorage::parse_task_status("COMPLETED"),
            TaskStatus::Completed
        );
        assert_eq!(
            RemoteStorage::parse_task_status("FAILED"),
            TaskStatus::Failed
        );
        assert_eq!(
            RemoteStorage::parse_task_status("CANCELLED"),
            TaskStatus::Cancelled
        );
        assert_eq!(
            RemoteStorage::parse_task_status("UNKNOWN"),
            TaskStatus::Pending
        );
    }

    #[test]
    fn test_convert_token_usage() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
        };
        let converted = RemoteStorage::convert_token_usage(&usage);
        assert_eq!(converted.input_tokens, 100);
        assert_eq!(converted.output_tokens, 50);
        assert!(converted.cache_read_tokens.is_none());
        assert!(converted.cache_write_tokens.is_none());
    }
}
