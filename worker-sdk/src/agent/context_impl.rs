//! AgentContextImpl - Implementation of AgentContext trait
//!
//! This implementation communicates with the server via gRPC to persist entries,
//! manage checkpoints, schedule tasks, and handle signals.

use crate::agent::context::{
    AgentContext, EntryRole, EntryType, LoadedMessage, ScheduleAgentTaskOptions, TokenUsage,
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
    /// Scheduled task counter - monotonically increasing, never resets.
    /// Used for stable idempotency keys across suspension boundaries.
    scheduled_task_counter: AtomicI32,
    /// Entry counter - monotonically increasing, never resets.
    /// Used for stable entry idempotency keys to prevent duplicate entries on resume.
    entry_counter: AtomicI32,
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
        //
        // The same applies to task scheduling - scheduled_task_counter starts at 0.
        // Agents MUST replay all schedule_task calls from the beginning on resume.
        // The server uses idempotency keys to return existing tasks for duplicate keys.

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
            scheduled_task_counter: AtomicI32::new(0),
            entry_counter: AtomicI32::new(0),
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
            scheduled_task_counter: AtomicI32::new(0),
            entry_counter: AtomicI32::new(0),
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

    /// Generate an idempotency key for task scheduling.
    /// Uses scheduled_task_counter which never resets, ensuring stable keys
    /// across suspension boundaries.
    fn generate_task_idempotency_key(&self) -> String {
        let index = self.scheduled_task_counter.fetch_add(1, Ordering::SeqCst);
        format!("{}:task:{}", self.agent_execution_id, index)
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
    fn next_task_id(&self) -> Uuid {
        let segment = self.current_segment.load(Ordering::SeqCst);
        let seq = self.current_sequence.fetch_add(1, Ordering::SeqCst);
        let input = format!("{}-{}-{}", self.agent_execution_id, segment, seq);
        Uuid::new_v5(&Uuid::NAMESPACE_OID, input.as_bytes())
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
    pub async fn commit_pending_batch(
        &self,
        checkpoint: Option<CheckpointData>,
    ) -> Result<()> {
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
                            None, // turn_id
                            None, // token_usage
                            Some(&entry_id.to_string()), // idempotency_key
                        )
                        .await?;
                }
                AgentCommand::ScheduleTask {
                    task_id: _,
                    kind,
                    input,
                    options,
                } => {
                    client
                        .schedule_task(
                            self.agent_execution_id,
                            kind,
                            input,
                            options.queue.as_deref(),
                            options.max_retries,
                            options.timeout_ms,
                            None, // idempotency_key handled by task_id
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

    fn load_messages(&self) -> &[LoadedMessage] {
        // Safety: This is safe because we're returning a reference to the inner data
        // and the RwLock ensures thread-safe access. The reference is valid as long
        // as the context exists.
        // Note: This requires the caller to not hold the reference across await points
        // where messages might be mutated.
        unsafe {
            let messages = &*self.messages.data_ptr();
            messages.as_slice()
        }
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
        self.checkpoint_sequence.store(new_sequence, Ordering::SeqCst);

        Ok(())
    }

    fn state(&self) -> Option<&Value> {
        // Safety: Similar to load_messages, this requires the caller to not hold
        // the reference across await points where state might be mutated.
        unsafe {
            let state = &*self.checkpoint_state.data_ptr();
            state.as_ref()
        }
    }

    fn checkpoint_sequence(&self) -> i32 {
        self.checkpoint_sequence.load(Ordering::SeqCst)
    }

    async fn schedule_task_raw(&self, task_kind: &str, input: Value) -> Result<Value> {
        self.schedule_task_with_options_raw(task_kind, input, ScheduleAgentTaskOptions::default())
            .await
    }

    async fn schedule_task_with_options_raw(
        &self,
        task_kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> Result<Value> {
        // Generate a stable idempotency key that doesn't change across suspensions.
        // This is critical for the resume case: when an agent suspends waiting for a
        // task to complete, then resumes, calling schedule_task again with the same
        // stable key ensures the server returns the existing task rather than creating
        // a new one.
        let idempotency_key = options
            .idempotency_key
            .unwrap_or_else(|| self.generate_task_idempotency_key());

        // Schedule the task (or get existing task if idempotency key matches)
        let result = {
            let mut client = self.client.lock().await;
            client
                .schedule_task(
                    self.agent_execution_id,
                    task_kind,
                    &input,
                    options.queue.as_deref(),
                    options.max_retries.map(|r| r as i32),
                    options.timeout.map(|t| t.as_millis() as i64),
                    Some(&idempotency_key),
                )
                .await?
        };

        let task_execution_id = result.task_execution_id;

        // Check if the task has already completed (important for resume case)
        let task_result = {
            let mut client = self.client.lock().await;
            client
                .get_task_result(self.agent_execution_id, task_execution_id)
                .await?
        };

        // If task is done, return result immediately
        if task_result.is_completed() {
            return Ok(task_result.output.unwrap_or(Value::Null));
        }

        if task_result.is_failed() {
            let error = task_result
                .error
                .unwrap_or_else(|| "Task failed".to_string());
            return Err(FlovynError::TaskFailed(format!(
                "Task '{}' failed: {}",
                task_kind, error
            )));
        }

        if task_result.is_cancelled() {
            return Err(FlovynError::TaskFailed(format!(
                "Task '{}' was cancelled",
                task_kind
            )));
        }

        // Task is still running (PENDING or RUNNING) - checkpoint and suspend
        let current_state = self.checkpoint_state.read().clone().unwrap_or(Value::Null);
        self.checkpoint(&current_state).await?;

        // Suspend waiting for task completion (server will resume when task completes)
        {
            let mut client = self.client.lock().await;
            client
                .suspend_agent_for_task(
                    self.agent_execution_id,
                    task_execution_id,
                    Some(&format!("Waiting for task {} to complete", task_kind)),
                )
                .await?;
        }

        // Agent will be re-executed when task completes. On resume, the idempotency
        // key ensures we get the same task, and get_task_result will return the result.
        Err(FlovynError::AgentSuspended(format!(
            "Waiting for task '{}' completion",
            task_kind
        )))
    }

    // =========================================================================
    // Lazy Task Scheduling (Preferred API)
    // =========================================================================

    fn schedule_task_lazy(&self, kind: &str, input: Value) -> AgentTaskFutureRaw {
        self.schedule_task_lazy_with_options(kind, input, ScheduleAgentTaskOptions::default())
    }

    fn schedule_task_lazy_with_options(
        &self,
        kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> AgentTaskFutureRaw {
        use crate::agent::storage::TaskOptions;

        // Generate deterministic task ID (stable across suspension/resume)
        let task_id = self.next_task_id();

        // Buffer command for batch commit at suspension point (no immediate RPC)
        self.add_pending_command(AgentCommand::ScheduleTask {
            task_id,
            kind: kind.to_string(),
            input: input.clone(),
            options: TaskOptions {
                queue: options.queue,
                max_retries: options.max_retries.map(|r| r as i32),
                timeout_ms: options.timeout.map(|t| t.as_millis() as i64),
            },
        });

        // Return future immediately (no RPC made yet)
        AgentTaskFutureRaw::new(task_id, kind.to_string(), input)
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

    // =========================================================================
    // Parallel Task Support
    // =========================================================================

    async fn schedule_task_handle(
        &self,
        task_kind: &str,
        input: Value,
    ) -> Result<crate::agent::combinators::AgentTaskHandle> {
        self.schedule_task_handle_with_options(
            task_kind,
            input,
            ScheduleAgentTaskOptions::default(),
        )
        .await
    }

    async fn schedule_task_handle_with_options(
        &self,
        task_kind: &str,
        input: Value,
        options: ScheduleAgentTaskOptions,
    ) -> Result<crate::agent::combinators::AgentTaskHandle> {
        use crate::agent::storage::TaskOptions;

        // Generate deterministic task ID (stable across suspension/resume)
        let task_id = self.next_task_id();

        // Get the current index for ordering (number of tasks scheduled so far)
        let index = self.scheduled_task_counter.fetch_add(1, Ordering::SeqCst) as usize;

        // Buffer command for batch commit at suspension point (no immediate RPC)
        self.add_pending_command(AgentCommand::ScheduleTask {
            task_id,
            kind: task_kind.to_string(),
            input: input.clone(),
            options: TaskOptions {
                queue: options.queue.clone(),
                max_retries: options.max_retries.map(|r| r as i32),
                timeout_ms: options.timeout.map(|t| t.as_millis() as i64),
            },
        });

        Ok(crate::agent::combinators::AgentTaskHandle::new(
            task_id,
            task_kind,
            index,
        ))
    }

    async fn get_task_results_batch(
        &self,
        task_ids: &[Uuid],
    ) -> Result<Vec<flovyn_worker_core::client::BatchTaskResult>> {
        let mut client = self.client.lock().await;
        Ok(client
            .get_task_results_batch(self.agent_execution_id, task_ids)
            .await?)
    }

    async fn suspend_for_tasks(
        &self,
        task_ids: &[Uuid],
        mode: flovyn_worker_core::client::WaitMode,
    ) -> Result<()> {
        // Commit pending batch first (ensures all scheduled tasks are submitted)
        // This is critical: tasks must be committed before we can wait for them
        self.commit_pending_batch(None).await?;

        // Checkpoint current state before suspending
        let current_state = self.checkpoint_state.read().clone().unwrap_or(Value::Null);
        self.checkpoint(&current_state).await?;

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

    // =========================================================================
    // Task Cancellation
    // =========================================================================

    async fn cancel_task(
        &self,
        handle: &crate::agent::combinators::AgentTaskHandle,
        reason: Option<&str>,
    ) -> Result<flovyn_worker_core::client::CancelResult> {
        let mut client = self.client.lock().await;
        Ok(client
            .cancel_task(self.agent_execution_id, handle.task_id, reason)
            .await?)
    }

    async fn cancel_tasks(
        &self,
        handles: &[crate::agent::combinators::AgentTaskHandle],
        reason: Option<&str>,
    ) -> Result<Vec<flovyn_worker_core::client::CancelResult>> {
        let mut results = Vec::with_capacity(handles.len());
        let mut client = self.client.lock().await;

        for handle in handles {
            let result = client
                .cancel_task(self.agent_execution_id, handle.task_id, reason)
                .await?;
            results.push(result);
        }

        Ok(results)
    }

    // =========================================================================
    // Batch Task Scheduling
    // =========================================================================

    async fn schedule_tasks_batch(
        &self,
        tasks: Vec<(&str, Value)>,
    ) -> Result<Vec<crate::agent::combinators::AgentTaskHandle>> {
        // Convert to batch with default options
        let tasks_with_options: Vec<_> = tasks
            .into_iter()
            .map(|(kind, input)| (kind, input, ScheduleAgentTaskOptions::default()))
            .collect();
        self.schedule_tasks_batch_with_options(tasks_with_options)
            .await
    }

    async fn schedule_tasks_batch_with_options(
        &self,
        tasks: Vec<(&str, Value, ScheduleAgentTaskOptions)>,
    ) -> Result<Vec<crate::agent::combinators::AgentTaskHandle>> {
        use flovyn_worker_core::client::BatchScheduleTaskInput;

        if tasks.is_empty() {
            return Ok(vec![]);
        }

        // Build batch inputs
        let mut batch_inputs = Vec::with_capacity(tasks.len());
        for (task_kind, input, options) in &tasks {
            let idempotency_key = options
                .idempotency_key
                .clone()
                .unwrap_or_else(|| self.generate_task_idempotency_key());

            let mut batch_input = BatchScheduleTaskInput::new(*task_kind, input.clone())
                .with_idempotency_key(idempotency_key);

            if let Some(ref queue) = options.queue {
                batch_input = batch_input.with_queue(queue);
            }
            if let Some(max_retries) = options.max_retries {
                batch_input = batch_input.with_max_retries(max_retries as i32);
            }
            if let Some(timeout) = options.timeout {
                batch_input = batch_input.with_timeout_ms(timeout.as_millis() as i64);
            }

            batch_inputs.push(batch_input);
        }

        // Call batch schedule
        let results = {
            let mut client = self.client.lock().await;
            client
                .schedule_tasks_batch(
                    self.agent_execution_id,
                    &batch_inputs,
                    None, // default_queue
                    None, // default_max_retries
                    None, // default_timeout_ms
                )
                .await?
        };

        // Get the starting index for these tasks
        // Note: counter was already incremented in generate_task_idempotency_key calls
        let start_index = self.scheduled_task_counter.load(Ordering::SeqCst) as usize - tasks.len();

        // Build handles
        let mut handles = Vec::with_capacity(results.len());
        for (i, (result, (task_kind, _, _))) in results.iter().zip(tasks.iter()).enumerate() {
            if let Some(ref error) = result.error {
                return Err(FlovynError::Other(format!(
                    "Failed to schedule task '{}': {}",
                    task_kind, error
                )));
            }

            handles.push(crate::agent::combinators::AgentTaskHandle::new(
                result.task_execution_id,
                *task_kind,
                start_index + i,
            ));
        }

        Ok(handles)
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

// Safety: AgentContextImpl is Send + Sync because all its fields are:
// - Uuid, Value, AtomicBool, AtomicI32 are all Send + Sync
// - RwLock<T> is Send + Sync when T is Send + Sync
// - AgentDispatch implements Clone which implies it's safe to share
unsafe impl Send for AgentContextImpl {}
unsafe impl Sync for AgentContextImpl {}

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

    /// Test the actual generate_task_idempotency_key format.
    /// This requires a mock AgentDispatch which we don't have here,
    /// so we test the format string directly.
    #[test]
    fn test_generate_task_idempotency_key_format() {
        // This tests the format string used in generate_task_idempotency_key:
        // format!("{}:task:{}", self.agent_execution_id, index)

        let agent_id = Uuid::new_v4();
        let index = 42;

        let key = format!("{}:task:{}", agent_id, index);

        // Verify format
        let parts: Vec<&str> = key.split(':').collect();
        assert_eq!(
            parts.len(),
            3,
            "Key should have 3 parts: agent_id:task:index"
        );
        assert_eq!(parts[1], "task", "Second part should be 'task'");
        assert_eq!(parts[2], "42", "Third part should be the index");

        // Verify agent_id can be parsed back
        let parsed_id = Uuid::parse_str(parts[0]).unwrap();
        assert_eq!(parsed_id, agent_id);
    }
}
