//! ReplayEngine - Language-agnostic replay logic for workflow execution.
//!
//! This module provides a shared replay engine that handles event pre-filtering,
//! sequence management, and terminal event lookup. It's used by both the Rust SDK
//! and the FFI layer to ensure consistent replay behavior across all language SDKs.
//!
//! ## How it works
//!
//! 1. Engine is created with replay events from the server
//! 2. Events are pre-filtered by type for O(1) lookup
//! 3. Per-type sequence counters track replay progress
//! 4. Terminal events are looked up lazily when needed
//!
//! ## Usage
//!
//! ```ignore
//! let engine = ReplayEngine::new(events);
//!
//! // Get next sequence and check if replaying
//! let seq = engine.next_task_seq();
//! if let Some(event) = engine.get_task_event(seq) {
//!     // Replaying: validate determinism and look up terminal event
//!     let task_id = event.get_string("taskExecutionId").unwrap();
//!     if let Some(terminal) = engine.find_terminal_task_event(&task_id) {
//!         // Task completed or failed
//!     }
//! } else {
//!     // New: generate command
//! }
//! ```

use crate::workflow::event::{EventType, ReplayEvent};
use crate::workflow::execution::EventLookup;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// Language-agnostic replay engine for workflow execution.
///
/// Handles event pre-filtering, sequence management, and terminal event lookup.
/// Used by both Rust SDK and FFI layer.
pub struct ReplayEngine {
    // Pre-filtered event lists by type
    task_events: Vec<ReplayEvent>,
    timer_events: Vec<ReplayEvent>,
    promise_events: Vec<ReplayEvent>,
    child_workflow_events: Vec<ReplayEvent>,
    operation_events: Vec<ReplayEvent>,
    state_events: Vec<ReplayEvent>,
    signal_events: Vec<ReplayEvent>,

    // All events for terminal event lookup
    all_events: Vec<ReplayEvent>,

    // Per-type sequence counters (atomic for thread-safety)
    next_task_seq: AtomicU32,
    next_timer_seq: AtomicU32,
    next_promise_seq: AtomicU32,
    next_child_workflow_seq: AtomicU32,
    next_operation_seq: AtomicU32,
    next_state_seq: AtomicU32,
    next_signal_seq: AtomicU32,

    // Caches built from events
    operation_cache: HashMap<String, Value>,
    state: RwLock<HashMap<String, Value>>,
}

impl ReplayEngine {
    /// Create a new ReplayEngine from replay events.
    ///
    /// This pre-filters events by type for O(1) lookup during replay,
    /// and builds operation cache and initial state.
    pub fn new(events: Vec<ReplayEvent>) -> Self {
        // Pre-filter events by type
        let task_events: Vec<ReplayEvent> = events
            .iter()
            .filter(|e| e.event_type() == EventType::TaskScheduled)
            .cloned()
            .collect();

        let timer_events: Vec<ReplayEvent> = events
            .iter()
            .filter(|e| e.event_type() == EventType::TimerStarted)
            .cloned()
            .collect();

        let promise_events: Vec<ReplayEvent> = events
            .iter()
            .filter(|e| e.event_type() == EventType::PromiseCreated)
            .cloned()
            .collect();

        let child_workflow_events: Vec<ReplayEvent> = events
            .iter()
            .filter(|e| e.event_type() == EventType::ChildWorkflowInitiated)
            .cloned()
            .collect();

        let operation_events: Vec<ReplayEvent> = events
            .iter()
            .filter(|e| e.event_type() == EventType::OperationCompleted)
            .cloned()
            .collect();

        let state_events: Vec<ReplayEvent> = events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type(),
                    EventType::StateSet | EventType::StateCleared
                )
            })
            .cloned()
            .collect();

        let signal_events: Vec<ReplayEvent> = events
            .iter()
            .filter(|e| e.event_type() == EventType::SignalReceived)
            .cloned()
            .collect();

        // Build operation cache from OperationCompleted events
        let operation_cache = crate::workflow::execution::build_operation_cache(&events);

        // Build initial state from StateSet/StateCleared events
        let state = crate::workflow::execution::build_initial_state(&events);

        Self {
            task_events,
            timer_events,
            promise_events,
            child_workflow_events,
            operation_events,
            state_events,
            signal_events,
            all_events: events,
            next_task_seq: AtomicU32::new(0),
            next_timer_seq: AtomicU32::new(0),
            next_promise_seq: AtomicU32::new(0),
            next_child_workflow_seq: AtomicU32::new(0),
            next_operation_seq: AtomicU32::new(0),
            next_state_seq: AtomicU32::new(0),
            next_signal_seq: AtomicU32::new(0),
            operation_cache,
            state: RwLock::new(state),
        }
    }

    // =========================================================================
    // Sequence Management
    // =========================================================================

    /// Get next task sequence number and increment.
    pub fn next_task_seq(&self) -> u32 {
        self.next_task_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Get next timer sequence number and increment.
    pub fn next_timer_seq(&self) -> u32 {
        self.next_timer_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Get next promise sequence number and increment.
    pub fn next_promise_seq(&self) -> u32 {
        self.next_promise_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Get next child workflow sequence number and increment.
    pub fn next_child_workflow_seq(&self) -> u32 {
        self.next_child_workflow_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Get next operation sequence number and increment.
    pub fn next_operation_seq(&self) -> u32 {
        self.next_operation_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Get next state sequence number and increment.
    pub fn next_state_seq(&self) -> u32 {
        self.next_state_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Get next signal sequence number and increment.
    pub fn next_signal_seq(&self) -> u32 {
        self.next_signal_seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Peek at the current signal sequence without incrementing.
    pub fn peek_signal_seq(&self) -> u32 {
        self.next_signal_seq.load(Ordering::SeqCst)
    }

    // =========================================================================
    // Event Lookup (for replay validation)
    // =========================================================================

    /// Get the task event at the given sequence index (if replaying).
    pub fn get_task_event(&self, seq: u32) -> Option<&ReplayEvent> {
        self.task_events.get(seq as usize)
    }

    /// Get the timer event at the given sequence index (if replaying).
    pub fn get_timer_event(&self, seq: u32) -> Option<&ReplayEvent> {
        self.timer_events.get(seq as usize)
    }

    /// Get the promise event at the given sequence index (if replaying).
    pub fn get_promise_event(&self, seq: u32) -> Option<&ReplayEvent> {
        self.promise_events.get(seq as usize)
    }

    /// Get the child workflow event at the given sequence index (if replaying).
    pub fn get_child_workflow_event(&self, seq: u32) -> Option<&ReplayEvent> {
        self.child_workflow_events.get(seq as usize)
    }

    /// Get the operation event at the given sequence index (if replaying).
    pub fn get_operation_event(&self, seq: u32) -> Option<&ReplayEvent> {
        self.operation_events.get(seq as usize)
    }

    /// Get the state event at the given sequence index (if replaying).
    pub fn get_state_event(&self, seq: u32) -> Option<&ReplayEvent> {
        self.state_events.get(seq as usize)
    }

    /// Get the signal event at the given sequence index (if replaying).
    pub fn get_signal_event(&self, seq: u32) -> Option<&ReplayEvent> {
        self.signal_events.get(seq as usize)
    }

    /// Check if currently replaying for tasks (seq < task_event_count).
    pub fn is_replaying_task(&self, seq: u32) -> bool {
        (seq as usize) < self.task_events.len()
    }

    /// Check if currently replaying for timers.
    pub fn is_replaying_timer(&self, seq: u32) -> bool {
        (seq as usize) < self.timer_events.len()
    }

    /// Check if currently replaying for promises.
    pub fn is_replaying_promise(&self, seq: u32) -> bool {
        (seq as usize) < self.promise_events.len()
    }

    /// Check if currently replaying for child workflows.
    pub fn is_replaying_child_workflow(&self, seq: u32) -> bool {
        (seq as usize) < self.child_workflow_events.len()
    }

    /// Check if currently replaying for operations.
    pub fn is_replaying_operation(&self, seq: u32) -> bool {
        (seq as usize) < self.operation_events.len()
    }

    /// Check if there are pending signals.
    pub fn has_pending_signal(&self) -> bool {
        (self.next_signal_seq.load(Ordering::SeqCst) as usize) < self.signal_events.len()
    }

    /// Get the number of pending signals (total signals - consumed signals).
    pub fn pending_signal_count(&self) -> usize {
        let current = self.next_signal_seq.load(Ordering::SeqCst) as usize;
        if current < self.signal_events.len() {
            self.signal_events.len() - current
        } else {
            0
        }
    }

    // =========================================================================
    // Terminal Event Lookup
    // =========================================================================

    /// Find terminal event (TaskCompleted or TaskFailed) for a task by execution ID.
    pub fn find_terminal_task_event(&self, task_execution_id: &str) -> Option<&ReplayEvent> {
        EventLookup::find_terminal_task_event(&self.all_events, task_execution_id)
    }

    /// Find terminal event (TimerFired or TimerCancelled) for a timer by ID.
    pub fn find_terminal_timer_event(&self, timer_id: &str) -> Option<&ReplayEvent> {
        EventLookup::find_terminal_timer_event(&self.all_events, timer_id)
    }

    /// Find terminal event (PromiseResolved, PromiseRejected, or PromiseTimeout) for a promise.
    pub fn find_terminal_promise_event(&self, promise_id: &str) -> Option<&ReplayEvent> {
        EventLookup::find_terminal_promise_event(&self.all_events, promise_id)
    }

    /// Find terminal event (ChildWorkflowCompleted or ChildWorkflowFailed) for a child workflow.
    pub fn find_terminal_child_workflow_event(&self, name: &str) -> Option<&ReplayEvent> {
        EventLookup::find_terminal_child_workflow_event(&self.all_events, name)
    }

    // =========================================================================
    // State Management
    // =========================================================================

    /// Get a value from workflow state.
    pub fn get_state(&self, key: &str) -> Option<Value> {
        self.state.read().get(key).cloned()
    }

    /// Set a value in workflow state.
    pub fn set_state(&self, key: &str, value: Value) {
        self.state.write().insert(key.to_string(), value);
    }

    /// Clear a specific key from workflow state.
    pub fn clear_state(&self, key: &str) {
        self.state.write().remove(key);
    }

    /// Clear all workflow state.
    pub fn clear_all_state(&self) {
        self.state.write().clear();
    }

    /// Get all keys in workflow state.
    pub fn state_keys(&self) -> Vec<String> {
        self.state.read().keys().cloned().collect()
    }

    // =========================================================================
    // Operation Cache
    // =========================================================================

    /// Get a cached operation result by name.
    pub fn get_cached_operation(&self, name: &str) -> Option<&Value> {
        self.operation_cache.get(name)
    }

    // =========================================================================
    // Accessors
    // =========================================================================

    /// Get the number of task events.
    pub fn task_event_count(&self) -> usize {
        self.task_events.len()
    }

    /// Get the number of timer events.
    pub fn timer_event_count(&self) -> usize {
        self.timer_events.len()
    }

    /// Get the number of promise events.
    pub fn promise_event_count(&self) -> usize {
        self.promise_events.len()
    }

    /// Get the number of child workflow events.
    pub fn child_workflow_event_count(&self) -> usize {
        self.child_workflow_events.len()
    }

    /// Get the number of operation events.
    pub fn operation_event_count(&self) -> usize {
        self.operation_events.len()
    }

    /// Get the number of state events.
    pub fn state_event_count(&self) -> usize {
        self.state_events.len()
    }

    /// Get the number of signal events.
    pub fn signal_event_count(&self) -> usize {
        self.signal_events.len()
    }

    /// Get the total number of events.
    pub fn total_event_count(&self) -> usize {
        self.all_events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    // =========================================================================
    // Event Filtering Tests
    // =========================================================================

    #[test]
    fn test_new_filters_task_events() {
        let events = vec![
            ReplayEvent::new(1, EventType::TaskScheduled, json!({"kind": "a"}), now()),
            ReplayEvent::new(2, EventType::TimerStarted, json!({"timerId": "t1"}), now()),
            ReplayEvent::new(3, EventType::TaskScheduled, json!({"kind": "b"}), now()),
            ReplayEvent::new(
                4,
                EventType::PromiseCreated,
                json!({"promiseId": "p1"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.task_event_count(), 2);
        assert_eq!(
            engine.get_task_event(0).unwrap().get_string("kind"),
            Some("a")
        );
        assert_eq!(
            engine.get_task_event(1).unwrap().get_string("kind"),
            Some("b")
        );
    }

    #[test]
    fn test_new_filters_timer_events() {
        let events = vec![
            ReplayEvent::new(1, EventType::TimerStarted, json!({"timerId": "t1"}), now()),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({"kind": "a"}), now()),
            ReplayEvent::new(3, EventType::TimerStarted, json!({"timerId": "t2"}), now()),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.timer_event_count(), 2);
        assert_eq!(
            engine.get_timer_event(0).unwrap().get_string("timerId"),
            Some("t1")
        );
        assert_eq!(
            engine.get_timer_event(1).unwrap().get_string("timerId"),
            Some("t2")
        );
    }

    #[test]
    fn test_new_filters_promise_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                json!({"promiseId": "p1"}),
                now(),
            ),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({}), now()),
            ReplayEvent::new(
                3,
                EventType::PromiseCreated,
                json!({"promiseId": "p2"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.promise_event_count(), 2);
    }

    #[test]
    fn test_new_filters_child_workflow_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                json!({"childExecutionName": "child1"}),
                now(),
            ),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({}), now()),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.child_workflow_event_count(), 1);
    }

    #[test]
    fn test_new_filters_operation_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                json!({"operationName": "op1", "result": 42}),
                now(),
            ),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({}), now()),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.operation_event_count(), 1);
    }

    #[test]
    fn test_new_filters_state_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::StateSet,
                json!({"key": "a", "value": 1}),
                now(),
            ),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({}), now()),
            ReplayEvent::new(3, EventType::StateCleared, json!({"key": "a"}), now()),
            ReplayEvent::new(
                4,
                EventType::StateSet,
                json!({"key": "b", "value": 2}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.state_event_count(), 3);
    }

    #[test]
    fn test_new_preserves_event_order() {
        let events = vec![
            ReplayEvent::new(1, EventType::TaskScheduled, json!({"order": 1}), now()),
            ReplayEvent::new(5, EventType::TaskScheduled, json!({"order": 2}), now()),
            ReplayEvent::new(10, EventType::TaskScheduled, json!({"order": 3}), now()),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(
            engine.get_task_event(0).unwrap().get("order"),
            Some(&json!(1))
        );
        assert_eq!(
            engine.get_task_event(1).unwrap().get("order"),
            Some(&json!(2))
        );
        assert_eq!(
            engine.get_task_event(2).unwrap().get("order"),
            Some(&json!(3))
        );
    }

    #[test]
    fn test_new_with_empty_events() {
        let engine = ReplayEngine::new(vec![]);

        assert_eq!(engine.task_event_count(), 0);
        assert_eq!(engine.timer_event_count(), 0);
        assert_eq!(engine.promise_event_count(), 0);
        assert_eq!(engine.total_event_count(), 0);
    }

    // =========================================================================
    // Sequence Management Tests
    // =========================================================================

    #[test]
    fn test_next_task_seq_increments() {
        let engine = ReplayEngine::new(vec![]);

        assert_eq!(engine.next_task_seq(), 0);
        assert_eq!(engine.next_task_seq(), 1);
        assert_eq!(engine.next_task_seq(), 2);
    }

    #[test]
    fn test_next_timer_seq_increments() {
        let engine = ReplayEngine::new(vec![]);

        assert_eq!(engine.next_timer_seq(), 0);
        assert_eq!(engine.next_timer_seq(), 1);
    }

    #[test]
    fn test_next_promise_seq_increments() {
        let engine = ReplayEngine::new(vec![]);

        assert_eq!(engine.next_promise_seq(), 0);
        assert_eq!(engine.next_promise_seq(), 1);
    }

    #[test]
    fn test_sequence_counters_independent() {
        let engine = ReplayEngine::new(vec![]);

        assert_eq!(engine.next_task_seq(), 0);
        assert_eq!(engine.next_timer_seq(), 0);
        assert_eq!(engine.next_promise_seq(), 0);
        assert_eq!(engine.next_task_seq(), 1);
        assert_eq!(engine.next_timer_seq(), 1);
    }

    #[test]
    fn test_sequence_counters_thread_safe() {
        use std::sync::Arc;
        use std::thread;

        let engine = Arc::new(ReplayEngine::new(vec![]));
        let mut handles = vec![];

        for _ in 0..10 {
            let e = Arc::clone(&engine);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    e.next_task_seq();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // After 10 threads each incrementing 100 times
        assert_eq!(engine.next_task_seq(), 1000);
    }

    // =========================================================================
    // Event Lookup Tests
    // =========================================================================

    #[test]
    fn test_get_task_event_returns_correct_event() {
        let events = vec![
            ReplayEvent::new(1, EventType::TaskScheduled, json!({"id": "first"}), now()),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({"id": "second"}), now()),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(
            engine.get_task_event(0).unwrap().get_string("id"),
            Some("first")
        );
        assert_eq!(
            engine.get_task_event(1).unwrap().get_string("id"),
            Some("second")
        );
    }

    #[test]
    fn test_get_task_event_returns_none_beyond_count() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::TaskScheduled,
            json!({}),
            now(),
        )];

        let engine = ReplayEngine::new(events);

        assert!(engine.get_task_event(0).is_some());
        assert!(engine.get_task_event(1).is_none());
        assert!(engine.get_task_event(100).is_none());
    }

    #[test]
    fn test_is_replaying_task_true_when_seq_in_range() {
        let events = vec![
            ReplayEvent::new(1, EventType::TaskScheduled, json!({}), now()),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({}), now()),
        ];

        let engine = ReplayEngine::new(events);

        assert!(engine.is_replaying_task(0));
        assert!(engine.is_replaying_task(1));
    }

    #[test]
    fn test_is_replaying_task_false_when_seq_beyond() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::TaskScheduled,
            json!({}),
            now(),
        )];

        let engine = ReplayEngine::new(events);

        assert!(!engine.is_replaying_task(1));
        assert!(!engine.is_replaying_task(100));
    }

    // =========================================================================
    // Terminal Event Lookup Tests
    // =========================================================================

    #[test]
    fn test_find_terminal_task_event_completed() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                json!({"taskExecutionId": "task-1"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                json!({"taskExecutionId": "task-1", "result": 42}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_task_event("task-1");

        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::TaskCompleted);
    }

    #[test]
    fn test_find_terminal_task_event_failed() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                json!({"taskExecutionId": "task-1"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskFailed,
                json!({"taskExecutionId": "task-1", "error": "oops"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_task_event("task-1");

        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::TaskFailed);
    }

    #[test]
    fn test_find_terminal_task_event_latest() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                json!({"taskExecutionId": "task-1"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskFailed,
                json!({"taskExecutionId": "task-1", "error": "first"}),
                now(),
            ),
            ReplayEvent::new(
                3,
                EventType::TaskCompleted,
                json!({"taskExecutionId": "task-1", "result": "retry succeeded"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_task_event("task-1");

        // Should return the latest (highest sequence number)
        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::TaskCompleted);
    }

    #[test]
    fn test_find_terminal_task_event_not_found() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::TaskScheduled,
            json!({"taskExecutionId": "task-1"}),
            now(),
        )];

        let engine = ReplayEngine::new(events);

        assert!(engine.find_terminal_task_event("task-1").is_none());
        assert!(engine.find_terminal_task_event("nonexistent").is_none());
    }

    #[test]
    fn test_find_terminal_timer_event_fired() {
        let events = vec![
            ReplayEvent::new(1, EventType::TimerStarted, json!({"timerId": "t1"}), now()),
            ReplayEvent::new(2, EventType::TimerFired, json!({"timerId": "t1"}), now()),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_timer_event("t1");

        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::TimerFired);
    }

    #[test]
    fn test_find_terminal_timer_event_cancelled() {
        let events = vec![
            ReplayEvent::new(1, EventType::TimerStarted, json!({"timerId": "t1"}), now()),
            ReplayEvent::new(
                2,
                EventType::TimerCancelled,
                json!({"timerId": "t1"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_timer_event("t1");

        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::TimerCancelled);
    }

    #[test]
    fn test_find_terminal_promise_event_resolved() {
        let promise_id = "550e8400-e29b-41d4-a716-446655440000";
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                json!({"promiseName": "approval", "promiseId": promise_id}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseResolved,
                json!({"promiseName": "approval", "promiseId": promise_id, "value": true}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_promise_event(promise_id);

        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::PromiseResolved);
    }

    #[test]
    fn test_find_terminal_promise_event_rejected() {
        let promise_id = "550e8400-e29b-41d4-a716-446655440001";
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                json!({"promiseName": "approval", "promiseId": promise_id}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseRejected,
                json!({"promiseName": "approval", "promiseId": promise_id, "error": "denied"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_promise_event(promise_id);

        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::PromiseRejected);
    }

    #[test]
    fn test_find_terminal_promise_event_timeout() {
        let promise_id = "550e8400-e29b-41d4-a716-446655440002";
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                json!({"promiseName": "approval", "promiseId": promise_id}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseTimeout,
                json!({"promiseName": "approval", "promiseId": promise_id}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_promise_event(promise_id);

        assert!(terminal.is_some());
        assert_eq!(terminal.unwrap().event_type(), EventType::PromiseTimeout);
    }

    #[test]
    fn test_find_terminal_child_workflow_completed() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                json!({"childExecutionName": "payment"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::ChildWorkflowCompleted,
                json!({"childExecutionName": "payment", "output": {"success": true}}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_child_workflow_event("payment");

        assert!(terminal.is_some());
        assert_eq!(
            terminal.unwrap().event_type(),
            EventType::ChildWorkflowCompleted
        );
    }

    #[test]
    fn test_find_terminal_child_workflow_failed() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::ChildWorkflowInitiated,
                json!({"childExecutionName": "payment"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::ChildWorkflowFailed,
                json!({"childExecutionName": "payment", "error": "payment failed"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);
        let terminal = engine.find_terminal_child_workflow_event("payment");

        assert!(terminal.is_some());
        assert_eq!(
            terminal.unwrap().event_type(),
            EventType::ChildWorkflowFailed
        );
    }

    // =========================================================================
    // State Management Tests
    // =========================================================================

    #[test]
    fn test_initial_state_from_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::StateSet,
                json!({"key": "count", "value": 5}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::StateSet,
                json!({"key": "name", "value": "test"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.get_state("count"), Some(json!(5)));
        assert_eq!(engine.get_state("name"), Some(json!("test")));
    }

    #[test]
    fn test_initial_state_cleared_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::StateSet,
                json!({"key": "count", "value": 5}),
                now(),
            ),
            ReplayEvent::new(2, EventType::StateCleared, json!({"key": "count"}), now()),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.get_state("count"), None);
    }

    #[test]
    fn test_initial_state_latest_wins() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::StateSet,
                json!({"key": "count", "value": 1}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::StateSet,
                json!({"key": "count", "value": 2}),
                now(),
            ),
            ReplayEvent::new(
                3,
                EventType::StateSet,
                json!({"key": "count", "value": 3}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.get_state("count"), Some(json!(3)));
    }

    #[test]
    fn test_get_state_returns_value() {
        let engine = ReplayEngine::new(vec![]);
        engine.set_state("key", json!("value"));

        assert_eq!(engine.get_state("key"), Some(json!("value")));
    }

    #[test]
    fn test_get_state_returns_none() {
        let engine = ReplayEngine::new(vec![]);

        assert_eq!(engine.get_state("nonexistent"), None);
    }

    #[test]
    fn test_set_state_adds_new_key() {
        let engine = ReplayEngine::new(vec![]);

        engine.set_state("new_key", json!(42));

        assert_eq!(engine.get_state("new_key"), Some(json!(42)));
    }

    #[test]
    fn test_set_state_overwrites_existing() {
        let engine = ReplayEngine::new(vec![]);

        engine.set_state("key", json!(1));
        engine.set_state("key", json!(2));

        assert_eq!(engine.get_state("key"), Some(json!(2)));
    }

    #[test]
    fn test_clear_state_removes_key() {
        let engine = ReplayEngine::new(vec![]);

        engine.set_state("key", json!(1));
        engine.clear_state("key");

        assert_eq!(engine.get_state("key"), None);
    }

    #[test]
    fn test_clear_state_nonexistent_noop() {
        let engine = ReplayEngine::new(vec![]);

        // Should not panic
        engine.clear_state("nonexistent");
    }

    #[test]
    fn test_state_keys_returns_all_keys() {
        let engine = ReplayEngine::new(vec![]);

        engine.set_state("a", json!(1));
        engine.set_state("b", json!(2));
        engine.set_state("c", json!(3));

        let mut keys = engine.state_keys();
        keys.sort();

        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    // =========================================================================
    // Operation Cache Tests
    // =========================================================================

    #[test]
    fn test_operation_cache_from_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                json!({"operationName": "fetch-user", "result": {"id": 1}}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::OperationCompleted,
                json!({"operationName": "fetch-order", "result": {"orderId": 42}}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(
            engine.get_cached_operation("fetch-user"),
            Some(&json!({"id": 1}))
        );
        assert_eq!(
            engine.get_cached_operation("fetch-order"),
            Some(&json!({"orderId": 42}))
        );
    }

    #[test]
    fn test_get_cached_operation_returns_value() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            json!({"operationName": "op1", "result": "cached"}),
            now(),
        )];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.get_cached_operation("op1"), Some(&json!("cached")));
    }

    #[test]
    fn test_get_cached_operation_returns_none() {
        let engine = ReplayEngine::new(vec![]);

        assert_eq!(engine.get_cached_operation("nonexistent"), None);
    }

    #[test]
    fn test_operation_cache_latest_wins() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                json!({"operationName": "op1", "result": "first"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::OperationCompleted,
                json!({"operationName": "op1", "result": "second"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        assert_eq!(engine.get_cached_operation("op1"), Some(&json!("second")));
    }

    // =========================================================================
    // Integration Tests
    // =========================================================================

    #[test]
    fn test_replay_scenario_task_completed() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                json!({"kind": "send-email", "taskExecutionId": "task-1"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TaskCompleted,
                json!({"taskExecutionId": "task-1", "result": {"sent": true}}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        // Simulate replay
        let seq = engine.next_task_seq();
        assert_eq!(seq, 0);

        let event = engine.get_task_event(seq).unwrap();
        assert_eq!(event.get_string("kind"), Some("send-email"));

        let task_id = event.get_string("taskExecutionId").unwrap();
        let terminal = engine.find_terminal_task_event(task_id).unwrap();
        assert_eq!(terminal.event_type(), EventType::TaskCompleted);
    }

    #[test]
    fn test_replay_scenario_task_pending() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::TaskScheduled,
            json!({"kind": "send-email", "taskExecutionId": "task-1"}),
            now(),
        )];

        let engine = ReplayEngine::new(events);

        let seq = engine.next_task_seq();
        let event = engine.get_task_event(seq).unwrap();
        let task_id = event.get_string("taskExecutionId").unwrap();

        // No terminal event - task is still pending
        assert!(engine.find_terminal_task_event(task_id).is_none());
    }

    #[test]
    fn test_replay_scenario_mixed_events() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::TaskScheduled,
                json!({"kind": "task1", "taskExecutionId": "t1"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TimerStarted,
                json!({"timerId": "timer1"}),
                now(),
            ),
            ReplayEvent::new(
                3,
                EventType::TaskCompleted,
                json!({"taskExecutionId": "t1", "result": 1}),
                now(),
            ),
            ReplayEvent::new(
                4,
                EventType::TaskScheduled,
                json!({"kind": "task2", "taskExecutionId": "t2"}),
                now(),
            ),
            ReplayEvent::new(
                5,
                EventType::TimerFired,
                json!({"timerId": "timer1"}),
                now(),
            ),
        ];

        let engine = ReplayEngine::new(events);

        // Process task 1
        let task_seq = engine.next_task_seq();
        assert_eq!(task_seq, 0);
        let task1_id = engine
            .get_task_event(task_seq)
            .unwrap()
            .get_string("taskExecutionId")
            .unwrap();
        assert!(engine.find_terminal_task_event(task1_id).is_some());

        // Process timer
        let timer_seq = engine.next_timer_seq();
        assert_eq!(timer_seq, 0);
        let timer_id = engine
            .get_timer_event(timer_seq)
            .unwrap()
            .get_string("timerId")
            .unwrap();
        assert!(engine.find_terminal_timer_event(timer_id).is_some());

        // Process task 2
        let task_seq2 = engine.next_task_seq();
        assert_eq!(task_seq2, 1);
        let task2_id = engine
            .get_task_event(task_seq2)
            .unwrap()
            .get_string("taskExecutionId")
            .unwrap();
        assert!(engine.find_terminal_task_event(task2_id).is_none()); // Not completed
    }

    #[test]
    fn test_replay_scenario_determinism() {
        let events = vec![
            ReplayEvent::new(1, EventType::TaskScheduled, json!({"kind": "a"}), now()),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({"kind": "b"}), now()),
            ReplayEvent::new(3, EventType::TaskScheduled, json!({"kind": "c"}), now()),
        ];

        // Create two engines with the same events
        let engine1 = ReplayEngine::new(events.clone());
        let engine2 = ReplayEngine::new(events);

        // Both should produce the same sequence
        assert_eq!(engine1.next_task_seq(), engine2.next_task_seq());
        assert_eq!(engine1.next_task_seq(), engine2.next_task_seq());
        assert_eq!(engine1.next_task_seq(), engine2.next_task_seq());

        // And the same event lookups
        assert_eq!(
            engine1.get_task_event(0).unwrap().get_string("kind"),
            engine2.get_task_event(0).unwrap().get_string("kind")
        );
    }
}
