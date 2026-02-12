//! Agent Task Combinators
//!
//! Provides parallel task execution patterns for agents, similar to workflow combinators
//! but using checkpoint-based recovery instead of event-sourced replay.
//!
//! ## Overview
//!
//! Agents can schedule multiple tasks and wait for them using two patterns:
//! - `agent_join_all()` - Wait for ALL tasks to complete (like `tokio::join!`)
//! - `agent_select()` - Wait for ANY task to complete (like `tokio::select!`)
//!
//! ## Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::combinators::{agent_join_all, agent_select};
//!
//! // Schedule multiple tasks without waiting
//! let handle1 = ctx.schedule_task_handle("process-item", json!({"item": "a"})).await?;
//! let handle2 = ctx.schedule_task_handle("process-item", json!({"item": "b"})).await?;
//! let handle3 = ctx.schedule_task_handle("process-item", json!({"item": "c"})).await?;
//!
//! // Wait for all to complete (join_all semantics)
//! let results = agent_join_all(ctx, vec![handle1, handle2, handle3]).await?;
//!
//! // Or wait for first to complete (select semantics)
//! let (winner_index, result) = agent_select(ctx, vec![handle1, handle2]).await?;
//! ```

use crate::agent::context::AgentContext;
use crate::error::{FlovynError, Result};
use serde_json::Value;
use uuid::Uuid;

/// Handle to a scheduled task that hasn't been awaited yet.
///
/// Created by `schedule_task_handle()` on the AgentContext. The task is
/// scheduled on the server immediately, but the agent doesn't wait for it
/// until a combinator function is called.
#[derive(Debug, Clone)]
pub struct AgentTaskHandle {
    /// Task execution ID
    pub(crate) task_id: Uuid,
    /// Task kind (for error messages)
    pub(crate) task_kind: String,
    /// Index for ordering in join_all results
    pub(crate) index: usize,
}

impl AgentTaskHandle {
    /// Create a new task handle
    pub fn new(task_id: Uuid, task_kind: impl Into<String>, index: usize) -> Self {
        Self {
            task_id,
            task_kind: task_kind.into(),
            index,
        }
    }

    /// Get the task execution ID
    pub fn task_id(&self) -> Uuid {
        self.task_id
    }

    /// Get the task kind
    pub fn task_kind(&self) -> &str {
        &self.task_kind
    }

    /// Get the index (for ordering)
    pub fn index(&self) -> usize {
        self.index
    }
}

/// Outcome of a task execution.
#[derive(Debug, Clone)]
pub enum TaskOutcome {
    /// Task completed successfully
    Completed(Value),
    /// Task failed with an error
    Failed(String),
    /// Task was cancelled
    Cancelled,
    /// Task is still pending/running (shouldn't appear in final results)
    Pending,
}

impl TaskOutcome {
    /// Check if the task completed successfully
    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed(_))
    }

    /// Check if the task failed
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// Check if the task was cancelled
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Check if the task is in a terminal state
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending)
    }

    /// Convert to Result, returning error for non-completed outcomes
    pub fn into_result(self) -> Result<Value> {
        match self {
            Self::Completed(v) => Ok(v),
            Self::Failed(e) => Err(FlovynError::TaskFailed(e)),
            Self::Cancelled => Err(FlovynError::TaskFailed("Task was cancelled".to_string())),
            Self::Pending => Err(FlovynError::TaskFailed("Task is still pending".to_string())),
        }
    }
}

/// Wait for all tasks to complete.
///
/// Suspends the agent until ALL tasks have reached a terminal state (completed, failed,
/// or cancelled). If any task fails, returns an error immediately.
///
/// # Arguments
/// * `ctx` - The agent context
/// * `handles` - Task handles from `schedule_task_handle()`
///
/// # Returns
/// A vector of task results in the same order as the input handles.
///
/// # Errors
/// Returns an error if any task fails or is cancelled.
///
/// # Example
/// ```rust,ignore
/// let handle1 = ctx.schedule_task_handle("task-a", input1).await?;
/// let handle2 = ctx.schedule_task_handle("task-b", input2).await?;
/// let results = agent_join_all(ctx, vec![handle1, handle2]).await?;
/// // results[0] is from handle1, results[1] is from handle2
/// ```
pub async fn agent_join_all(
    ctx: &dyn AgentContext,
    handles: Vec<AgentTaskHandle>,
) -> Result<Vec<Value>> {
    if handles.is_empty() {
        return Ok(vec![]);
    }

    let task_ids: Vec<Uuid> = handles.iter().map(|h| h.task_id).collect();

    // Batch fetch all task statuses
    let statuses = ctx.get_task_results_batch(&task_ids).await?;

    let mut results: Vec<Option<Value>> = vec![None; handles.len()];
    let mut all_complete = true;

    for (i, (handle, status)) in handles.iter().zip(statuses.iter()).enumerate() {
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
                    handle.task_kind, handle.task_id, error
                )));
            }
            "CANCELLED" => {
                return Err(FlovynError::TaskFailed(format!(
                    "Task '{}' (id={}) was cancelled",
                    handle.task_kind, handle.task_id
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
    ctx.suspend_for_tasks(&task_ids, flovyn_worker_core::client::WaitMode::All)
        .await?;

    // Agent will be re-executed when all tasks complete
    // On resume, this function will be called again and all tasks should be done
    Err(FlovynError::AgentSuspended(
        "Agent suspended waiting for all tasks to complete".to_string(),
    ))
}

/// Wait for any task to complete.
///
/// Suspends the agent until ANY task reaches a terminal state. Returns the index
/// and result of the first completed task.
///
/// # Arguments
/// * `ctx` - The agent context
/// * `handles` - Task handles from `schedule_task_handle()`
///
/// # Returns
/// A tuple of (index, result) where index is the position in the handles vector.
///
/// # Errors
/// Returns an error if the first completed task failed.
///
/// # Example
/// ```rust,ignore
/// let handle1 = ctx.schedule_task_handle("slow-api", input1).await?;
/// let handle2 = ctx.schedule_task_handle("fallback-api", input2).await?;
/// let (winner_index, result) = agent_select(ctx, vec![handle1, handle2]).await?;
/// if winner_index == 0 {
///     println!("Primary API won");
/// } else {
///     println!("Fallback API won");
/// }
/// ```
pub async fn agent_select(
    ctx: &dyn AgentContext,
    handles: Vec<AgentTaskHandle>,
) -> Result<(usize, Value)> {
    if handles.is_empty() {
        return Err(FlovynError::InvalidArgument(
            "agent_select requires at least one handle".to_string(),
        ));
    }

    let task_ids: Vec<Uuid> = handles.iter().map(|h| h.task_id).collect();

    // Batch fetch all task statuses
    let statuses = ctx.get_task_results_batch(&task_ids).await?;

    // Return first completed or failed
    for (i, (handle, status)) in handles.iter().zip(statuses.iter()).enumerate() {
        match status.status.as_str() {
            "COMPLETED" => {
                return Ok((i, status.output.clone().unwrap_or(Value::Null)));
            }
            "FAILED" => {
                let error = status
                    .error
                    .clone()
                    .unwrap_or_else(|| "Task failed".to_string());
                return Err(FlovynError::TaskFailed(format!(
                    "Task '{}' (id={}) failed: {}",
                    handle.task_kind, handle.task_id, error
                )));
            }
            "CANCELLED" => {
                return Err(FlovynError::TaskFailed(format!(
                    "Task '{}' (id={}) was cancelled",
                    handle.task_kind, handle.task_id
                )));
            }
            _ => {
                // PENDING or RUNNING - continue checking
            }
        }
    }

    // None complete yet - suspend and wait for any task
    ctx.suspend_for_tasks(&task_ids, flovyn_worker_core::client::WaitMode::Any)
        .await?;

    // Agent will be re-executed when any task completes
    Err(FlovynError::AgentSuspended(
        "Agent suspended waiting for any task to complete".to_string(),
    ))
}

/// Wait for all tasks and collect all outcomes (including failures).
///
/// Unlike `agent_join_all`, this function doesn't fail-fast on task failures.
/// It waits for all tasks and returns the outcome of each.
///
/// # Arguments
/// * `ctx` - The agent context
/// * `handles` - Task handles from `schedule_task_handle()`
///
/// # Returns
/// A vector of `TaskOutcome` in the same order as the input handles.
///
/// # Example
/// ```rust,ignore
/// let outcomes = agent_join_all_outcomes(ctx, handles).await?;
/// for (i, outcome) in outcomes.iter().enumerate() {
///     match outcome {
///         TaskOutcome::Completed(v) => println!("Task {} succeeded: {:?}", i, v),
///         TaskOutcome::Failed(e) => println!("Task {} failed: {}", i, e),
///         TaskOutcome::Cancelled => println!("Task {} was cancelled", i),
///         TaskOutcome::Pending => println!("Task {} still running", i),
///     }
/// }
/// ```
pub async fn agent_join_all_outcomes(
    ctx: &dyn AgentContext,
    handles: Vec<AgentTaskHandle>,
) -> Result<Vec<TaskOutcome>> {
    if handles.is_empty() {
        return Ok(vec![]);
    }

    let task_ids: Vec<Uuid> = handles.iter().map(|h| h.task_id).collect();

    // Batch fetch all task statuses
    let statuses = ctx.get_task_results_batch(&task_ids).await?;

    let mut all_terminal = true;
    let mut outcomes: Vec<TaskOutcome> = Vec::with_capacity(handles.len());

    for status in statuses.iter() {
        let outcome = match status.status.as_str() {
            "COMPLETED" => TaskOutcome::Completed(status.output.clone().unwrap_or(Value::Null)),
            "FAILED" => TaskOutcome::Failed(
                status
                    .error
                    .clone()
                    .unwrap_or_else(|| "Task failed".to_string()),
            ),
            "CANCELLED" => TaskOutcome::Cancelled,
            _ => {
                all_terminal = false;
                TaskOutcome::Pending
            }
        };
        outcomes.push(outcome);
    }

    if all_terminal {
        return Ok(outcomes);
    }

    // Not all done - suspend and wait
    ctx.suspend_for_tasks(&task_ids, flovyn_worker_core::client::WaitMode::All)
        .await?;

    Err(FlovynError::AgentSuspended(
        "Agent suspended waiting for all tasks".to_string(),
    ))
}

/// Result from `agent_select_with_cancel()`.
#[derive(Debug, Clone)]
pub struct SelectWithCancelResult {
    /// Index of the winning task in the original handles vector
    pub winner_index: usize,
    /// Result from the winning task
    pub result: Value,
    /// Results of cancellation attempts for non-winning tasks
    pub cancel_results: Vec<CancelAttempt>,
}

/// Result of an individual cancel attempt.
#[derive(Debug, Clone)]
pub struct CancelAttempt {
    /// Index of the task in the original handles vector
    pub index: usize,
    /// Task execution ID
    pub task_id: Uuid,
    /// Whether the cancellation was successful
    pub cancelled: bool,
    /// Reason cancellation wasn't possible (e.g., task already completed)
    pub reason: Option<String>,
}

/// Wait for any task to complete, then cancel remaining tasks.
///
/// Similar to `agent_select`, but automatically cancels all non-winning tasks
/// after the first task completes. This is useful for race scenarios where
/// you only need one result and want to clean up the others.
///
/// # Arguments
/// * `ctx` - The agent context
/// * `handles` - Task handles from `schedule_task_handle()`
/// * `cancel_reason` - Optional reason for cancellation
///
/// # Returns
/// A `SelectWithCancelResult` containing the winner index, result, and
/// cancellation results for remaining tasks.
///
/// # Errors
/// Returns an error if the first completed task failed.
///
/// # Example
/// ```rust,ignore
/// let handle1 = ctx.schedule_task_handle("primary-api", input1).await?;
/// let handle2 = ctx.schedule_task_handle("backup-api", input2).await?;
/// let handle3 = ctx.schedule_task_handle("fallback-api", input3).await?;
///
/// let result = agent_select_with_cancel(
///     ctx,
///     vec![handle1, handle2, handle3],
///     Some("Race completed")
/// ).await?;
///
/// println!("Winner: task {} with result {:?}", result.winner_index, result.result);
/// for attempt in result.cancel_results {
///     if attempt.cancelled {
///         println!("Cancelled task {}", attempt.index);
///     }
/// }
/// ```
pub async fn agent_select_with_cancel(
    ctx: &dyn AgentContext,
    handles: Vec<AgentTaskHandle>,
    cancel_reason: Option<&str>,
) -> Result<SelectWithCancelResult> {
    if handles.is_empty() {
        return Err(FlovynError::InvalidArgument(
            "agent_select_with_cancel requires at least one handle".to_string(),
        ));
    }

    let task_ids: Vec<Uuid> = handles.iter().map(|h| h.task_id).collect();

    // Batch fetch all task statuses
    let statuses = ctx.get_task_results_batch(&task_ids).await?;

    // Find first completed or failed task
    let mut winner: Option<(usize, Value)> = None;
    for (i, (handle, status)) in handles.iter().zip(statuses.iter()).enumerate() {
        match status.status.as_str() {
            "COMPLETED" => {
                winner = Some((i, status.output.clone().unwrap_or(Value::Null)));
                break;
            }
            "FAILED" => {
                let error = status
                    .error
                    .clone()
                    .unwrap_or_else(|| "Task failed".to_string());
                return Err(FlovynError::TaskFailed(format!(
                    "Task '{}' (id={}) failed: {}",
                    handle.task_kind, handle.task_id, error
                )));
            }
            "CANCELLED" => {
                return Err(FlovynError::TaskFailed(format!(
                    "Task '{}' (id={}) was cancelled",
                    handle.task_kind, handle.task_id
                )));
            }
            _ => {
                // PENDING or RUNNING - continue checking
            }
        }
    }

    // If we have a winner, cancel remaining tasks
    if let Some((winner_index, result)) = winner {
        // Collect non-winning handles for cancellation
        let to_cancel: Vec<&AgentTaskHandle> = handles
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != winner_index)
            .map(|(_, h)| h)
            .collect();

        // Cancel all non-winning tasks
        let cancel_results = ctx
            .cancel_tasks(
                &to_cancel.iter().cloned().cloned().collect::<Vec<_>>(),
                cancel_reason,
            )
            .await?;

        // Build cancel attempt results
        let mut cancel_attempts = Vec::with_capacity(to_cancel.len());
        for (handle, cancel_result) in to_cancel.iter().zip(cancel_results.iter()) {
            cancel_attempts.push(CancelAttempt {
                index: handle.index,
                task_id: handle.task_id,
                cancelled: cancel_result.cancelled,
                reason: if cancel_result.cancelled {
                    None
                } else {
                    // Task was in a terminal state, provide status as reason
                    Some(format!("Task already in state: {}", cancel_result.status))
                },
            });
        }

        return Ok(SelectWithCancelResult {
            winner_index,
            result,
            cancel_results: cancel_attempts,
        });
    }

    // None complete yet - suspend and wait for any task
    ctx.suspend_for_tasks(&task_ids, flovyn_worker_core::client::WaitMode::Any)
        .await?;

    // Agent will be re-executed when any task completes
    Err(FlovynError::AgentSuspended(
        "Agent suspended waiting for any task to complete".to_string(),
    ))
}

/// Result from `agent_join_all_with_timeout()`.
#[derive(Debug, Clone)]
pub struct JoinAllTimeoutResult {
    /// Results from tasks that completed successfully (index, value)
    pub completed: Vec<(usize, Value)>,
    /// Tasks that failed (index, error message)
    pub failed: Vec<(usize, String)>,
    /// Tasks that were cancelled due to timeout (index, task_id)
    pub cancelled: Vec<(usize, Uuid)>,
    /// Whether the operation timed out
    pub timed_out: bool,
}

impl JoinAllTimeoutResult {
    /// Check if all tasks completed successfully (no failures, no cancellations)
    pub fn all_succeeded(&self) -> bool {
        self.failed.is_empty() && self.cancelled.is_empty() && !self.timed_out
    }

    /// Get all results in order, returning None for failed/cancelled tasks
    pub fn results_in_order(&self, total_count: usize) -> Vec<Option<Value>> {
        let mut results = vec![None; total_count];
        for (idx, value) in &self.completed {
            if *idx < total_count {
                results[*idx] = Some(value.clone());
            }
        }
        results
    }

    /// Convert to a Vec<Value>, returning error if any task failed or was cancelled
    pub fn into_results(self) -> Result<Vec<Value>> {
        if !self.failed.is_empty() {
            let (idx, err) = &self.failed[0];
            return Err(FlovynError::TaskFailed(format!(
                "Task at index {} failed: {}",
                idx, err
            )));
        }
        if !self.cancelled.is_empty() {
            return Err(FlovynError::TaskFailed(format!(
                "Task at index {} was cancelled due to timeout",
                self.cancelled[0].0
            )));
        }
        // Sort by index and extract values
        let mut sorted: Vec<_> = self.completed;
        sorted.sort_by_key(|(idx, _)| *idx);
        Ok(sorted.into_iter().map(|(_, v)| v).collect())
    }
}

/// Wait for all tasks to complete with a timeout.
///
/// If all tasks complete before the timeout, returns all results.
/// If the timeout is exceeded, cancels remaining pending tasks and returns
/// partial results with information about which tasks completed, failed, or were cancelled.
///
/// # Arguments
/// * `ctx` - The agent context
/// * `handles` - Task handles from `schedule_task_handle()`
/// * `timeout` - Maximum time to wait for all tasks
///
/// # Returns
/// A `JoinAllTimeoutResult` containing completed results, failures, and cancellations.
///
/// # Note
/// This function does NOT suspend the agent - it relies on the caller to manage
/// checkpoint/suspend if needed. For durable timeout behavior, use `agent_join_all`
/// with external timeout tracking in agent state.
///
/// # Example
/// ```rust,ignore
/// let handle1 = ctx.schedule_task_handle("fast-task", input1).await?;
/// let handle2 = ctx.schedule_task_handle("slow-task", input2).await?;
/// let handle3 = ctx.schedule_task_handle("medium-task", input3).await?;
///
/// let result = agent_join_all_with_timeout(
///     ctx,
///     vec![handle1, handle2, handle3],
///     Duration::from_secs(5),
///     Some("Timeout exceeded")
/// ).await?;
///
/// if result.all_succeeded() {
///     let values = result.into_results()?;
///     println!("All {} tasks completed", values.len());
/// } else {
///     println!("Completed: {}, Failed: {}, Cancelled: {}",
///         result.completed.len(),
///         result.failed.len(),
///         result.cancelled.len()
///     );
/// }
/// ```
pub async fn agent_join_all_with_timeout(
    ctx: &dyn AgentContext,
    handles: Vec<AgentTaskHandle>,
    timeout: std::time::Duration,
    cancel_reason: Option<&str>,
) -> Result<JoinAllTimeoutResult> {
    use std::time::Instant;

    if handles.is_empty() {
        return Ok(JoinAllTimeoutResult {
            completed: vec![],
            failed: vec![],
            cancelled: vec![],
            timed_out: false,
        });
    }

    let start = Instant::now();
    let task_ids: Vec<Uuid> = handles.iter().map(|h| h.task_id).collect();

    // Poll until all complete or timeout
    loop {
        // Check if we've exceeded timeout
        if start.elapsed() >= timeout {
            // Timeout - cancel remaining pending tasks
            let statuses = ctx.get_task_results_batch(&task_ids).await?;

            let mut completed = Vec::new();
            let mut failed = Vec::new();
            let mut to_cancel = Vec::new();

            for (i, (handle, status)) in handles.iter().zip(statuses.iter()).enumerate() {
                match status.status.as_str() {
                    "COMPLETED" => {
                        completed.push((i, status.output.clone().unwrap_or(Value::Null)));
                    }
                    "FAILED" => {
                        failed.push((
                            i,
                            status
                                .error
                                .clone()
                                .unwrap_or_else(|| "Task failed".to_string()),
                        ));
                    }
                    "CANCELLED" => {
                        // Already cancelled, don't try to cancel again
                    }
                    _ => {
                        // PENDING or RUNNING - needs cancellation
                        to_cancel.push(handle.clone());
                    }
                }
            }

            // Cancel pending tasks
            let mut cancelled = Vec::new();
            if !to_cancel.is_empty() {
                let cancel_results = ctx.cancel_tasks(&to_cancel, cancel_reason).await?;
                for (handle, _result) in to_cancel.iter().zip(cancel_results.iter()) {
                    cancelled.push((handle.index, handle.task_id));
                }
            }

            return Ok(JoinAllTimeoutResult {
                completed,
                failed,
                cancelled,
                timed_out: true,
            });
        }

        // Batch fetch all task statuses
        let statuses = ctx.get_task_results_batch(&task_ids).await?;

        let mut completed = Vec::new();
        let mut failed = Vec::new();
        let mut all_terminal = true;

        for (i, status) in statuses.iter().enumerate() {
            match status.status.as_str() {
                "COMPLETED" => {
                    completed.push((i, status.output.clone().unwrap_or(Value::Null)));
                }
                "FAILED" => {
                    failed.push((
                        i,
                        status
                            .error
                            .clone()
                            .unwrap_or_else(|| "Task failed".to_string()),
                    ));
                }
                "CANCELLED" => {
                    // Task was cancelled externally
                }
                _ => {
                    // PENDING or RUNNING
                    all_terminal = false;
                }
            }
        }

        if all_terminal {
            return Ok(JoinAllTimeoutResult {
                completed,
                failed,
                cancelled: vec![],
                timed_out: false,
            });
        }

        // Not all done yet - sleep briefly before next poll
        // This is a simple polling approach; for production use, consider
        // using server-side wait with timeout support
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_task_handle() {
        let handle = AgentTaskHandle::new(Uuid::new_v4(), "test-task", 0);
        assert_eq!(handle.task_kind(), "test-task");
        assert_eq!(handle.index(), 0);
    }

    #[test]
    fn test_task_outcome() {
        let completed = TaskOutcome::Completed(serde_json::json!({"result": 42}));
        assert!(completed.is_completed());
        assert!(completed.is_terminal());

        let failed = TaskOutcome::Failed("error".to_string());
        assert!(failed.is_failed());
        assert!(failed.is_terminal());

        let cancelled = TaskOutcome::Cancelled;
        assert!(cancelled.is_cancelled());
        assert!(cancelled.is_terminal());

        let pending = TaskOutcome::Pending;
        assert!(!pending.is_terminal());
    }
}
