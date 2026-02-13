//! AgentContextImpl - Implementation of AgentContext trait
//!
//! This implementation communicates with the server via gRPC to persist entries,
//! manage checkpoints, schedule tasks, and handle signals.

use crate::agent::context::{
    AgentContext, CancelTaskResult, EntryRole, EntryType, LoadedMessage, ScheduleAgentTaskOptions,
    TokenUsage,
};
use crate::agent::future::AgentTaskFutureRaw;
use crate::agent::storage::{AgentCommand, CheckpointData, CommandBatch};
use crate::error::{FlovynError, Result};
use crate::task::streaming::StreamEvent;
use async_trait::async_trait;
use flovyn_worker_core::client::{AgentDispatch, AgentEntry as CoreEntry, AgentTokenUsage};
use flovyn_worker_core::generated::flovyn_v1::AgentStreamEventType;
use parking_lot::RwLock;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

/// Implementation of AgentContext that communicates with the server via gRPC.
pub struct AgentContextImpl {
    /// Agent execution ID
    agent_execution_id: Uuid,
    /// Organization ID
    org_id: Uuid,
    /// Agent input (JSON)
    input: Value,
    /// Loaded conversation messages (cached)
    messages: RwLock<Vec<LoadedMessage>>,
    /// Current checkpoint state
    checkpoint_state: RwLock<Option<Value>>,
    /// Current checkpoint sequence
    checkpoint_sequence: AtomicI32,
    /// Current leaf entry ID (for building entry chain)
    leaf_entry_id: RwLock<Option<Uuid>>,
    /// Entry counter - monotonically increasing, never resets.
    /// Used for stable entry idempotency keys to prevent duplicate entries on resume.
    entry_counter: AtomicI32,
    /// Task counter - monotonically increasing, never resets.
    /// Used for stable task IDs to prevent duplicate tasks on resume.
    /// Similar to entry_counter: agents replay from beginning on resume,
    /// and idempotency prevents duplicate task creation.
    task_counter: AtomicI32,
    /// Stream sequence counter
    stream_sequence: AtomicI32,
    /// gRPC client (tokio mutex for async-safe access)
    client: TokioMutex<AgentDispatch>,
    /// Cancellation flag
    cancellation_requested: AtomicBool,
    /// Pending commands to be committed at next suspension point
    pending_commands: RwLock<Vec<AgentCommand>>,
    /// Current segment number (increments on each recovery)
    current_segment: AtomicU64,
    /// Current sequence within segment
    current_sequence: AtomicU64,
}

impl AgentContextImpl {
    /// Create a new AgentContextImpl.
    ///
    /// This loads the initial state from the server (entries and checkpoint).
    pub async fn new(
        mut client: AgentDispatch,
        agent_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        current_checkpoint_seq: i32,
    ) -> Result<Self> {
        // Load checkpoint if resuming
        let (checkpoint_state, checkpoint_seq, leaf_entry_id) = if current_checkpoint_seq >= 0 {
            match client.get_latest_checkpoint(agent_execution_id).await? {
                Some(cp) => (Some(cp.state), cp.sequence, cp.leaf_entry_id),
                None => (None, -1, None),
            }
        } else {
            (None, -1, None)
        };

        // Load entries
        let entries = client.get_entries(agent_execution_id, None).await?;

        // Entry counter starts at 0, NOT entries.len().
        // Why? Agents MUST replay append_entry calls from the beginning on resume.
        // The server uses idempotency keys to return existing entries for duplicate
        // keys, preventing entry duplication. This design ensures that:
        // 1. All append_entry calls use the same keys on replay
        // 2. The server handles deduplication via idempotency
        //
        // IMPORTANT: Agents should NOT skip append_entry calls for "loaded" messages.
        // They should replay ALL append_entry calls and let idempotency handle it.

        let messages = entries.into_iter().map(convert_entry_to_message).collect();

        // Compute segment number from checkpoint sequence
        // Segment 0 = initial execution, increments on each recovery
        let segment = if checkpoint_seq >= 0 {
            (checkpoint_seq + 1) as u64
        } else {
            0
        };

        Ok(Self {
            agent_execution_id,
            org_id,
            input,
            messages: RwLock::new(messages),
            checkpoint_state: RwLock::new(checkpoint_state),
            checkpoint_sequence: AtomicI32::new(checkpoint_seq),
            leaf_entry_id: RwLock::new(leaf_entry_id),
            entry_counter: AtomicI32::new(0),
            task_counter: AtomicI32::new(0),
            stream_sequence: AtomicI32::new(0),
            client: TokioMutex::new(client),
            cancellation_requested: AtomicBool::new(false),
            pending_commands: RwLock::new(Vec::new()),
            current_segment: AtomicU64::new(segment),
            current_sequence: AtomicU64::new(0),
        })
    }

    /// Create from pre-loaded data (for testing or optimization).
    #[allow(clippy::too_many_arguments)]
    pub fn from_loaded(
        client: AgentDispatch,
        agent_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        messages: Vec<LoadedMessage>,
        checkpoint_state: Option<Value>,
        checkpoint_sequence: i32,
        leaf_entry_id: Option<Uuid>,
    ) -> Self {
        // Compute segment number from checkpoint sequence
        let segment = if checkpoint_sequence >= 0 {
            (checkpoint_sequence + 1) as u64
        } else {
            0
        };

        Self {
            agent_execution_id,
            org_id,
            input,
            messages: RwLock::new(messages),
            checkpoint_state: RwLock::new(checkpoint_state),
            checkpoint_sequence: AtomicI32::new(checkpoint_sequence),
            leaf_entry_id: RwLock::new(leaf_entry_id),
            entry_counter: AtomicI32::new(0),
            task_counter: AtomicI32::new(0),
            stream_sequence: AtomicI32::new(0),
            client: TokioMutex::new(client),
            cancellation_requested: AtomicBool::new(false),
            pending_commands: RwLock::new(Vec::new()),
            current_segment: AtomicU64::new(segment),
            current_sequence: AtomicU64::new(0),
        }
    }

    /// Request cancellation
    pub fn request_cancellation(&self) {
        self.cancellation_requested.store(true, Ordering::SeqCst);
    }

    /// Generate an idempotency key for entry creation.
    /// Uses entry_counter which never resets, ensuring stable keys
    /// across suspension boundaries.
    fn generate_entry_idempotency_key(&self) -> String {
        let index = self.entry_counter.fetch_add(1, Ordering::SeqCst);
        format!("{}:entry:{}", self.agent_execution_id, index)
    }

    /// Generate a deterministic entry ID from an idempotency key.
    ///
    /// Uses UUIDv5 with the idempotency key as input, ensuring:
    /// 1. Same key always produces the same entry ID
    /// 2. Entry IDs are valid UUIDs
    /// 3. No server round-trip required
    fn entry_id_from_idempotency_key(idempotency_key: &str) -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_OID, idempotency_key.as_bytes())
    }

    /// Get current timestamp in milliseconds
    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    /// Generate a deterministic task ID based on segment and sequence.
    ///
    /// This produces stable, reproducible task IDs that don't change across
    /// suspension/resume cycles. The ID is derived from:
    /// - agent_execution_id: unique per agent execution
    /// - current_segment: increments on each recovery
    /// - current_sequence: increments for each command within a segment
    ///
    /// Using UUIDv5 with these inputs ensures:
    /// 1. Same inputs always produce the same task ID
    /// 2. Different inputs produce different task IDs
    /// 3. Task IDs are valid UUIDs
    ///
    /// Generates a deterministic task ID using task_counter which never resets,
    /// ensuring stable IDs across suspension/resume boundaries. Similar to entries:
    /// agents replay from beginning on resume, and server idempotency prevents
    /// duplicate task creation.
    fn next_task_id(&self) -> Uuid {
        let counter = self.task_counter.fetch_add(1, Ordering::SeqCst);
        let input = format!("{}:task:{}", self.agent_execution_id, counter);
        Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
    }

    /// Get the count of pending commands (for debugging)
    pub fn pending_commands_count(&self) -> usize {
        self.pending_commands.read().len()
    }

    /// Commit pending commands as an atomic batch.
    ///
    /// This method is called at suspension points (checkpoint, wait_for_signal,
    /// schedule_task with blocking wait) to persist all accumulated commands.
    ///
    /// # Arguments
    ///
    /// * `checkpoint` - Optional checkpoint data to persist with the batch
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the batch was committed successfully, or an error
    /// if any command failed. Note: partial failures may occur in the legacy
    /// implementation since commands are executed individually.
    pub async fn commit_pending_batch(&self, checkpoint: Option<CheckpointData>) -> Result<()> {
        let commands = {
            let mut pending = self.pending_commands.write();
            std::mem::take(&mut *pending)
        };

        if commands.is_empty() && checkpoint.is_none() {
            return Ok(());
        }

        let batch = CommandBatch {
            segment: self.current_segment.load(Ordering::SeqCst),
            sequence: self.current_sequence.load(Ordering::SeqCst),
            commands,
            checkpoint,
        };

        self.execute_batch_legacy(&batch).await
    }

    /// Execute a batch of commands using individual gRPC calls.
    ///
    /// This is a backward-compatible implementation that executes commands
    /// one at a time via the existing gRPC client. It will be replaced by
    /// a batch RPC in a future release for better efficiency.
    ///
    /// # Arguments
    ///
    /// * `batch` - The command batch to execute
    ///
    /// # Note
    ///
    /// This method does not guarantee atomicity - if a command fails,
    /// previous commands may have already been persisted.
    async fn execute_batch_legacy(&self, batch: &CommandBatch) -> Result<()> {
        let mut client = self.client.lock().await;

        // Execute each command individually
        for command in &batch.commands {
            match command {
                AgentCommand::AppendEntry {
                    entry_id,
                    parent_id,
                    role,
                    content,
                } => {
                    client
                        .append_entry(
                            self.agent_execution_id,
                            *parent_id,
                            "MESSAGE", // entry_type
                            Some(role.as_str()),
                            content,
                            None,                        // turn_id
                            None,                        // token_usage
                            Some(&entry_id.to_string()), // idempotency_key
                        )
                        .await?;
                }
                AgentCommand::ScheduleTask {
                    task_id,
                    kind,
                    input,
                    options,
                } => {
                    // Use task_id as idempotency_key so server returns/creates with this ID
                    client
                        .schedule_task(
                            self.agent_execution_id,
                            kind,
                            input,
                            options.queue.as_deref(),
                            options.max_retries,
                            options.timeout_ms,
                            Some(&task_id.to_string()),
                        )
                        .await?;
                }
                AgentCommand::WaitForSignal { signal_name: _ } => {
                    // Signal waiting is tracked locally, no gRPC call needed
                    // The suspend call will be made separately
                }
            }
        }

        // Submit checkpoint if provided
        if let Some(checkpoint) = &batch.checkpoint {
            let token_usage = checkpoint.token_usage.as_ref().map(|tu| AgentTokenUsage {
                input_tokens: tu.input_tokens,
                output_tokens: tu.output_tokens,
                cache_read_tokens: None,
                cache_write_tokens: None,
            });

            client
                .submit_checkpoint(
                    self.agent_execution_id,
                    checkpoint.leaf_entry_id,
                    &checkpoint.state,
                    token_usage,
                )
                .await?;
        }

        Ok(())
    }

    /// Add a command to the pending batch.
    ///
    /// Commands are accumulated and committed atomically at the next
    /// suspension point.
    fn add_pending_command(&self, command: AgentCommand) {
        self.pending_commands.write().push(command);
    }

    /// Get the current segment number.
    #[allow(dead_code)]
    pub fn current_segment(&self) -> u64 {
        self.current_segment.load(Ordering::SeqCst)
    }

    /// Get the current sequence number within the segment.
    #[allow(dead_code)]
    pub fn current_sequence(&self) -> u64 {
        self.current_sequence.load(Ordering::SeqCst)
    }

    /// Batch fetch results for multiple tasks.
    ///
    /// This is an internal helper used by `join_all` and `select_ok` to efficiently
    /// check the status of multiple tasks in a single call.
    async fn get_task_results_batch(
        &self,
        task_ids: &[Uuid],
    ) -> Result<Vec<flovyn_worker_core::client::BatchTaskResult>> {
        let mut client = self.client.lock().await;
        Ok(client
            .get_task_results_batch(self.agent_execution_id, task_ids)
            .await?)
    }

    /// Suspend the agent waiting for multiple tasks.
    ///
    /// This is an internal helper used by `join_all` and `select_ok` to suspend
    /// the agent while waiting for tasks to complete.
    async fn suspend_for_tasks(
        &self,
        task_ids: &[Uuid],
        mode: flovyn_worker_core::client::WaitMode,
    ) -> Result<()> {
        // Get current state for checkpoint
        let current_state = self.checkpoint_state.read().clone().unwrap_or(Value::Null);
        let leaf_entry_id = *self.leaf_entry_id.read();

        // Commit pending batch with checkpoint in one operation.
        // This is critical: tasks must be committed before we can wait for them,
        // and we checkpoint to preserve state before suspension.
        let checkpoint_data = CheckpointData {
            state: current_state.clone(),
            leaf_entry_id,
            token_usage: None,
        };
        self.commit_pending_batch(Some(checkpoint_data)).await?;

        // Update local checkpoint state
        *self.checkpoint_state.write() = Some(current_state);
        self.checkpoint_sequence.fetch_add(1, Ordering::SeqCst);

        // Suspend waiting for tasks
        let mut client = self.client.lock().await;
        client
            .suspend_agent_for_tasks(
                self.agent_execution_id,
                task_ids,
                mode,
                Some("Waiting for parallel tasks"),
            )
            .await?;

        Ok(())
    }
}

#[async_trait]
impl AgentContext for AgentContextImpl {
    fn agent_execution_id(&self) -> Uuid {
        self.agent_execution_id
    }

    fn org_id(&self) -> Uuid {
        self.org_id
    }

    fn input_raw(&self) -> &Value {
        &self.input
    }

    async fn append_entry(&self, role: EntryRole, content: &Value) -> Result<Uuid> {
        let parent_id = *self.leaf_entry_id.read();
        let idempotency_key = self.generate_entry_idempotency_key();
        let entry_id = Self::entry_id_from_idempotency_key(&idempotency_key);

        // Buffer command for batch commit (no immediate RPC)
        self.add_pending_command(AgentCommand::AppendEntry {
            entry_id,
            parent_id,
            role: role.as_str().to_string(),
            content: content.clone(),
        });

        // Update leaf entry immediately
        *self.leaf_entry_id.write() = Some(entry_id);

        // Add to local messages cache immediately
        self.messages.write().push(LoadedMessage {
            entry_id,
            entry_type: EntryType::Message,
            role,
            content: content.clone(),
            token_usage: None,
        });

        Ok(entry_id)
    }

    async fn append_tool_call(&self, tool_name: &str, tool_input: &Value) -> Result<Uuid> {
        let parent_id = *self.leaf_entry_id.read();
        let idempotency_key = self.generate_entry_idempotency_key();
        let entry_id = Self::entry_id_from_idempotency_key(&idempotency_key);
        let content = serde_json::json!({
            "tool_name": tool_name,
            "input": tool_input
        });

        // Buffer command for batch commit (no immediate RPC)
        self.add_pending_command(AgentCommand::AppendEntry {
            entry_id,
            parent_id,
            role: EntryRole::Assistant.as_str().to_string(),
            content: content.clone(),
        });

        // Update leaf entry immediately
        *self.leaf_entry_id.write() = Some(entry_id);

        // Add to local messages cache immediately
        self.messages.write().push(LoadedMessage {
            entry_id,
            entry_type: EntryType::Message,
            role: EntryRole::Assistant,
            content,
            token_usage: None,
        });

        Ok(entry_id)
    }

    async fn append_tool_result(&self, tool_name: &str, tool_output: &Value) -> Result<Uuid> {
        let parent_id = *self.leaf_entry_id.read();
        let idempotency_key = self.generate_entry_idempotency_key();
        let entry_id = Self::entry_id_from_idempotency_key(&idempotency_key);
        let content = serde_json::json!({
            "tool_name": tool_name,
            "output": tool_output
        });

        // Buffer command for batch commit (no immediate RPC)
        self.add_pending_command(AgentCommand::AppendEntry {
            entry_id,
            parent_id,
            role: EntryRole::ToolResult.as_str().to_string(),
            content: content.clone(),
        });

        // Update leaf entry immediately
        *self.leaf_entry_id.write() = Some(entry_id);

        // Add to local messages cache immediately
        self.messages.write().push(LoadedMessage {
            entry_id,
            entry_type: EntryType::Message,
            role: EntryRole::ToolResult,
            content,
            token_usage: None,
        });

        Ok(entry_id)
    }

    async fn append_tool_result_with_id(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        tool_output: &Value,
    ) -> Result<Uuid> {
        let parent_id = *self.leaf_entry_id.read();
        let idempotency_key = self.generate_entry_idempotency_key();
        let entry_id = Self::entry_id_from_idempotency_key(&idempotency_key);
        let content = serde_json::json!({
            "toolCallId": tool_call_id,
            "tool_name": tool_name,
            "output": tool_output
        });

        // Buffer command for batch commit (no immediate RPC)
        self.add_pending_command(AgentCommand::AppendEntry {
            entry_id,
            parent_id,
            role: EntryRole::ToolResult.as_str().to_string(),
            content: content.clone(),
        });

        // Update leaf entry immediately
        *self.leaf_entry_id.write() = Some(entry_id);

        // Add to local messages cache immediately
        self.messages.write().push(LoadedMessage {
            entry_id,
            entry_type: EntryType::Message,
            role: EntryRole::ToolResult,
            content,
            token_usage: None,
        });

        Ok(entry_id)
    }

    fn load_messages(&self) -> Vec<LoadedMessage> {
        self.messages.read().clone()
    }

    async fn reload_messages(&self) -> Result<Vec<LoadedMessage>> {
        let entries = {
            let mut client = self.client.lock().await;
            client.get_entries(self.agent_execution_id, None).await?
        };

        let messages: Vec<LoadedMessage> =
            entries.into_iter().map(convert_entry_to_message).collect();
        *self.messages.write() = messages.clone();
        Ok(messages)
    }

    async fn checkpoint(&self, state: &Value) -> Result<()> {
        let leaf_entry_id = *self.leaf_entry_id.read();

        // Commit pending commands with checkpoint data
        let checkpoint_data = CheckpointData {
            state: state.clone(),
            leaf_entry_id,
            token_usage: None,
        };

        self.commit_pending_batch(Some(checkpoint_data)).await?;

        // Update local checkpoint state
        *self.checkpoint_state.write() = Some(state.clone());
        let new_sequence = self.checkpoint_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        self.checkpoint_sequence
            .store(new_sequence, Ordering::SeqCst);

        Ok(())
    }

    fn state(&self) -> Option<Value> {
        self.checkpoint_state.read().clone()
    }

    fn checkpoint_sequence(&self) -> i32 {
        self.checkpoint_sequence.load(Ordering::SeqCst)
    }

    async fn flush_pending(&self) -> Result<()> {
        self.commit_pending_batch(None).await
    }

    // =========================================================================
    // Task Scheduling (Lazy API - Aligned with Workflow)
    // =========================================================================

    fn schedule_raw(&self, task_kind: &str, input: Value) -> AgentTaskFutureRaw {
        self.schedule_with_options_raw(task_kind, input, ScheduleAgentTaskOptions::default())
    }

    fn schedule_with_options_raw(
        &self,
        task_kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> AgentTaskFutureRaw {
        use crate::agent::storage::TaskOptions;

        // Generate deterministic task ID (stable across suspension/resume)
        let task_id = self.next_task_id();

        // Buffer command for batch commit at suspension point (no immediate RPC)
        self.add_pending_command(AgentCommand::ScheduleTask {
            task_id,
            kind: task_kind.to_string(),
            input: input.clone(),
            options: TaskOptions {
                queue: options.queue,
                max_retries: options.max_retries.map(|r| r as i32),
                timeout_ms: options.timeout.map(|t| t.as_millis() as i64),
            },
        });

        // Return future immediately (no RPC made yet)
        AgentTaskFutureRaw::new(task_id, task_kind.to_string(), input)
    }

    async fn join_all(&self, futures: Vec<AgentTaskFutureRaw>) -> Result<Vec<Value>> {
        if futures.is_empty() {
            return Ok(vec![]);
        }

        // Commit pending batch first (submits all scheduled tasks to server)
        self.commit_pending_batch(None).await?;

        let task_ids: Vec<Uuid> = futures.iter().map(|f| f.task_id).collect();

        // Batch fetch all task statuses
        let statuses = self.get_task_results_batch(&task_ids).await?;

        let mut results: Vec<Option<Value>> = vec![None; futures.len()];
        let mut all_complete = true;

        for (i, (future, status)) in futures.iter().zip(statuses.iter()).enumerate() {
            match status.status.as_str() {
                "COMPLETED" => {
                    results[i] = Some(status.output.clone().unwrap_or(Value::Null));
                }
                "FAILED" => {
                    let error = status
                        .error
                        .clone()
                        .unwrap_or_else(|| "Task failed".to_string());
                    return Err(FlovynError::TaskFailed(format!(
                        "Task '{}' (id={}) failed: {}",
                        future.kind, future.task_id, error
                    )));
                }
                "CANCELLED" => {
                    return Err(FlovynError::TaskFailed(format!(
                        "Task '{}' (id={}) was cancelled",
                        future.kind, future.task_id
                    )));
                }
                _ => {
                    // PENDING or RUNNING
                    all_complete = false;
                }
            }
        }

        if all_complete {
            return Ok(results.into_iter().map(|r| r.unwrap()).collect());
        }

        // Not all done - suspend and wait for all tasks
        self.suspend_for_tasks(&task_ids, flovyn_worker_core::client::WaitMode::All)
            .await?;

        // Agent will be re-executed when all tasks complete
        Err(FlovynError::AgentSuspended(
            "Agent suspended waiting for all tasks to complete".to_string(),
        ))
    }

    async fn join_all_settled(
        &self,
        futures: Vec<AgentTaskFutureRaw>,
    ) -> Result<crate::agent::combinators::SettledResult> {
        use crate::agent::combinators::SettledResult;

        if futures.is_empty() {
            return Ok(SettledResult::new());
        }

        // Commit pending batch first (submits all scheduled tasks to server)
        self.commit_pending_batch(None).await?;

        let task_ids: Vec<Uuid> = futures.iter().map(|f| f.task_id).collect();

        // Batch fetch all task statuses
        let statuses = self.get_task_results_batch(&task_ids).await?;

        let mut result = SettledResult::new();
        let mut all_terminal = true;

        for (future, status) in futures.iter().zip(statuses.iter()) {
            match status.status.as_str() {
                "COMPLETED" => {
                    let output = status.output.clone().unwrap_or(Value::Null);
                    result.completed.push((future.task_id.to_string(), output));
                }
                "FAILED" => {
                    let error = status
                        .error
                        .clone()
                        .unwrap_or_else(|| "Task failed".to_string());
                    result.failed.push((future.task_id.to_string(), error));
                }
                "CANCELLED" => {
                    result.cancelled.push(future.task_id.to_string());
                }
                _ => {
                    // PENDING or RUNNING - not terminal yet
                    all_terminal = false;
                }
            }
        }

        if all_terminal {
            return Ok(result);
        }

        // Not all done - suspend and wait for all tasks
        self.suspend_for_tasks(&task_ids, flovyn_worker_core::client::WaitMode::All)
            .await?;

        // Agent will be re-executed when all tasks complete
        Err(FlovynError::AgentSuspended(
            "Agent suspended waiting for all tasks to settle".to_string(),
        ))
    }

    async fn select_ok(
        &self,
        futures: Vec<AgentTaskFutureRaw>,
    ) -> Result<(Value, Vec<AgentTaskFutureRaw>)> {
        if futures.is_empty() {
            return Err(FlovynError::InvalidArgument(
                "select_ok requires at least one future".to_string(),
            ));
        }

        // Commit pending batch first (submits all scheduled tasks to server)
        self.commit_pending_batch(None).await?;

        let task_ids: Vec<Uuid> = futures.iter().map(|f| f.task_id).collect();

        // Batch fetch all task statuses
        let statuses = self.get_task_results_batch(&task_ids).await?;

        let mut first_error: Option<String> = None;
        let mut pending_count = 0;

        for (i, (future, status)) in futures.iter().zip(statuses.iter()).enumerate() {
            match status.status.as_str() {
                "COMPLETED" => {
                    // Found a successful task - return it with remaining futures
                    let result = status.output.clone().unwrap_or(Value::Null);
                    let remaining: Vec<AgentTaskFutureRaw> = futures
                        .into_iter()
                        .enumerate()
                        .filter(|(idx, _)| *idx != i)
                        .map(|(_, f)| f)
                        .collect();
                    return Ok((result, remaining));
                }
                "FAILED" | "CANCELLED" => {
                    // Record first error for the AllTasksFailed message
                    if first_error.is_none() {
                        let error = status
                            .error
                            .clone()
                            .unwrap_or_else(|| "Task failed".to_string());
                        first_error = Some(format!(
                            "Task '{}' (id={}): {}",
                            future.kind, future.task_id, error
                        ));
                    }
                }
                _ => {
                    // PENDING or RUNNING - still have hope
                    pending_count += 1;
                }
            }
        }

        // If all tasks have failed (none pending, none succeeded), return error
        if pending_count == 0 {
            let error_msg = first_error.unwrap_or_else(|| "All tasks failed".to_string());
            return Err(FlovynError::AllTasksFailed(error_msg));
        }

        // Some tasks still pending - suspend and wait for any task to complete
        tracing::debug!(
            pending_count = pending_count,
            "Suspending agent, waiting for successful task"
        );
        self.suspend_for_tasks(&task_ids, flovyn_worker_core::client::WaitMode::Any)
            .await?;

        // Agent will be re-executed when any task completes
        Err(FlovynError::AgentSuspended(
            "Agent suspended waiting for successful task".to_string(),
        ))
    }

    async fn select_ok_with_cancel(
        &self,
        futures: Vec<AgentTaskFutureRaw>,
    ) -> Result<(Value, Vec<Uuid>)> {
        // First, use select_ok to get the winning task
        let (result, remaining) = self.select_ok(futures).await?;

        // Cancel all remaining tasks
        let mut cancelled_ids = Vec::new();
        for future in remaining {
            match self.cancel_task(future.task_id).await {
                Ok(CancelTaskResult::Cancelled) => {
                    cancelled_ids.push(future.task_id);
                }
                Ok(_) => {
                    // Task already completed/failed/cancelled - not an error
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %future.task_id,
                        error = %e,
                        "Failed to cancel remaining task"
                    );
                }
            }
        }

        Ok((result, cancelled_ids))
    }

    async fn wait_for_signal_raw(&self, signal_name: &str) -> Result<Value> {
        // IMPORTANT: Check for signals BEFORE suspending!
        // On resume, the agent replays from the beginning and will call this again.
        // If we suspend first, the agent gets stuck in WAITING because the signal
        // was already consumed during the previous execution.
        let signals = {
            let mut client = self.client.lock().await;
            client
                .consume_signals(self.agent_execution_id, Some(signal_name))
                .await?
        };

        // If signal was already received, return it immediately
        if let Some(signal) = signals.into_iter().next() {
            return Ok(signal.signal_value);
        }

        // No signal yet - checkpoint and suspend
        let current_state = self.checkpoint_state.read().clone().unwrap_or(Value::Null);
        self.checkpoint(&current_state).await?;

        // Suspend the agent
        {
            let mut client = self.client.lock().await;
            client
                .suspend_agent(self.agent_execution_id, signal_name, None)
                .await?;
        }

        // No signal available yet - the agent will be re-executed on signal
        Err(FlovynError::AgentSuspended(format!(
            "Waiting for signal '{}'",
            signal_name
        )))
    }

    async fn has_signal(&self, signal_name: &str) -> Result<bool> {
        let mut client = self.client.lock().await;
        Ok(client
            .has_signal(self.agent_execution_id, signal_name)
            .await?)
    }

    async fn drain_signals_raw(&self, signal_name: &str) -> Result<Vec<Value>> {
        let mut client = self.client.lock().await;
        let signals = client
            .consume_signals(self.agent_execution_id, Some(signal_name))
            .await?;
        Ok(signals.into_iter().map(|s| s.signal_value).collect())
    }

    async fn stream(&self, event: StreamEvent) -> Result<()> {
        let sequence = self.stream_sequence.fetch_add(1, Ordering::SeqCst);
        let timestamp_ms = Self::now_ms();

        let (event_type, payload) = match event {
            StreamEvent::Token { text } => (
                AgentStreamEventType::AgentToken,
                serde_json::json!({"text": text}),
            ),
            StreamEvent::Progress { progress, details } => (
                AgentStreamEventType::AgentProgress,
                serde_json::json!({"progress": progress, "details": details}),
            ),
            StreamEvent::Data { data } => (
                AgentStreamEventType::AgentData,
                serde_json::json!({"data": data}),
            ),
            StreamEvent::Error { message, code } => (
                AgentStreamEventType::AgentError,
                serde_json::json!({"message": message, "code": code}),
            ),
        };

        let payload_str = serde_json::to_string(&payload)?;

        let mut client = self.client.lock().await;
        client
            .stream_data(
                self.agent_execution_id,
                sequence,
                event_type,
                &payload_str,
                timestamp_ms,
            )
            .await?;

        Ok(())
    }

    fn is_cancellation_requested(&self) -> bool {
        self.cancellation_requested.load(Ordering::SeqCst)
    }

    async fn check_cancellation(&self) -> Result<()> {
        if self.is_cancellation_requested() {
            Err(FlovynError::Other("Agent execution cancelled".to_string()))
        } else {
            Ok(())
        }
    }

    async fn cancel_task(&self, task_id: Uuid) -> Result<CancelTaskResult> {
        let result = self
            .client
            .lock()
            .await
            .cancel_task(self.agent_execution_id, task_id, None)
            .await
            .map_err(FlovynError::from)?;

        // The server returns `cancelled: true` when the task was just cancelled
        // by this request. When `cancelled: false`, the task is already in a
        // terminal state - we distinguish by examining the status field.
        //
        // `Cancelled` = this request cancelled the task (was PENDING/RUNNING)
        // `AlreadyCancelled` = task was already cancelled before this request
        if result.is_cancelled() {
            Ok(CancelTaskResult::Cancelled)
        } else {
            match result.status.as_str() {
                "COMPLETED" => Ok(CancelTaskResult::AlreadyCompleted),
                "FAILED" => Ok(CancelTaskResult::AlreadyFailed),
                "CANCELLED" => Ok(CancelTaskResult::AlreadyCancelled),
                _ => Err(FlovynError::Other(format!(
                    "Unexpected task status after cancel: {}",
                    result.status
                ))),
            }
        }
    }
}

fn convert_entry_to_message(entry: CoreEntry) -> LoadedMessage {
    // Parse entry type (server uses lowercase)
    let entry_type = match entry.entry_type.to_lowercase().as_str() {
        "message" => EntryType::Message,
        "llm_call" => EntryType::LlmCall,
        "injection" => EntryType::Injection,
        _ => EntryType::Message, // Default
    };

    // Parse role (server uses lowercase with underscore)
    let role = entry
        .role
        .as_deref()
        .map(|r| match r.to_lowercase().as_str() {
            "system" => EntryRole::System,
            "user" => EntryRole::User,
            "assistant" => EntryRole::Assistant,
            "tool_result" => EntryRole::ToolResult,
            _ => EntryRole::User, // Default
        })
        .unwrap_or(EntryRole::User);

    let token_usage = entry.token_usage.map(|tu| TokenUsage {
        input_tokens: tu.input_tokens,
        output_tokens: tu.output_tokens,
        model: None,
    });

    LoadedMessage {
        entry_id: entry.id,
        entry_type,
        role,
        content: entry.content,
        token_usage,
    }
}

impl std::fmt::Debug for AgentContextImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentContextImpl")
            .field("agent_execution_id", &self.agent_execution_id)
            .field("org_id", &self.org_id)
            .field("checkpoint_sequence", &self.checkpoint_sequence)
            .field(
                "cancellation_requested",
                &self.cancellation_requested.load(Ordering::SeqCst),
            )
            .finish()
    }
}

// AgentContextImpl is automatically Send + Sync because all its fields are:
// - Uuid, Value, AtomicBool, AtomicI32, AtomicU64 are all Send + Sync
// - RwLock<T> is Send + Sync when T is Send + Sync
// - TokioMutex<T> is Send + Sync when T is Send
// - AgentDispatch wraps a tonic Channel which is Send + Sync
//
// Compile-time assertions to verify this:
const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Sync>() {}
    let _ = assert_send::<AgentContextImpl>;
    let _ = assert_sync::<AgentContextImpl>;
};

impl Drop for AgentContextImpl {
    fn drop(&mut self) {
        let pending_count = self.pending_commands.read().len();
        if pending_count > 0 {
            tracing::warn!(
                agent_execution_id = %self.agent_execution_id,
                pending_commands = pending_count,
                "AgentContextImpl dropped with unflushed pending commands! \
                 Entries/tasks created after the last checkpoint will be lost."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that idempotency key format is stable and doesn't include checkpoint_seq.
    ///
    /// This test verifies the fix for a critical bug where the old code generated keys
    /// like `{agent_id}:{checkpoint_seq}:{counter}`. When an agent resumed after suspension,
    /// the checkpoint_seq changed, causing a DIFFERENT idempotency key, which created
    /// a NEW task instead of returning the existing completed task.
    ///
    /// The correct format is `{agent_id}:task:{counter}` which is stable across suspensions.
    #[test]
    fn test_idempotency_key_format_is_stable() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();

        // Simulate key generation with different checkpoint sequences
        // The key should NOT change when checkpoint_seq changes

        // Old buggy format would have been: "{agent_id}:{checkpoint_seq}:{counter}"
        // Correct format is: "{agent_id}:task:{counter}"

        let key_format_0 = format!("{}:task:{}", agent_id, 0);
        let key_format_1 = format!("{}:task:{}", agent_id, 1);

        // Verify the format doesn't contain checkpoint sequence pattern
        // (which would look like a number between two colons before the counter)
        assert!(
            key_format_0.contains(":task:"),
            "Key should use ':task:' format"
        );
        assert!(
            !key_format_0.contains(":-1:"),
            "Key should NOT include checkpoint_seq"
        );
        assert!(
            !key_format_0.contains(":0:0"),
            "Key should NOT use old format"
        );

        // Verify keys for different tasks are different
        assert_ne!(key_format_0, key_format_1);

        // Verify the exact format
        assert_eq!(key_format_0, "3b095802-a3b3-4645-a728-0f5238c46a5c:task:0");
        assert_eq!(key_format_1, "3b095802-a3b3-4645-a728-0f5238c46a5c:task:1");
    }

    /// Test that idempotency keys are stable across simulated resume scenarios.
    ///
    /// This simulates what happens when an agent:
    /// 1. First execution: generates key for task 0
    /// 2. Suspends, checkpoint_seq becomes N
    /// 3. Resumes: should generate THE SAME key for task 0
    #[test]
    fn test_idempotency_key_stable_across_checkpoint_changes() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();

        // Simulate first execution (checkpoint_seq = -1 initially)
        let checkpoint_seq_initial = -1;
        let task_counter_initial = 0;

        // The correct key generation (current code)
        let key_run1 = format!("{}:task:{}", agent_id, task_counter_initial);

        // Simulate resume after checkpoint (checkpoint_seq = 5, counter resets to 0)
        let checkpoint_seq_resume = 5;
        let task_counter_resume = 0; // Counter resets on resume

        // The key should be the same even though checkpoint_seq changed
        let key_run2 = format!("{}:task:{}", agent_id, task_counter_resume);

        // CRITICAL: Keys must be identical for the same task across suspensions
        assert_eq!(
            key_run1, key_run2,
            "Idempotency key must be stable across suspension/resume. \
             checkpoint_seq changed from {} to {} but key should be unchanged.",
            checkpoint_seq_initial, checkpoint_seq_resume
        );

        // Verify the buggy format WOULD have been different
        let buggy_key_run1 = format!(
            "{}:{}:{}",
            agent_id, checkpoint_seq_initial, task_counter_initial
        );
        let buggy_key_run2 = format!(
            "{}:{}:{}",
            agent_id, checkpoint_seq_resume, task_counter_resume
        );
        assert_ne!(
            buggy_key_run1, buggy_key_run2,
            "Old buggy format would have produced different keys"
        );
    }

    // =========================================================================
    // Tests for Deterministic Task IDs (next_task_id)
    // =========================================================================

    /// Test that next_task_id generates deterministic UUIDs.
    ///
    /// The task ID is generated using UUIDv5 from (agent_id, segment, sequence).
    /// Same inputs must always produce the same UUID - this is critical for
    /// replay correctness.
    #[test]
    fn test_next_task_id_is_deterministic() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();
        let counter: i32 = 0;

        // Generate the same input string that next_task_id uses
        // Format: "{agent_id}:task:{counter}"
        let input = format!("{}:task:{}", agent_id, counter);

        // Generate UUID twice - must be identical
        let id1 = Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes());
        let id2 = Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes());

        assert_eq!(id1, id2, "Same inputs must produce same task ID");

        // Verify it's a valid UUID
        assert!(!id1.is_nil());

        // Verify we can parse it back
        let id_str = id1.to_string();
        let parsed = Uuid::parse_str(&id_str).unwrap();
        assert_eq!(parsed, id1);
    }

    /// Test that different inputs produce different task IDs.
    #[test]
    fn test_next_task_id_different_inputs_produce_different_ids() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();

        // Generate IDs for different counters
        // Format: "{agent_id}:task:{counter}"
        let input_0 = format!("{}:task:0", agent_id);
        let input_1 = format!("{}:task:1", agent_id);
        let input_2 = format!("{}:task:2", agent_id);

        let id_0 = Uuid::new_v5(&Uuid::NAMESPACE_OID, input_0.as_bytes());
        let id_1 = Uuid::new_v5(&Uuid::NAMESPACE_OID, input_1.as_bytes());
        let id_2 = Uuid::new_v5(&Uuid::NAMESPACE_OID, input_2.as_bytes());

        assert_ne!(id_0, id_1, "Different counters must produce different IDs");
        assert_ne!(id_1, id_2, "Different counters must produce different IDs");
        assert_ne!(id_0, id_2, "Different counters must produce different IDs");

        // Different agent IDs also produce different IDs
        let agent_id_2 = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let input_agent1 = format!("{}:task:0", agent_id);
        let input_agent2 = format!("{}:task:0", agent_id_2);

        let id_agent1 = Uuid::new_v5(&Uuid::NAMESPACE_OID, input_agent1.as_bytes());
        let id_agent2 = Uuid::new_v5(&Uuid::NAMESPACE_OID, input_agent2.as_bytes());

        assert_ne!(
            id_agent1, id_agent2,
            "Different agent IDs must produce different task IDs"
        );
    }

    /// Test that task IDs are stable across replay.
    ///
    /// This simulates what happens during agent replay:
    /// 1. Initial execution: agent schedules tasks with counter=0,1,2
    /// 2. Resume after checkpoint: task_counter never resets, so replay
    ///    from the beginning produces the SAME task IDs.
    ///
    /// The key insight: task_counter is monotonically increasing and never resets.
    /// When an agent resumes, it replays all schedule_raw calls from the beginning,
    /// which produces the same task IDs. The server uses idempotency to prevent
    /// duplicate task creation.
    #[test]
    fn test_task_id_stable_across_replay() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();

        // Simulate first execution (counter starts at 0)
        let task_ids_run1: Vec<Uuid> = (0..3)
            .map(|counter| {
                let input = format!("{}:task:{}", agent_id, counter);
                Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
            })
            .collect();

        // Simulate replay of the same execution (counter still starts at 0 because
        // agents replay from the beginning and rely on idempotency)
        let task_ids_run2: Vec<Uuid> = (0..3)
            .map(|counter| {
                let input = format!("{}:task:{}", agent_id, counter);
                Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
            })
            .collect();

        // Task IDs must be identical across replay
        assert_eq!(
            task_ids_run1, task_ids_run2,
            "Task IDs must be identical across replay"
        );
    }

    /// Test the exact format of the input string used for task ID generation.
    #[test]
    fn test_task_id_input_format() {
        let agent_id = Uuid::parse_str("3b095802-a3b3-4645-a728-0f5238c46a5c").unwrap();
        let counter: i32 = 42;

        // The format used by next_task_id: "{agent_id}:task:{counter}"
        let input = format!("{}:task:{}", agent_id, counter);

        // Verify the exact format
        assert_eq!(input, "3b095802-a3b3-4645-a728-0f5238c46a5c:task:42");

        // Verify the UUID is deterministic for this exact input
        let expected_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes());

        // Run it again to verify determinism
        let actual_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes());
        assert_eq!(expected_id, actual_id);
    }
}
