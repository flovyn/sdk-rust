//! AgentContextImpl - Implementation of AgentContext trait
//!
//! This implementation communicates with the server via gRPC to persist entries,
//! manage checkpoints, schedule tasks, and handle signals.

use crate::agent::child::{
    AgentMode, CancellationMode, ChildEvent, ChildEventInfo, ChildHandle, HandoffCompletion,
    HandoffOptions, SpawnOptions,
};
use crate::agent::context::{
    AgentContext, CancelTaskResult, EntryRole, EntryType, LoadedMessage, ScheduleAgentTaskOptions,
    TokenUsage,
};
use crate::agent::executor::TaskExecutor;
use crate::agent::future::AgentTaskFutureRaw;
use crate::agent::queue::QueueContext;
use crate::agent::signals::SignalSource;
use crate::agent::storage::{AgentCommand, AgentStorage, CheckpointData, CommandBatch};
use crate::error::{FlovynError, Result};
use crate::task::streaming::StreamEvent;
use async_trait::async_trait;
use flovyn_worker_core::client::{AgentDispatch, AgentEntry as CoreEntry};
use flovyn_worker_core::generated::flovyn_v1::AgentStreamEventType;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

/// Serialize JSON to canonical form (sorted keys) for consistent hashing.
///
/// This ensures that `{"a":1,"b":2}` and `{"b":2,"a":1}` produce the same string,
/// which is required for content-based idempotency keys.
fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut pairs: Vec<_> = map.iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            let inner: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", k, canonical_json_string(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(canonical_json_string).collect();
            format!("[{}]", inner.join(","))
        }
        _ => value.to_string(),
    }
}

/// Implementation of AgentContext that communicates with the server via gRPC.
///
/// # ID Generation
///
/// **Entry IDs** use random UUIDs (`Uuid::new_v4()`), NOT counter-based IDs.
/// This is intentional: agents use checkpoint-based recovery, not deterministic replay.
/// Counter-based entry IDs fail when:
/// - Conditional code paths exist (signal handling, branching)
/// - Compaction changes entry indices
/// - Branching creates different entry chains
///
/// **Task IDs** use content-based hashing for idempotency.
/// Key = `hash(agent_id + kind + canonical_json(input))`.
/// This ensures:
/// - Resume: same task + same input → same key → server returns existing task
/// - Branching: different tasks → different keys → no collision
/// - Local mode: same formula works without server
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
    /// Stream sequence counter
    stream_sequence: AtomicI32,
    /// gRPC client (tokio mutex for async-safe access)
    /// Still needed for RPCs not yet abstracted (PollAgent, CompleteAgent,
    /// FailAgent, SuspendAgent, StreamAgentData, etc.)
    client: TokioMutex<AgentDispatch>,
    /// Pluggable storage backend (remote gRPC, SQLite, in-memory)
    storage: Arc<dyn AgentStorage>,
    /// Pluggable task executor (remote or local in-process)
    _task_executor: Arc<dyn TaskExecutor>,
    /// Pluggable signal source (remote, channel, stdin)
    _signal_source: Arc<dyn SignalSource>,
    /// Cancellation flag
    cancellation_requested: AtomicBool,
    /// Pending commands to be committed at next suspension point
    pending_commands: RwLock<Vec<AgentCommand>>,
    /// Current segment number (increments on each recovery)
    current_segment: AtomicU64,
    /// Current sequence within segment
    current_sequence: AtomicU64,
    /// Parent execution ID (if this agent was spawned as a child)
    parent_execution_id: Option<Uuid>,
    /// Tracked child agent handles (child_id -> ChildHandle)
    children: RwLock<HashMap<Uuid, ChildHandle>>,
    /// Queue context for resolving target queues when spawning children
    queue_context: Option<QueueContext>,
    /// Current turn ID for grouping entries by conversation turn
    current_turn_id: RwLock<Option<String>>,
}

impl AgentContextImpl {
    /// Create a new AgentContextImpl with default remote backends.
    ///
    /// This loads the initial state from the server (entries and checkpoint).
    /// Uses `RemoteStorage`, `RemoteTaskExecutor`, and `RemoteSignalSource`
    /// as the default backends, preserving existing remote-mode behavior.
    pub async fn new(
        client: AgentDispatch,
        agent_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        current_checkpoint_seq: i32,
    ) -> Result<Self> {
        use crate::agent::executor::RemoteTaskExecutor;
        use crate::agent::signals::RemoteSignalSource;
        use crate::agent::storage::RemoteStorage;

        // Create default remote backends from the client
        let storage = Arc::new(RemoteStorage::new(client.clone(), org_id)) as Arc<dyn AgentStorage>;
        let task_executor = Arc::new(RemoteTaskExecutor::new()) as Arc<dyn TaskExecutor>;
        let signal_source = Arc::new(RemoteSignalSource::new()) as Arc<dyn SignalSource>;

        Self::with_backends(
            client,
            agent_execution_id,
            org_id,
            input,
            current_checkpoint_seq,
            storage,
            task_executor,
            signal_source,
        )
        .await
    }

    /// Create a new AgentContextImpl with custom backends.
    ///
    /// This allows plugging in different storage, executor, and signal
    /// implementations for local mode, testing, or hybrid configurations.
    #[allow(clippy::too_many_arguments)]
    pub async fn with_backends(
        mut client: AgentDispatch,
        agent_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        current_checkpoint_seq: i32,
        storage: Arc<dyn AgentStorage>,
        task_executor: Arc<dyn TaskExecutor>,
        signal_source: Arc<dyn SignalSource>,
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
            stream_sequence: AtomicI32::new(0),
            client: TokioMutex::new(client),
            storage,
            _task_executor: task_executor,
            _signal_source: signal_source,
            cancellation_requested: AtomicBool::new(false),
            pending_commands: RwLock::new(Vec::new()),
            current_segment: AtomicU64::new(segment),
            current_sequence: AtomicU64::new(0),
            parent_execution_id: None,
            children: RwLock::new(HashMap::new()),
            queue_context: None,
            current_turn_id: RwLock::new(None),
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
        use crate::agent::executor::RemoteTaskExecutor;
        use crate::agent::signals::RemoteSignalSource;
        use crate::agent::storage::RemoteStorage;

        let storage = Arc::new(RemoteStorage::new(client.clone(), org_id)) as Arc<dyn AgentStorage>;
        let task_executor = Arc::new(RemoteTaskExecutor::new()) as Arc<dyn TaskExecutor>;
        let signal_source = Arc::new(RemoteSignalSource::new()) as Arc<dyn SignalSource>;

        Self::from_loaded_with_backends(
            client,
            agent_execution_id,
            org_id,
            input,
            messages,
            checkpoint_state,
            checkpoint_sequence,
            leaf_entry_id,
            storage,
            task_executor,
            signal_source,
        )
    }

    /// Create from pre-loaded data with custom backends.
    #[allow(clippy::too_many_arguments)]
    pub fn from_loaded_with_backends(
        client: AgentDispatch,
        agent_execution_id: Uuid,
        org_id: Uuid,
        input: Value,
        messages: Vec<LoadedMessage>,
        checkpoint_state: Option<Value>,
        checkpoint_sequence: i32,
        leaf_entry_id: Option<Uuid>,
        storage: Arc<dyn AgentStorage>,
        task_executor: Arc<dyn TaskExecutor>,
        signal_source: Arc<dyn SignalSource>,
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
            stream_sequence: AtomicI32::new(0),
            client: TokioMutex::new(client),
            storage,
            _task_executor: task_executor,
            _signal_source: signal_source,
            cancellation_requested: AtomicBool::new(false),
            pending_commands: RwLock::new(Vec::new()),
            current_segment: AtomicU64::new(segment),
            current_sequence: AtomicU64::new(0),
            parent_execution_id: None,
            children: RwLock::new(HashMap::new()),
            queue_context: None,
            current_turn_id: RwLock::new(None),
        }
    }

    /// Set the parent execution ID (called by the agent worker when creating the context)
    pub fn set_parent_execution_id(&mut self, parent_id: Option<Uuid>) {
        self.parent_execution_id = parent_id;
    }

    /// Set the current turn ID. All subsequent `append_entry` calls will include
    /// this turn_id until it is changed again.
    pub fn set_turn_id(&self, turn_id: Option<String>) {
        *self.current_turn_id.write() = turn_id;
    }

    /// Set the queue context for resolving target queues when spawning children
    pub fn set_queue_context(&mut self, queue_context: QueueContext) {
        self.queue_context = Some(queue_context);
    }

    /// Request cancellation
    pub fn request_cancellation(&self) {
        self.cancellation_requested.store(true, Ordering::SeqCst);
    }

    /// Get current timestamp in milliseconds
    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }

    /// Generate task idempotency key from content hash.
    ///
    /// Key = hash(agent_id + kind + canonical_json(input))
    ///
    /// This ensures:
    /// - Resume: same task + same input → same key → server returns existing task
    /// - Branching: different tasks → different keys → no collision
    /// - Local mode: same formula works without server
    fn generate_task_idempotency_key(&self, kind: &str, input: &Value) -> String {
        use sha2::{Digest, Sha256};

        let canonical = canonical_json_string(input);

        // Log first 500 chars of canonical JSON for debugging
        let preview: String = canonical.chars().take(500).collect();
        tracing::debug!(
            agent_execution_id = %self.agent_execution_id,
            kind = kind,
            canonical_preview = %preview,
            canonical_len = canonical.len(),
            "generate_task_idempotency_key: input"
        );

        let mut hasher = Sha256::new();
        hasher.update(self.agent_execution_id.as_bytes());
        hasher.update(b":");
        hasher.update(kind.as_bytes());
        hasher.update(b":");
        hasher.update(canonical.as_bytes());
        // Use first 16 bytes (128 bits) - sufficient for uniqueness
        let hash = hasher.finalize();
        let key = format!("task:{}", hex::encode(&hash[..16]));

        tracing::info!(
            agent_execution_id = %self.agent_execution_id,
            kind = kind,
            idempotency_key = %key,
            "generate_task_idempotency_key: result"
        );

        key
    }

    /// Generate task ID from idempotency key (deterministic).
    fn task_id_from_idempotency_key(&self, idempotency_key: &str) -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_OID, idempotency_key.as_bytes())
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

        let cmd_count = commands.len();
        let has_checkpoint = checkpoint.is_some();
        let checkpoint_leaf = checkpoint.as_ref().and_then(|c| c.leaf_entry_id);

        tracing::debug!(
            agent_execution_id = %self.agent_execution_id,
            command_count = cmd_count,
            has_checkpoint = has_checkpoint,
            checkpoint_leaf_entry_id = ?checkpoint_leaf,
            "commit_pending_batch called"
        );

        if commands.is_empty() && checkpoint.is_none() {
            tracing::debug!("commit_pending_batch: nothing to commit, returning early");
            return Ok(());
        }

        let batch = CommandBatch {
            segment: self.current_segment.load(Ordering::SeqCst),
            sequence: self.current_sequence.load(Ordering::SeqCst),
            commands,
            checkpoint,
        };

        // Delegate to the pluggable storage backend
        self.storage
            .commit_batch(self.agent_execution_id, batch)
            .await
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

    /// Batch fetch results for multiple tasks via the storage backend.
    ///
    /// This is an internal helper used by `join_all` and `select_ok` to efficiently
    /// check the status of multiple tasks in a single call.
    async fn get_task_results_batch(
        &self,
        task_ids: &[Uuid],
    ) -> Result<Vec<crate::agent::storage::TaskResult>> {
        self.storage
            .get_task_results(self.agent_execution_id, task_ids)
            .await
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

    fn set_turn_id(&self, turn_id: Option<String>) {
        *self.current_turn_id.write() = turn_id;
    }

    async fn append_entry(&self, role: EntryRole, content: &Value) -> Result<Uuid> {
        let parent_id = *self.leaf_entry_id.read();
        // Use random UUID - agents don't need counter-based IDs because they
        // use checkpoint-based recovery, not deterministic replay.
        let entry_id = Uuid::new_v4();

        // Buffer command for batch commit (no immediate RPC)
        self.add_pending_command(AgentCommand::AppendEntry {
            entry_id,
            parent_id,
            role: role.as_str().to_string(),
            content: content.clone(),
            turn_id: self.current_turn_id.read().clone(),
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
        let entry_id = Uuid::new_v4();
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
            turn_id: self.current_turn_id.read().clone(),
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
        let entry_id = Uuid::new_v4();
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
            turn_id: self.current_turn_id.read().clone(),
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
        let entry_id = Uuid::new_v4();
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
            turn_id: self.current_turn_id.read().clone(),
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

        tracing::info!(
            agent_execution_id = %self.agent_execution_id,
            leaf_entry_id = ?leaf_entry_id,
            state_is_null = state.is_null(),
            "checkpoint() called with explicit state"
        );

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

        tracing::info!(
            agent_execution_id = %self.agent_execution_id,
            new_sequence = new_sequence,
            "checkpoint() completed"
        );

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

        // Generate content-based idempotency key and task ID.
        // Key = hash(agent_id + kind + canonical_json(input))
        // This ensures:
        // - Resume: same task + same input → same key → server returns existing task
        // - Branching: different tasks → different keys → no collision
        let idempotency_key = self.generate_task_idempotency_key(task_kind, &input);
        let task_id = self.task_id_from_idempotency_key(&idempotency_key);

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
            idempotency_key,
        });

        // Return future immediately (no RPC made yet)
        AgentTaskFutureRaw::new(task_id, task_kind.to_string(), input)
    }

    async fn join_all(&self, futures: Vec<AgentTaskFutureRaw>) -> Result<Vec<Value>> {
        use crate::agent::storage::TaskStatus;

        if futures.is_empty() {
            return Ok(vec![]);
        }

        let task_ids: Vec<Uuid> = futures.iter().map(|f| f.task_id).collect();
        tracing::info!(
            agent_execution_id = %self.agent_execution_id,
            task_count = futures.len(),
            task_ids = ?task_ids,
            "join_all: starting"
        );

        // Commit pending batch first (submits all scheduled tasks to server)
        self.commit_pending_batch(None).await?;

        // Batch fetch all task statuses
        let statuses = self.get_task_results_batch(&task_ids).await?;

        let mut results: Vec<Option<Value>> = vec![None; futures.len()];
        let mut all_complete = true;

        for (i, (future, status)) in futures.iter().zip(statuses.iter()).enumerate() {
            tracing::debug!(
                agent_execution_id = %self.agent_execution_id,
                task_id = %future.task_id,
                task_status = %status.status,
                "join_all: task status"
            );
            match status.status {
                TaskStatus::Completed => {
                    results[i] = Some(status.output.clone().unwrap_or(Value::Null));
                }
                TaskStatus::Failed => {
                    let error = status
                        .error
                        .clone()
                        .unwrap_or_else(|| "Task failed".to_string());
                    return Err(FlovynError::TaskFailed(format!(
                        "Task '{}' (id={}) failed: {}",
                        future.kind, future.task_id, error
                    )));
                }
                TaskStatus::Cancelled => {
                    return Err(FlovynError::TaskFailed(format!(
                        "Task '{}' (id={}) was cancelled",
                        future.kind, future.task_id
                    )));
                }
                TaskStatus::Pending | TaskStatus::Running => {
                    all_complete = false;
                }
            }
        }

        if all_complete {
            tracing::info!(
                agent_execution_id = %self.agent_execution_id,
                "join_all: all tasks complete, returning results"
            );
            return Ok(results.into_iter().map(|r| r.unwrap()).collect());
        }

        tracing::info!(
            agent_execution_id = %self.agent_execution_id,
            "join_all: not all tasks complete, suspending"
        );

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
        use crate::agent::storage::TaskStatus;

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
            match status.status {
                TaskStatus::Completed => {
                    let output = status.output.clone().unwrap_or(Value::Null);
                    result.completed.push((future.task_id.to_string(), output));
                }
                TaskStatus::Failed => {
                    let error = status
                        .error
                        .clone()
                        .unwrap_or_else(|| "Task failed".to_string());
                    result.failed.push((future.task_id.to_string(), error));
                }
                TaskStatus::Cancelled => {
                    result.cancelled.push(future.task_id.to_string());
                }
                TaskStatus::Pending | TaskStatus::Running => {
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
        use crate::agent::storage::TaskStatus;

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
            match status.status {
                TaskStatus::Completed => {
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
                TaskStatus::Failed | TaskStatus::Cancelled => {
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
                TaskStatus::Pending | TaskStatus::Running => {
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

        // If signal was already received, return it immediately
        if let Some(value) = self
            .storage
            .pop_signal(self.agent_execution_id, signal_name)
            .await?
        {
            return Ok(value);
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
        Ok(self
            .storage
            .has_signal(self.agent_execution_id, signal_name)
            .await?)
    }

    async fn drain_signals_raw(&self, signal_name: &str) -> Result<Vec<Value>> {
        // Pop signals one at a time until none remain
        let mut values = Vec::new();
        while let Some(value) = self
            .storage
            .pop_signal(self.agent_execution_id, signal_name)
            .await?
        {
            values.push(value);
        }
        Ok(values)
    }

    async fn drain_signals_by_pattern(&self, pattern: &str) -> Result<Vec<(String, Value)>> {
        Ok(self
            .storage
            .drain_signals_by_pattern(self.agent_execution_id, pattern)
            .await?)
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

    // =========================================================================
    // Hierarchical Agent Operations
    // =========================================================================

    async fn spawn_agent(
        &self,
        mode: AgentMode,
        input: Value,
        options: SpawnOptions,
    ) -> Result<ChildHandle> {
        let (agent_kind, mode_str): (String, &str) = match &mode {
            AgentMode::Remote(kind) => (kind.clone(), "REMOTE"),
            AgentMode::Local(kind) => (kind.clone(), "LOCAL"),
            AgentMode::External(_) => {
                return Err(FlovynError::NotSupported(
                    "External agent spawning not yet implemented".into(),
                ));
            }
        };

        // Only send an explicit queue override to the server.
        // When no explicit queue is set, the server handles fallback:
        //   template's default_queue → parent's queue
        let resolved_queue = options.queue.clone();

        let input_bytes = serde_json::to_vec(&input)?;
        let max_budget_tokens = options
            .budget
            .as_ref()
            .and_then(|b| b.max_tokens.map(|t| t as i64));

        let child_id = self
            .client
            .lock()
            .await
            .spawn_child_agent(
                self.org_id,
                self.agent_execution_id,
                &agent_kind,
                &input_bytes,
                resolved_queue.as_deref(),
                mode_str,
                max_budget_tokens,
                options.template.as_deref(),
            )
            .await
            .map_err(FlovynError::from)?;

        let handle = ChildHandle {
            child_id,
            mode: mode.clone(),
        };

        // Track the child handle
        self.children.write().insert(child_id, handle.clone());

        Ok(handle)
    }

    async fn signal_child(&self, handle: &ChildHandle, name: &str, payload: Value) -> Result<()> {
        let payload_bytes = serde_json::to_vec(&payload)?;
        self.client
            .lock()
            .await
            .send_child_signal(
                self.org_id,
                self.agent_execution_id,
                handle.child_id,
                name,
                &payload_bytes,
            )
            .await
            .map_err(FlovynError::from)?;
        Ok(())
    }

    async fn signal_parent(&self, name: &str, payload: Value) -> Result<()> {
        let parent_id = self.parent_execution_id.ok_or_else(|| {
            FlovynError::InvalidArgument("This agent has no parent to signal".into())
        })?;
        let _ = parent_id; // Used for validation only; the server resolves parent from child_id

        let payload_bytes = serde_json::to_vec(&payload)?;
        self.client
            .lock()
            .await
            .send_parent_signal(self.org_id, self.agent_execution_id, name, &payload_bytes)
            .await
            .map_err(FlovynError::from)?;
        Ok(())
    }

    async fn cancel_child(&self, handle: &ChildHandle, _mode: CancellationMode) -> Result<()> {
        // Send a "cancel" signal to the child agent
        let payload_bytes = serde_json::to_vec(&Value::Null)?;
        self.client
            .lock()
            .await
            .send_child_signal(
                self.org_id,
                self.agent_execution_id,
                handle.child_id,
                "cancel",
                &payload_bytes,
            )
            .await
            .map_err(FlovynError::from)?;
        Ok(())
    }

    async fn poll_child_events(&self, handles: &[ChildHandle]) -> Result<Vec<ChildEventInfo>> {
        let child_ids: Vec<Uuid> = handles.iter().map(|h| h.child_id).collect();
        let results = self
            .client
            .lock()
            .await
            .poll_child_events(self.org_id, self.agent_execution_id, &child_ids)
            .await
            .map_err(FlovynError::from)?;

        Ok(results
            .into_iter()
            .map(|r| {
                let event = match r.event_type.as_str() {
                    "completed" => ChildEvent::Completed {
                        child_id: r.child_execution_id,
                        output: r.output.unwrap_or(Value::Null),
                    },
                    "failed" => ChildEvent::Failed {
                        child_id: r.child_execution_id,
                        error: r.error.unwrap_or_default(),
                    },
                    "signal" => ChildEvent::Signal {
                        child_id: r.child_execution_id,
                        signal_name: r.signal_name.unwrap_or_default(),
                        payload: r.signal_payload.unwrap_or(Value::Null),
                    },
                    _ => ChildEvent::Failed {
                        child_id: r.child_execution_id,
                        error: format!("Unknown event type: {}", r.event_type),
                    },
                };
                ChildEventInfo {
                    child_id: r.child_execution_id,
                    event,
                }
            })
            .collect())
    }

    async fn join_children(&self, handles: &[ChildHandle]) -> Result<Vec<ChildEvent>> {
        if handles.is_empty() {
            return Ok(vec![]);
        }

        // First poll to see if all children are already done
        let events = self.poll_child_events(handles).await?;
        if events.len() == handles.len() {
            return Ok(events.into_iter().map(|e| e.event).collect());
        }

        // Not all done yet — checkpoint and suspend waiting for all children
        let current_state = self.checkpoint_state.read().clone().unwrap_or(Value::Null);
        let leaf_entry_id = *self.leaf_entry_id.read();
        let checkpoint_data = CheckpointData {
            state: current_state.clone(),
            leaf_entry_id,
            token_usage: None,
        };
        self.commit_pending_batch(Some(checkpoint_data)).await?;
        *self.checkpoint_state.write() = Some(current_state);
        self.checkpoint_sequence.fetch_add(1, Ordering::SeqCst);

        let child_ids: Vec<Uuid> = handles.iter().map(|h| h.child_id).collect();
        self.client
            .lock()
            .await
            .suspend_agent_for_all_children(
                self.agent_execution_id,
                &child_ids,
                Some("Waiting for all children to complete"),
            )
            .await
            .map_err(FlovynError::from)?;

        Err(FlovynError::AgentSuspended(
            "Agent suspended waiting for all children to complete".to_string(),
        ))
    }

    async fn select_child(&self, handles: &[ChildHandle]) -> Result<ChildEvent> {
        if handles.is_empty() {
            return Err(FlovynError::InvalidArgument(
                "select_child requires at least one handle".into(),
            ));
        }

        // First poll to see if any child has an event
        let events = self.poll_child_events(handles).await?;
        if let Some(first) = events.into_iter().next() {
            return Ok(first.event);
        }

        // No events yet — checkpoint and suspend waiting for any child event
        let current_state = self.checkpoint_state.read().clone().unwrap_or(Value::Null);
        let leaf_entry_id = *self.leaf_entry_id.read();
        let checkpoint_data = CheckpointData {
            state: current_state.clone(),
            leaf_entry_id,
            token_usage: None,
        };
        self.commit_pending_batch(Some(checkpoint_data)).await?;
        *self.checkpoint_state.write() = Some(current_state);
        self.checkpoint_sequence.fetch_add(1, Ordering::SeqCst);

        let child_ids: Vec<Uuid> = handles.iter().map(|h| h.child_id).collect();
        self.client
            .lock()
            .await
            .suspend_agent_for_child_event(
                self.agent_execution_id,
                &child_ids,
                Some("Waiting for any child event"),
            )
            .await
            .map_err(FlovynError::from)?;

        Err(FlovynError::AgentSuspended(
            "Agent suspended waiting for child event".to_string(),
        ))
    }

    async fn get_child_handle(&self, child_id: Uuid) -> Result<ChildHandle> {
        // First check local in-memory map (populated during this execution)
        let children = self.children.read();
        if let Some(handle) = children.get(&child_id).cloned() {
            return Ok(handle);
        }
        drop(children);

        // On resume, child handles from a previous execution are not in memory.
        // Construct a handle with Remote mode as a sensible default — the child_id
        // is the important part for join/signal/cancel operations.
        Ok(ChildHandle {
            child_id,
            mode: AgentMode::Remote(String::new()),
        })
    }

    async fn handoff_to_agent(
        &self,
        mode: AgentMode,
        input: Value,
        options: HandoffOptions,
    ) -> Result<Value> {
        let spawn_opts = options.spawn.unwrap_or_default();
        let handle = self.spawn_agent(mode, input, spawn_opts).await?;

        let completion = options.completion.unwrap_or_default();
        match completion {
            HandoffCompletion::WaitForChild => {
                let events = self.join_children(&[handle]).await?;
                match events.into_iter().next() {
                    Some(ChildEvent::Completed { output, .. }) => Ok(output),
                    Some(ChildEvent::Failed { error, .. }) => Err(FlovynError::Other(format!(
                        "Handoff child failed: {}",
                        error
                    ))),
                    _ => Err(FlovynError::Other("Unexpected child event".into())),
                }
            }
            HandoffCompletion::Immediate => Ok(Value::Null),
        }
    }

    fn parent_execution_id(&self) -> Option<Uuid> {
        self.parent_execution_id
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

    // =========================================================================
    // Tests for Random UUID Generation
    // =========================================================================
    //
    // Agents use random UUIDs for entry and task IDs because they use
    // checkpoint-based recovery, NOT deterministic replay.
    //
    // Counter-based IDs were removed because they fail when:
    // - Conditional code paths exist (signal handling, branching)
    // - Compaction changes entry indices
    // - Branching creates different entry chains
    //
    // Random UUIDs work in all cases because each entry/task gets a unique ID
    // regardless of execution order.

    /// Test that random UUIDs are unique
    #[test]
    fn test_random_uuid_is_unique() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    /// Test that random UUIDs can be used as idempotency keys
    #[test]
    fn test_random_uuid_as_idempotency_key() {
        let entry_id = Uuid::new_v4();
        let idempotency_key = entry_id.to_string();

        // Server parses idempotency_key back to UUID and uses it as entry_id
        // (see DomainAgentEntry::new_with_idempotency)
        let parsed_id = Uuid::parse_str(&idempotency_key).unwrap();
        assert_eq!(entry_id, parsed_id);
    }

    /// Test that entry IDs work correctly with parent-child chains
    #[test]
    fn test_entry_chain_with_random_ids() {
        // Simulate a conversation chain
        let entry1_id = Uuid::new_v4();
        let entry2_id = Uuid::new_v4();
        let entry3_id = Uuid::new_v4();

        // Each entry has a parent pointer - this is how conversation structure is maintained
        let _parent1: Option<Uuid> = None; // Root entry
        let parent2: Option<Uuid> = Some(entry1_id);
        let parent3: Option<Uuid> = Some(entry2_id);

        // leaf_entry_id tracks current position
        let leaf_entry_id = entry3_id;

        // All IDs are unique and chain is valid
        assert_ne!(entry1_id, entry2_id);
        assert_ne!(entry2_id, entry3_id);
        assert_eq!(parent2, Some(entry1_id));
        assert_eq!(parent3, Some(entry2_id));
        assert_eq!(leaf_entry_id, entry3_id);
    }
}
