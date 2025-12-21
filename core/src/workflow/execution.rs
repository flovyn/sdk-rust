//! Workflow execution utilities - language-agnostic replay logic
//!
//! This module provides core utilities for workflow execution that can be
//! shared across different language SDKs.

use crate::workflow::event::{EventType, ReplayEvent};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Workflow metadata extracted from a workflow definition.
///
/// This represents the static metadata about a workflow type,
/// independent of any specific execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMetadata {
    /// Unique workflow kind identifier (e.g., "order-workflow", "payment-flow")
    pub kind: String,
    /// Human-readable name for display purposes
    pub name: String,
    /// Optional description of what the workflow does
    pub description: Option<String>,
    /// Semantic version of the workflow implementation (as string, e.g., "1.2.0")
    pub version: Option<String>,
    /// Tags for categorization and filtering
    pub tags: Vec<String>,
    /// Whether the workflow supports graceful cancellation
    pub cancellable: bool,
    /// Timeout in seconds (None means use executor default)
    pub timeout_seconds: Option<u64>,
    /// SHA-256 hash of workflow content for version validation
    pub content_hash: Option<String>,
}

impl WorkflowMetadata {
    /// Create a new WorkflowMetadata with required fields only
    pub fn new(kind: impl Into<String>) -> Self {
        let kind = kind.into();
        Self {
            kind: kind.clone(),
            name: kind,
            description: None,
            version: None,
            tags: Vec::new(),
            cancellable: true,
            timeout_seconds: None,
            content_hash: None,
        }
    }

    /// Set the human-readable name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set whether the workflow is cancellable
    pub fn with_cancellable(mut self, cancellable: bool) -> Self {
        self.cancellable = cancellable;
        self
    }

    /// Set the timeout in seconds
    pub fn with_timeout_seconds(mut self, timeout: u64) -> Self {
        self.timeout_seconds = Some(timeout);
        self
    }

    /// Set the content hash
    pub fn with_content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
        self
    }
}

/// Trait for deterministic random number generation.
///
/// Workflows must use deterministic random to ensure replay consistency.
/// All random values are derived from a seed (typically the workflow execution ID).
pub trait DeterministicRandom: Send + Sync {
    /// Generate a random integer in the range [min, max)
    fn next_int(&self, min: i32, max: i32) -> i32;

    /// Generate a random long in the range [min, max)
    fn next_long(&self, min: i64, max: i64) -> i64;

    /// Generate a random double in the range [0, 1)
    fn next_double(&self) -> f64;

    /// Generate a random boolean
    fn next_bool(&self) -> bool;
}

/// Seeded deterministic random number generator using xorshift64.
///
/// This implementation provides consistent random sequences across
/// replays when initialized with the same seed.
pub struct SeededRandom {
    /// Current seed state
    state: RwLock<u64>,
}

impl SeededRandom {
    /// Create a new seeded random with the given seed
    pub fn new(seed: u64) -> Self {
        Self {
            state: RwLock::new(seed),
        }
    }

    /// Get next random u64 using xorshift64
    fn next_u64(&self) -> u64 {
        let mut state = self.state.write();
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }
}

impl DeterministicRandom for SeededRandom {
    fn next_int(&self, min: i32, max: i32) -> i32 {
        if min >= max {
            return min;
        }
        let range = (max - min) as u64;
        let random = self.next_u64();
        min + (random % range) as i32
    }

    fn next_long(&self, min: i64, max: i64) -> i64 {
        if min >= max {
            return min;
        }
        let range = (max - min) as u64;
        let random = self.next_u64();
        min + (random % range) as i64
    }

    fn next_double(&self) -> f64 {
        let random = self.next_u64();
        // Convert to f64 in range [0, 1)
        (random as f64) / (u64::MAX as f64)
    }

    fn next_bool(&self) -> bool {
        self.next_u64().is_multiple_of(2)
    }
}

/// Utilities for finding events in replay history.
///
/// These functions help locate specific events during workflow replay,
/// such as terminal task events or timer events.
pub struct EventLookup;

impl EventLookup {
    /// Find terminal event (TaskCompleted or TaskFailed) for a task by taskExecutionId.
    /// Returns the latest terminal event if multiple exist (e.g., retries).
    pub fn find_terminal_task_event<'a>(
        events: &'a [ReplayEvent],
        task_execution_id: &str,
    ) -> Option<&'a ReplayEvent> {
        events
            .iter()
            .filter(|e| {
                let matches_id = e
                    .get_string("taskExecutionId")
                    .or_else(|| e.get_string("taskId"))
                    .map(|id| id == task_execution_id)
                    .unwrap_or(false);
                matches_id
                    && (e.event_type() == EventType::TaskCompleted
                        || e.event_type() == EventType::TaskFailed)
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Find terminal event (ChildWorkflowCompleted or ChildWorkflowFailed) for a child workflow.
    pub fn find_terminal_child_workflow_event<'a>(
        events: &'a [ReplayEvent],
        name: &str,
    ) -> Option<&'a ReplayEvent> {
        events
            .iter()
            .filter(|e| {
                e.get_string("childExecutionName")
                    .map(|n| n == name)
                    .unwrap_or(false)
                    && (e.event_type() == EventType::ChildWorkflowCompleted
                        || e.event_type() == EventType::ChildWorkflowFailed)
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Find terminal event (TimerFired or TimerCancelled) for a timer by timerId.
    pub fn find_terminal_timer_event<'a>(
        events: &'a [ReplayEvent],
        timer_id: &str,
    ) -> Option<&'a ReplayEvent> {
        events
            .iter()
            .filter(|e| {
                e.get_string("timerId")
                    .map(|id| id == timer_id)
                    .unwrap_or(false)
                    && (e.event_type() == EventType::TimerFired
                        || e.event_type() == EventType::TimerCancelled)
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Find terminal event (PromiseResolved, PromiseRejected, or PromiseTimeout) for a promise.
    pub fn find_terminal_promise_event<'a>(
        events: &'a [ReplayEvent],
        promise_name: &str,
    ) -> Option<&'a ReplayEvent> {
        events
            .iter()
            .filter(|e| {
                // PromiseCreated uses promiseId, terminal events use promiseName
                let matches_name = e
                    .get_string("promiseName")
                    .or_else(|| e.get_string("promiseId"))
                    .map(|n| n == promise_name)
                    .unwrap_or(false);
                matches_name
                    && (e.event_type() == EventType::PromiseResolved
                        || e.event_type() == EventType::PromiseRejected
                        || e.event_type() == EventType::PromiseTimeout)
            })
            .max_by_key(|e| e.sequence_number())
    }

    /// Filter events by type for efficient lookup during replay.
    pub fn filter_events_by_type(
        events: &[ReplayEvent],
        event_type: EventType,
    ) -> Vec<ReplayEvent> {
        events
            .iter()
            .filter(|e| e.event_type() == event_type)
            .cloned()
            .collect()
    }

    /// Filter events matching multiple types (e.g., StateSet or StateCleared).
    pub fn filter_events_by_types(
        events: &[ReplayEvent],
        event_types: &[EventType],
    ) -> Vec<ReplayEvent> {
        events
            .iter()
            .filter(|e| event_types.contains(&e.event_type()))
            .cloned()
            .collect()
    }
}

/// Pre-populated operation cache from existing events.
///
/// During replay, operation results are cached to avoid re-execution.
/// This function extracts operation results from OperationCompleted events.
pub fn build_operation_cache(
    events: &[ReplayEvent],
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut cache = std::collections::HashMap::new();
    for event in events {
        if event.event_type() == EventType::OperationCompleted {
            if let Some(name) = event.get_string("operationName") {
                if let Some(result) = event.get("result") {
                    cache.insert(name.to_string(), result.clone());
                }
            }
        }
    }
    cache
}

/// Build initial state from existing events.
///
/// During replay, state is reconstructed from StateSet/StateCleared events.
pub fn build_initial_state(
    events: &[ReplayEvent],
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut state = std::collections::HashMap::new();
    for event in events {
        match event.event_type() {
            EventType::StateSet => {
                if let Some(key) = event.get_string("key") {
                    if let Some(value) = event.get("value") {
                        state.insert(key.to_string(), value.clone());
                    }
                }
            }
            EventType::StateCleared => {
                if let Some(key) = event.get_string("key") {
                    state.remove(key);
                }
            }
            _ => {}
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    // ==================== SeededRandom Tests ====================

    #[test]
    fn test_seeded_random_deterministic() {
        let r1 = SeededRandom::new(12345);
        let r2 = SeededRandom::new(12345);

        // Same seed should produce same sequence
        assert_eq!(r1.next_int(0, 100), r2.next_int(0, 100));
        assert_eq!(r1.next_long(0, 1000), r2.next_long(0, 1000));
        assert_eq!(r1.next_double(), r2.next_double());
        assert_eq!(r1.next_bool(), r2.next_bool());
    }

    #[test]
    fn test_seeded_random_different_seeds() {
        let r1 = SeededRandom::new(12345);
        let r2 = SeededRandom::new(67890);

        // Different seeds should (likely) produce different values
        // Note: There's a tiny chance they could match, but it's astronomically unlikely
        let v1: Vec<i32> = (0..10).map(|_| r1.next_int(0, 10000)).collect();
        let v2: Vec<i32> = (0..10).map(|_| r2.next_int(0, 10000)).collect();
        assert_ne!(v1, v2);
    }

    #[test]
    fn test_seeded_random_range() {
        let r = SeededRandom::new(42);

        for _ in 0..100 {
            let v = r.next_int(10, 20);
            assert!((10..20).contains(&v));
        }

        for _ in 0..100 {
            let v = r.next_long(100, 200);
            assert!((100..200).contains(&v));
        }

        for _ in 0..100 {
            let v = r.next_double();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn test_seeded_random_edge_cases() {
        let r = SeededRandom::new(1);

        // min >= max should return min
        assert_eq!(r.next_int(5, 5), 5);
        assert_eq!(r.next_int(10, 5), 10);
        assert_eq!(r.next_long(100, 100), 100);
        assert_eq!(r.next_long(200, 100), 200);
    }

    // ==================== EventLookup Tests ====================

    #[test]
    fn test_find_terminal_task_event() {
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
            ReplayEvent::new(
                3,
                EventType::TaskScheduled,
                json!({"taskExecutionId": "task-2"}),
                now(),
            ),
        ];

        let result = EventLookup::find_terminal_task_event(&events, "task-1");
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type(), EventType::TaskCompleted);

        let result = EventLookup::find_terminal_task_event(&events, "task-2");
        assert!(result.is_none()); // No terminal event yet

        let result = EventLookup::find_terminal_task_event(&events, "task-nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_find_terminal_timer_event() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::TimerStarted,
                json!({"timerId": "timer-1"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::TimerFired,
                json!({"timerId": "timer-1"}),
                now(),
            ),
        ];

        let result = EventLookup::find_terminal_timer_event(&events, "timer-1");
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type(), EventType::TimerFired);
    }

    #[test]
    fn test_find_terminal_promise_event() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::PromiseCreated,
                json!({"promiseId": "approval"}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::PromiseResolved,
                json!({"promiseName": "approval", "value": true}),
                now(),
            ),
        ];

        let result = EventLookup::find_terminal_promise_event(&events, "approval");
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type(), EventType::PromiseResolved);
    }

    #[test]
    fn test_filter_events_by_type() {
        let events = vec![
            ReplayEvent::new(1, EventType::TaskScheduled, json!({}), now()),
            ReplayEvent::new(2, EventType::OperationCompleted, json!({}), now()),
            ReplayEvent::new(3, EventType::TaskScheduled, json!({}), now()),
            ReplayEvent::new(4, EventType::TimerStarted, json!({}), now()),
        ];

        let tasks = EventLookup::filter_events_by_type(&events, EventType::TaskScheduled);
        assert_eq!(tasks.len(), 2);

        let operations = EventLookup::filter_events_by_type(&events, EventType::OperationCompleted);
        assert_eq!(operations.len(), 1);
    }

    #[test]
    fn test_filter_events_by_types() {
        let events = vec![
            ReplayEvent::new(1, EventType::StateSet, json!({"key": "a"}), now()),
            ReplayEvent::new(2, EventType::OperationCompleted, json!({}), now()),
            ReplayEvent::new(3, EventType::StateCleared, json!({"key": "b"}), now()),
            ReplayEvent::new(4, EventType::StateSet, json!({"key": "c"}), now()),
        ];

        let state_events = EventLookup::filter_events_by_types(
            &events,
            &[EventType::StateSet, EventType::StateCleared],
        );
        assert_eq!(state_events.len(), 3);
    }

    // ==================== Cache/State Building Tests ====================

    #[test]
    fn test_build_operation_cache() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                json!({"operationName": "fetch-user", "result": {"id": 1}}),
                now(),
            ),
            ReplayEvent::new(2, EventType::TaskScheduled, json!({}), now()),
            ReplayEvent::new(
                3,
                EventType::OperationCompleted,
                json!({"operationName": "fetch-order", "result": {"orderId": 42}}),
                now(),
            ),
        ];

        let cache = build_operation_cache(&events);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("fetch-user"), Some(&json!({"id": 1})));
        assert_eq!(cache.get("fetch-order"), Some(&json!({"orderId": 42})));
    }

    #[test]
    fn test_build_initial_state() {
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
                json!({"key": "name", "value": "test"}),
                now(),
            ),
            ReplayEvent::new(3, EventType::StateCleared, json!({"key": "count"}), now()),
            ReplayEvent::new(
                4,
                EventType::StateSet,
                json!({"key": "count", "value": 5}),
                now(),
            ),
        ];

        let state = build_initial_state(&events);
        assert_eq!(state.len(), 2);
        assert_eq!(state.get("count"), Some(&json!(5)));
        assert_eq!(state.get("name"), Some(&json!("test")));
    }
}
