//! AgentDispatch client wrapper

use crate::client::auth::AuthInterceptor;
use crate::error::{CoreError, CoreResult};
use crate::generated::flovyn_v1;
use crate::generated::flovyn_v1::agent_dispatch_client::AgentDispatchClient;
use serde_json::Value;
use std::time::Duration;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use uuid::Uuid;

/// Type alias for authenticated client
type AuthClient = AgentDispatchClient<InterceptedService<Channel, AuthInterceptor>>;

/// Client for agent dispatch operations
#[derive(Clone)]
pub struct AgentDispatch {
    inner: AuthClient,
}

/// Info about an agent execution returned from polling
#[derive(Debug, Clone)]
pub struct AgentExecutionInfo {
    /// Agent execution ID
    pub id: Uuid,
    /// Agent kind/type
    pub kind: String,
    /// Organization ID
    pub org_id: Uuid,
    /// Input data
    pub input: Value,
    /// Queue for worker routing
    pub queue: String,
    /// Creation timestamp (ms since epoch)
    pub created_at_ms: i64,
    /// Current checkpoint sequence (-1 if no checkpoint)
    pub current_checkpoint_seq: i32,
    /// Persistence mode (REMOTE or LOCAL)
    pub persistence_mode: String,
    /// Agent definition ID (if created from a definition)
    pub agent_definition_id: Option<Uuid>,
    /// Metadata
    pub metadata: Option<Value>,
    /// Parent execution ID (if this is a child agent)
    pub parent_execution_id: Option<Uuid>,
}

/// Result of a child event poll
#[derive(Debug, Clone)]
pub struct ChildEventResult {
    /// Child execution ID
    pub child_execution_id: Uuid,
    /// Event type: "completed", "failed", "signal"
    pub event_type: String,
    /// Output (if completed)
    pub output: Option<Value>,
    /// Error (if failed)
    pub error: Option<String>,
    /// Signal name (if signal event)
    pub signal_name: Option<String>,
    /// Signal payload (if signal event)
    pub signal_payload: Option<Value>,
}

/// Agent conversation entry
#[derive(Debug, Clone)]
pub struct AgentEntry {
    /// Entry ID
    pub id: Uuid,
    /// Parent entry ID (None for root entries)
    pub parent_id: Option<Uuid>,
    /// Entry type (MESSAGE, TOOL_CALL, TOOL_RESULT)
    pub entry_type: String,
    /// Role (system, user, assistant, tool)
    pub role: Option<String>,
    /// Content (JSON)
    pub content: Value,
    /// Turn ID for grouping entries
    pub turn_id: Option<Uuid>,
    /// Token usage (for LLM entries)
    pub token_usage: Option<TokenUsage>,
    /// Creation timestamp (ms since epoch)
    pub created_at_ms: i64,
}

/// Token usage statistics
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Input tokens consumed
    pub input_tokens: i64,
    /// Output tokens generated
    pub output_tokens: i64,
    /// Cache read tokens
    pub cache_read_tokens: Option<i64>,
    /// Cache write tokens
    pub cache_write_tokens: Option<i64>,
}

/// Agent checkpoint
#[derive(Debug, Clone)]
pub struct AgentCheckpoint {
    /// Checkpoint ID
    pub id: Uuid,
    /// Sequence number
    pub sequence: i32,
    /// Leaf entry ID at checkpoint time
    pub leaf_entry_id: Option<Uuid>,
    /// State (JSON)
    pub state: Value,
    /// Creation timestamp (ms since epoch)
    pub created_at_ms: i64,
}

/// Agent signal
#[derive(Debug, Clone)]
pub struct AgentSignal {
    /// Signal ID
    pub id: Uuid,
    /// Signal name
    pub signal_name: String,
    /// Signal value (JSON)
    pub signal_value: Value,
    /// Creation timestamp (ms since epoch)
    pub created_at_ms: i64,
}

/// Result of scheduling an agent task
#[derive(Debug, Clone)]
pub struct ScheduleTaskResult {
    /// Created task execution ID
    pub task_execution_id: Uuid,
    /// Whether idempotency key was used
    pub idempotency_key_used: bool,
    /// Whether a new task was created
    pub idempotency_key_new: bool,
}

/// Result of appending an entry
#[derive(Debug, Clone)]
pub struct AppendEntryResult {
    /// Entry ID (either newly created or existing if idempotency key matched)
    pub entry_id: Uuid,
    /// True if an existing entry was returned (idempotency key matched)
    pub already_existed: bool,
}

/// Result of signaling an agent
#[derive(Debug, Clone)]
pub struct SignalResult {
    /// Created signal ID
    pub signal_id: Uuid,
    /// Whether agent was resumed from WAITING
    pub agent_resumed: bool,
}

/// Result of querying a task execution
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Task status (PENDING, RUNNING, COMPLETED, FAILED, CANCELLED)
    pub status: String,
    /// Task output (if COMPLETED)
    pub output: Option<Value>,
    /// Error message (if FAILED)
    pub error: Option<String>,
}

impl TaskResult {
    /// Check if the task is completed successfully
    pub fn is_completed(&self) -> bool {
        self.status == "COMPLETED"
    }

    /// Check if the task has failed
    pub fn is_failed(&self) -> bool {
        self.status == "FAILED"
    }

    /// Check if the task is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.status == "CANCELLED"
    }

    /// Check if the task is still running (PENDING or RUNNING)
    pub fn is_running(&self) -> bool {
        self.status == "PENDING" || self.status == "RUNNING"
    }

    /// Check if task is in a terminal state (COMPLETED, FAILED, or CANCELLED)
    pub fn is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "COMPLETED" | "FAILED" | "CANCELLED")
    }
}

/// Wait mode for parallel task waiting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitMode {
    /// Resume when ALL tasks complete (join_all semantics)
    All,
    /// Resume when ANY task completes (select semantics)
    Any,
}

/// Result of a batch task result query
#[derive(Debug, Clone)]
pub struct BatchTaskResult {
    /// Task execution ID
    pub task_execution_id: Uuid,
    /// Task status
    pub status: String,
    /// Task output (if COMPLETED)
    pub output: Option<Value>,
    /// Error message (if FAILED)
    pub error: Option<String>,
}

impl BatchTaskResult {
    /// Check if the task is completed successfully
    pub fn is_completed(&self) -> bool {
        self.status == "COMPLETED"
    }

    /// Check if the task has failed
    pub fn is_failed(&self) -> bool {
        self.status == "FAILED"
    }

    /// Check if the task is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.status == "CANCELLED"
    }

    /// Check if the task is still running (PENDING or RUNNING)
    pub fn is_running(&self) -> bool {
        self.status == "PENDING" || self.status == "RUNNING"
    }

    /// Check if task is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "COMPLETED" | "FAILED" | "CANCELLED")
    }
}

/// Result of a task cancellation attempt
#[derive(Debug, Clone)]
pub struct CancelResult {
    /// Whether cancellation was successful
    pub cancelled: bool,
    /// Final task status after cancellation attempt
    pub status: String,
}

impl CancelResult {
    /// Check if the task was successfully cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Check if the task was already completed before cancel
    pub fn was_already_completed(&self) -> bool {
        !self.cancelled && self.status == "COMPLETED"
    }

    /// Check if the task was already failed before cancel
    pub fn was_already_failed(&self) -> bool {
        !self.cancelled && self.status == "FAILED"
    }
}

/// Input for a task in a batch schedule operation
#[derive(Debug, Clone)]
pub struct BatchScheduleTaskInput {
    /// Task kind
    pub task_kind: String,
    /// Task input (JSON)
    pub input: Value,
    /// Idempotency key (optional)
    pub idempotency_key: Option<String>,
    /// Override queue (optional)
    pub queue: Option<String>,
    /// Override max retries (optional)
    pub max_retries: Option<i32>,
    /// Override timeout in ms (optional)
    pub timeout_ms: Option<i64>,
}

impl BatchScheduleTaskInput {
    /// Create a new batch task input
    pub fn new(task_kind: impl Into<String>, input: Value) -> Self {
        Self {
            task_kind: task_kind.into(),
            input,
            idempotency_key: None,
            queue: None,
            max_retries: None,
            timeout_ms: None,
        }
    }

    /// Set the idempotency key
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Set the queue
    pub fn with_queue(mut self, queue: impl Into<String>) -> Self {
        self.queue = Some(queue.into());
        self
    }

    /// Set max retries
    pub fn with_max_retries(mut self, max_retries: i32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Set timeout
    pub fn with_timeout_ms(mut self, timeout_ms: i64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

/// Result entry for a task in a batch schedule operation
#[derive(Debug, Clone)]
pub struct BatchScheduleTaskResultEntry {
    /// Task execution ID
    pub task_execution_id: Uuid,
    /// Whether idempotency key was used
    pub idempotency_key_used: bool,
    /// Whether a new task was created (false if existing returned)
    pub idempotency_key_new: bool,
    /// Error message if this task failed to schedule
    pub error: Option<String>,
}

impl BatchScheduleTaskResultEntry {
    /// Check if this task was scheduled successfully
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

impl AgentDispatch {
    /// Create from a channel with authentication
    pub fn new(channel: Channel, token: &str) -> Self {
        let interceptor = AuthInterceptor::worker_token(token);
        Self {
            inner: AgentDispatchClient::with_interceptor(channel, interceptor),
        }
    }

    /// Poll for agents to execute
    pub async fn poll_agent(
        &mut self,
        worker_id: &str,
        org_id: &str,
        queue: &str,
        timeout: Duration,
        agent_capabilities: Vec<String>,
    ) -> CoreResult<Option<AgentExecutionInfo>> {
        let request = flovyn_v1::PollAgentRequest {
            worker_id: worker_id.to_string(),
            org_id: org_id.to_string(),
            queue: queue.to_string(),
            timeout_seconds: timeout.as_secs() as i64,
            agent_capabilities,
            worker_pool_id: None,
        };

        let response = self.inner.poll_agent(request).await?;

        let poll_response = response.into_inner();
        Ok(poll_response.agent_execution.map(|ae| AgentExecutionInfo {
            id: ae.id.parse().unwrap_or_default(),
            kind: ae.kind,
            org_id: ae.org_id.parse().unwrap_or_default(),
            input: serde_json::from_slice(&ae.input).unwrap_or(Value::Null),
            queue: ae.queue,
            created_at_ms: ae.created_at_ms,
            current_checkpoint_seq: ae.current_checkpoint_seq,
            persistence_mode: ae.persistence_mode,
            agent_definition_id: ae.agent_definition_id.and_then(|s| s.parse().ok()),
            metadata: if ae.metadata.is_empty() {
                None
            } else {
                serde_json::from_slice(&ae.metadata).ok()
            },
            parent_execution_id: ae.parent_execution_id.and_then(|s| s.parse().ok()),
        }))
    }

    /// Load conversation entries (server-side streaming)
    pub async fn get_entries(
        &mut self,
        agent_execution_id: Uuid,
        after_entry_id: Option<Uuid>,
    ) -> CoreResult<Vec<AgentEntry>> {
        let request = flovyn_v1::GetEntriesRequest {
            agent_execution_id: agent_execution_id.to_string(),
            after_entry_id: after_entry_id.map(|id| id.to_string()),
        };

        let response = self.inner.get_entries(request).await?;
        let mut stream = response.into_inner();

        let mut entries = Vec::new();
        while let Some(chunk) = stream.message().await? {
            if let Some(entry) = chunk.entry {
                entries.push(convert_entry(entry));
            }
        }

        Ok(entries)
    }

    /// Append a conversation entry
    ///
    /// If `idempotency_key` is provided, the server will return an existing entry
    /// if one already exists with the same key, preventing duplicate entries on
    /// agent resume.
    #[allow(clippy::too_many_arguments)]
    pub async fn append_entry(
        &mut self,
        agent_execution_id: Uuid,
        parent_id: Option<Uuid>,
        entry_type: &str,
        role: Option<&str>,
        content: &Value,
        turn_id: Option<Uuid>,
        token_usage: Option<TokenUsage>,
        idempotency_key: Option<&str>,
    ) -> CoreResult<AppendEntryResult> {
        let request = flovyn_v1::AppendEntryRequest {
            agent_execution_id: agent_execution_id.to_string(),
            parent_id: parent_id.map(|id| id.to_string()),
            entry_type: entry_type.to_string(),
            role: role.map(|r| r.to_string()),
            content: serde_json::to_vec(content)?,
            turn_id: turn_id.map(|id| id.to_string()),
            token_usage: token_usage.map(|tu| flovyn_v1::TokenUsage {
                input_tokens: tu.input_tokens,
                output_tokens: tu.output_tokens,
                cache_read_tokens: tu.cache_read_tokens,
                cache_write_tokens: tu.cache_write_tokens,
            }),
            idempotency_key: idempotency_key.map(|k| k.to_string()),
        };

        let response = self.inner.append_entry(request).await?;
        let resp = response.into_inner();
        let entry_id = resp
            .entry_id
            .parse()
            .map_err(|_| CoreError::Other("Invalid entry ID".into()))?;

        Ok(AppendEntryResult {
            entry_id,
            already_existed: resp.already_existed,
        })
    }

    /// Get the latest checkpoint
    pub async fn get_latest_checkpoint(
        &mut self,
        agent_execution_id: Uuid,
    ) -> CoreResult<Option<AgentCheckpoint>> {
        let request = flovyn_v1::GetCheckpointRequest {
            agent_execution_id: agent_execution_id.to_string(),
        };

        let response = self.inner.get_latest_checkpoint(request).await?;
        let resp = response.into_inner();

        Ok(resp.checkpoint.map(|cp| AgentCheckpoint {
            id: cp.id.parse().unwrap_or_default(),
            sequence: cp.sequence,
            leaf_entry_id: cp.leaf_entry_id.and_then(|s| s.parse().ok()),
            state: serde_json::from_slice(&cp.state).unwrap_or(Value::Null),
            created_at_ms: cp.created_at_ms,
        }))
    }

    /// Submit a checkpoint
    pub async fn submit_checkpoint(
        &mut self,
        agent_execution_id: Uuid,
        leaf_entry_id: Option<Uuid>,
        state: &Value,
        cumulative_token_usage: Option<TokenUsage>,
    ) -> CoreResult<(Uuid, i32)> {
        let request = flovyn_v1::SubmitCheckpointRequest {
            agent_execution_id: agent_execution_id.to_string(),
            leaf_entry_id: leaf_entry_id.map(|id| id.to_string()),
            state: serde_json::to_vec(state)?,
            cumulative_token_usage: cumulative_token_usage.map(|tu| flovyn_v1::TokenUsage {
                input_tokens: tu.input_tokens,
                output_tokens: tu.output_tokens,
                cache_read_tokens: tu.cache_read_tokens,
                cache_write_tokens: tu.cache_write_tokens,
            }),
        };

        let response = self.inner.submit_checkpoint(request).await?;
        let resp = response.into_inner();
        let checkpoint_id = resp
            .checkpoint_id
            .parse()
            .map_err(|_| CoreError::Other("Invalid checkpoint ID".into()))?;
        Ok((checkpoint_id, resp.sequence))
    }

    /// Schedule a task for the agent (client-side streaming for large inputs)
    #[allow(clippy::too_many_arguments)]
    pub async fn schedule_task(
        &mut self,
        agent_execution_id: Uuid,
        task_kind: &str,
        input: &Value,
        queue: Option<&str>,
        max_retries: Option<i32>,
        timeout_ms: Option<i64>,
        idempotency_key: Option<&str>,
    ) -> CoreResult<ScheduleTaskResult> {
        // Serialize input
        let input_bytes = serde_json::to_vec(input)?;

        // Create header chunk
        let header = flovyn_v1::ScheduleAgentTaskChunk {
            chunk: Some(flovyn_v1::schedule_agent_task_chunk::Chunk::Header(
                flovyn_v1::ScheduleAgentTaskHeader {
                    agent_execution_id: agent_execution_id.to_string(),
                    task_kind: task_kind.to_string(),
                    queue: queue.map(|q| q.to_string()),
                    max_retries,
                    timeout_ms,
                    idempotency_key: idempotency_key.map(|k| k.to_string()),
                    idempotency_key_ttl_seconds: None,
                    metadata: Default::default(),
                },
            )),
        };

        // Create input chunks (64KB each)
        let chunk_size = 64 * 1024;
        let mut chunks = vec![header];

        for chunk in input_bytes.chunks(chunk_size) {
            chunks.push(flovyn_v1::ScheduleAgentTaskChunk {
                chunk: Some(flovyn_v1::schedule_agent_task_chunk::Chunk::InputChunk(
                    chunk.to_vec(),
                )),
            });
        }

        let stream = tokio_stream::iter(chunks);
        let response = self.inner.schedule_agent_task(stream).await?;
        let resp = response.into_inner();

        Ok(ScheduleTaskResult {
            task_execution_id: resp
                .task_execution_id
                .parse()
                .map_err(|_| CoreError::Other("Invalid task execution ID".into()))?,
            idempotency_key_used: resp.idempotency_key_used,
            idempotency_key_new: resp.idempotency_key_new,
        })
    }

    /// Schedule multiple tasks (fallback implementation using multiple single-task calls)
    ///
    /// Note: This implementation calls schedule_task for each task sequentially.
    /// A more efficient batch RPC could be added in the future.
    pub async fn schedule_tasks_batch(
        &mut self,
        agent_execution_id: Uuid,
        tasks: &[BatchScheduleTaskInput],
        default_queue: Option<&str>,
        default_max_retries: Option<i32>,
        default_timeout_ms: Option<i64>,
    ) -> CoreResult<Vec<BatchScheduleTaskResultEntry>> {
        if tasks.is_empty() {
            return Ok(vec![]);
        }

        let mut results = Vec::with_capacity(tasks.len());

        for task in tasks {
            let queue = task
                .queue
                .as_deref()
                .or(default_queue)
                .map(|s| s.to_string());
            let max_retries = task.max_retries.or(default_max_retries);
            let timeout_ms = task.timeout_ms.or(default_timeout_ms);

            let result = self
                .schedule_task(
                    agent_execution_id,
                    &task.task_kind,
                    &task.input,
                    queue.as_deref(),
                    max_retries,
                    timeout_ms,
                    task.idempotency_key.as_deref(),
                )
                .await;

            match result {
                Ok(r) => {
                    results.push(BatchScheduleTaskResultEntry {
                        task_execution_id: r.task_execution_id,
                        idempotency_key_used: r.idempotency_key_used,
                        idempotency_key_new: r.idempotency_key_new,
                        error: None,
                    });
                }
                Err(e) => {
                    results.push(BatchScheduleTaskResultEntry {
                        task_execution_id: Uuid::nil(),
                        idempotency_key_used: false,
                        idempotency_key_new: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        Ok(results)
    }

    /// Complete the agent with output
    pub async fn complete_agent(
        &mut self,
        agent_execution_id: Uuid,
        output: &Value,
    ) -> CoreResult<()> {
        let request = flovyn_v1::CompleteAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            output: serde_json::to_vec(output)?,
        };

        self.inner.complete_agent(request).await?;
        Ok(())
    }

    /// Fail the agent with an error
    pub async fn fail_agent(
        &mut self,
        agent_execution_id: Uuid,
        error: &str,
        stack_trace: Option<&str>,
        failure_type: Option<&str>,
    ) -> CoreResult<()> {
        let request = flovyn_v1::FailAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            error: error.to_string(),
            stack_trace: stack_trace.map(|s| s.to_string()),
            failure_type: failure_type.map(|s| s.to_string()),
        };

        self.inner.fail_agent(request).await?;
        Ok(())
    }

    /// Suspend the agent waiting for a signal
    pub async fn suspend_agent(
        &mut self,
        agent_execution_id: Uuid,
        wait_for_signal: &str,
        reason: Option<&str>,
    ) -> CoreResult<()> {
        use flovyn_v1::suspend_agent_request::WaitCondition;

        let request = flovyn_v1::SuspendAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            wait_condition: Some(WaitCondition::WaitForSignal(wait_for_signal.to_string())),
            reason: reason.map(|r| r.to_string()),
        };

        self.inner.suspend_agent(request).await?;
        Ok(())
    }

    /// Suspend the agent waiting for a task to complete
    pub async fn suspend_agent_for_task(
        &mut self,
        agent_execution_id: Uuid,
        task_execution_id: Uuid,
        reason: Option<&str>,
    ) -> CoreResult<()> {
        use flovyn_v1::suspend_agent_request::WaitCondition;

        let request = flovyn_v1::SuspendAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            wait_condition: Some(WaitCondition::WaitForTask(task_execution_id.to_string())),
            reason: reason.map(|r| r.to_string()),
        };

        self.inner.suspend_agent(request).await?;
        Ok(())
    }

    /// Get the result of a task execution
    pub async fn get_task_result(
        &mut self,
        agent_execution_id: Uuid,
        task_execution_id: Uuid,
    ) -> CoreResult<TaskResult> {
        let request = flovyn_v1::GetAgentTaskResultRequest {
            agent_execution_id: agent_execution_id.to_string(),
            task_execution_id: task_execution_id.to_string(),
        };

        let response = self.inner.get_agent_task_result(request).await?;
        let resp = response.into_inner();

        Ok(TaskResult {
            status: resp.status,
            output: resp
                .output
                .and_then(|bytes| serde_json::from_slice(&bytes).ok()),
            error: resp.error,
        })
    }

    /// Suspend the agent waiting for multiple tasks to complete
    pub async fn suspend_agent_for_tasks(
        &mut self,
        agent_execution_id: Uuid,
        task_execution_ids: &[Uuid],
        mode: WaitMode,
        reason: Option<&str>,
    ) -> CoreResult<()> {
        use flovyn_v1::suspend_agent_request::WaitCondition;

        let proto_mode = match mode {
            WaitMode::All => flovyn_v1::WaitMode::All,
            WaitMode::Any => flovyn_v1::WaitMode::Any,
        };

        let request = flovyn_v1::SuspendAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            wait_condition: Some(WaitCondition::WaitForTasks(flovyn_v1::WaitForTasks {
                task_ids: task_execution_ids.iter().map(|id| id.to_string()).collect(),
                mode: proto_mode.into(),
            })),
            reason: reason.map(|r| r.to_string()),
        };

        self.inner.suspend_agent(request).await?;
        Ok(())
    }

    /// Get results for multiple tasks in a single call (batch query)
    pub async fn get_task_results_batch(
        &mut self,
        agent_execution_id: Uuid,
        task_execution_ids: &[Uuid],
    ) -> CoreResult<Vec<BatchTaskResult>> {
        let request = flovyn_v1::GetAgentTaskResultsRequest {
            agent_execution_id: agent_execution_id.to_string(),
            task_execution_ids: task_execution_ids.iter().map(|id| id.to_string()).collect(),
        };

        let response = self.inner.get_agent_task_results(request).await?;
        let resp = response.into_inner();

        Ok(resp
            .results
            .into_iter()
            .map(|entry| BatchTaskResult {
                task_execution_id: entry.task_execution_id.parse().unwrap_or_default(),
                status: entry.status,
                output: entry
                    .output
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok()),
                error: entry.error,
            })
            .collect())
    }

    /// Cancel a task
    pub async fn cancel_task(
        &mut self,
        agent_execution_id: Uuid,
        task_execution_id: Uuid,
        reason: Option<&str>,
    ) -> CoreResult<CancelResult> {
        let request = flovyn_v1::CancelAgentTaskRequest {
            agent_execution_id: agent_execution_id.to_string(),
            task_execution_id: task_execution_id.to_string(),
            reason: reason.map(|r| r.to_string()),
        };

        let response = self.inner.cancel_agent_task(request).await?;
        let resp = response.into_inner();

        Ok(CancelResult {
            cancelled: resp.cancelled,
            status: resp.status,
        })
    }

    /// Get the count of tasks scheduled by this agent.
    /// Used to restore the task counter on agent resume.
    pub async fn get_agent_task_count(&mut self, agent_execution_id: Uuid) -> CoreResult<i32> {
        let request = flovyn_v1::GetAgentTaskCountRequest {
            agent_execution_id: agent_execution_id.to_string(),
        };

        let response = self.inner.get_agent_task_count(request).await?;
        let resp = response.into_inner();

        Ok(resp.task_count)
    }

    /// Signal the agent
    pub async fn signal_agent(
        &mut self,
        agent_execution_id: Uuid,
        signal_name: &str,
        signal_value: &Value,
    ) -> CoreResult<SignalResult> {
        let request = flovyn_v1::SignalAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            signal_name: signal_name.to_string(),
            signal_value: serde_json::to_vec(signal_value)?,
        };

        let response = self.inner.signal_agent(request).await?;
        let resp = response.into_inner();

        Ok(SignalResult {
            signal_id: resp
                .signal_id
                .parse()
                .map_err(|_| CoreError::Other("Invalid signal ID".into()))?,
            agent_resumed: resp.agent_resumed,
        })
    }

    /// Consume signals by name
    pub async fn consume_signals(
        &mut self,
        agent_execution_id: Uuid,
        signal_name: Option<&str>,
    ) -> CoreResult<Vec<AgentSignal>> {
        let request = flovyn_v1::ConsumeSignalsRequest {
            agent_execution_id: agent_execution_id.to_string(),
            signal_name: signal_name.map(|s| s.to_string()),
        };

        let response = self.inner.consume_signals(request).await?;
        let resp = response.into_inner();

        Ok(resp
            .signals
            .into_iter()
            .map(|s| AgentSignal {
                id: s.id.parse().unwrap_or_default(),
                signal_name: s.signal_name,
                signal_value: serde_json::from_slice(&s.signal_value).unwrap_or(Value::Null),
                created_at_ms: s.created_at_ms,
            })
            .collect())
    }

    /// Check if a signal exists
    pub async fn has_signal(
        &mut self,
        agent_execution_id: Uuid,
        signal_name: &str,
    ) -> CoreResult<bool> {
        let request = flovyn_v1::HasSignalRequest {
            agent_execution_id: agent_execution_id.to_string(),
            signal_name: signal_name.to_string(),
        };

        let response = self.inner.has_signal(request).await?;
        Ok(response.into_inner().has_signal)
    }

    /// Stream agent data (client-side streaming)
    pub async fn stream_data(
        &mut self,
        agent_execution_id: Uuid,
        sequence: i32,
        event_type: flovyn_v1::AgentStreamEventType,
        payload: &str,
        timestamp_ms: i64,
    ) -> CoreResult<bool> {
        let request = flovyn_v1::StreamAgentDataRequest {
            agent_execution_id: agent_execution_id.to_string(),
            sequence,
            event_type: event_type as i32,
            payload: payload.to_string(),
            timestamp_ms,
        };

        let stream = tokio_stream::iter(vec![request]);
        let response = self.inner.stream_agent_data(stream).await?;
        Ok(response.into_inner().acknowledged)
    }

    // =========================================================================
    // Hierarchical Agent Operations
    // =========================================================================

    /// Spawn a child agent execution
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_child_agent(
        &mut self,
        org_id: Uuid,
        parent_execution_id: Uuid,
        agent_kind: &str,
        input: &[u8],
        queue: Option<&str>,
        mode: &str,
        max_budget_tokens: Option<i64>,
    ) -> CoreResult<Uuid> {
        let request = flovyn_v1::SpawnChildAgentRequest {
            org_id: org_id.to_string(),
            parent_execution_id: parent_execution_id.to_string(),
            agent_kind: agent_kind.to_string(),
            input: input.to_vec(),
            queue: queue.map(|q| q.to_string()),
            mode: mode.to_string(),
            system_prompt: None,
            tools: vec![],
            model: None,
            max_budget_tokens,
        };

        let response = self.inner.spawn_child_agent(request).await?;
        let resp = response.into_inner();
        resp.child_execution_id
            .parse()
            .map_err(|_| CoreError::Other("Invalid child_execution_id in response".to_string()))
    }

    /// Send a signal from child to parent
    pub async fn send_parent_signal(
        &mut self,
        org_id: Uuid,
        child_execution_id: Uuid,
        signal_name: &str,
        payload: &[u8],
    ) -> CoreResult<bool> {
        let request = flovyn_v1::SendParentSignalRequest {
            org_id: org_id.to_string(),
            child_execution_id: child_execution_id.to_string(),
            signal_name: signal_name.to_string(),
            payload: payload.to_vec(),
        };

        let response = self.inner.send_parent_signal(request).await?;
        Ok(response.into_inner().delivered)
    }

    /// Send a signal from parent to child
    pub async fn send_child_signal(
        &mut self,
        org_id: Uuid,
        parent_execution_id: Uuid,
        child_execution_id: Uuid,
        signal_name: &str,
        payload: &[u8],
    ) -> CoreResult<bool> {
        let request = flovyn_v1::SendChildSignalRequest {
            org_id: org_id.to_string(),
            parent_execution_id: parent_execution_id.to_string(),
            child_execution_id: child_execution_id.to_string(),
            signal_name: signal_name.to_string(),
            payload: payload.to_vec(),
        };

        let response = self.inner.send_child_signal(request).await?;
        Ok(response.into_inner().delivered)
    }

    /// Poll for child events (completion, failure, signals)
    pub async fn poll_child_events(
        &mut self,
        org_id: Uuid,
        parent_execution_id: Uuid,
        child_execution_ids: &[Uuid],
    ) -> CoreResult<Vec<ChildEventResult>> {
        let request = flovyn_v1::PollChildEventsRequest {
            org_id: org_id.to_string(),
            parent_execution_id: parent_execution_id.to_string(),
            child_execution_ids: child_execution_ids.iter().map(|id| id.to_string()).collect(),
        };

        let response = self.inner.poll_child_events(request).await?;
        let resp = response.into_inner();

        Ok(resp
            .events
            .into_iter()
            .map(|e| ChildEventResult {
                child_execution_id: e.child_execution_id.parse().unwrap_or_default(),
                event_type: e.event_type,
                output: e.output.and_then(|b| serde_json::from_slice(&b).ok()),
                error: e.error,
                signal_name: e.signal_name,
                signal_payload: e.signal_payload.and_then(|b| serde_json::from_slice(&b).ok()),
            })
            .collect())
    }

    /// Suspend agent waiting for child events (ANY mode)
    pub async fn suspend_agent_for_child_event(
        &mut self,
        agent_execution_id: Uuid,
        child_execution_ids: &[Uuid],
        reason: Option<&str>,
    ) -> CoreResult<()> {
        use flovyn_v1::suspend_agent_request::WaitCondition;

        let request = flovyn_v1::SuspendAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            wait_condition: Some(WaitCondition::WaitForChildEvent(
                flovyn_v1::WaitForChildEvent {
                    child_execution_ids: child_execution_ids
                        .iter()
                        .map(|id| id.to_string())
                        .collect(),
                },
            )),
            reason: reason.map(|r| r.to_string()),
        };

        self.inner.suspend_agent(request).await?;
        Ok(())
    }

    /// Suspend agent waiting for ALL children to complete
    pub async fn suspend_agent_for_all_children(
        &mut self,
        agent_execution_id: Uuid,
        child_execution_ids: &[Uuid],
        reason: Option<&str>,
    ) -> CoreResult<()> {
        use flovyn_v1::suspend_agent_request::WaitCondition;

        let request = flovyn_v1::SuspendAgentRequest {
            agent_execution_id: agent_execution_id.to_string(),
            wait_condition: Some(WaitCondition::WaitForAllChildren(
                flovyn_v1::WaitForAllChildren {
                    child_execution_ids: child_execution_ids
                        .iter()
                        .map(|id| id.to_string())
                        .collect(),
                },
            )),
            reason: reason.map(|r| r.to_string()),
        };

        self.inner.suspend_agent(request).await?;
        Ok(())
    }
}

fn convert_entry(entry: flovyn_v1::AgentEntry) -> AgentEntry {
    AgentEntry {
        id: entry.id.parse().unwrap_or_default(),
        parent_id: entry.parent_id.and_then(|s| s.parse().ok()),
        entry_type: entry.entry_type,
        role: entry.role,
        content: serde_json::from_slice(&entry.content).unwrap_or(Value::Null),
        turn_id: entry.turn_id.and_then(|s| s.parse().ok()),
        token_usage: entry.token_usage.map(|tu| TokenUsage {
            input_tokens: tu.input_tokens,
            output_tokens: tu.output_tokens,
            cache_read_tokens: tu.cache_read_tokens,
            cache_write_tokens: tu.cache_write_tokens,
        }),
        created_at_ms: entry.created_at_ms,
    }
}
