//! SDK Telemetry module for distributed tracing via span proxy.
//!
//! This module collects execution spans during workflow and task execution
//! and reports them to the server for unified distributed traces.
//!
//! The server creates OTLP spans from SDK data, so the SDK doesn't need
//! OpenTelemetry dependencies.

use crate::client::WorkflowDispatch;
use flovyn_core::generated::flovyn_v1::{ExecutionSpan, SdkInfo};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// SDK version (from Cargo.toml)
const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Maximum spans to batch before flushing
const MAX_BATCH_SIZE: usize = 50;

/// Get the current time in nanoseconds since Unix epoch
fn now_unix_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Build SDK info with auto-detected environment details
pub fn build_sdk_info() -> SdkInfo {
    SdkInfo {
        language: "rust".to_string(),
        sdk_version: SDK_VERSION.to_string(),
        runtime_version: Some(rustc_version()),
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        hostname: hostname::get().ok().and_then(|h| h.into_string().ok()),
    }
}

/// Get rustc version at compile time
fn rustc_version() -> String {
    // Use CARGO_CFG_TARGET_ARCH as a proxy for Rust version info
    // We can't easily get rustc version at runtime, so just use a placeholder
    format!("rustc-{}", std::env::consts::ARCH)
}

/// A span being recorded
#[derive(Debug, Clone)]
pub struct RecordingSpan {
    span_id: String,
    parent_span_id: Option<String>,
    span_type: String,
    workflow_id: String,
    task_id: Option<String>,
    start_time_unix_ns: i64,
    attributes: HashMap<String, String>,
}

impl RecordingSpan {
    /// Create a new recording span
    pub fn new(span_type: &str, workflow_id: &str) -> Self {
        Self {
            span_id: Uuid::new_v4().to_string(),
            parent_span_id: None,
            span_type: span_type.to_string(),
            workflow_id: workflow_id.to_string(),
            task_id: None,
            start_time_unix_ns: now_unix_ns(),
            attributes: HashMap::new(),
        }
    }

    /// Set parent span ID
    pub fn with_parent(mut self, parent_span_id: &str) -> Self {
        self.parent_span_id = Some(parent_span_id.to_string());
        self
    }

    /// Set task ID
    pub fn with_task_id(mut self, task_id: &str) -> Self {
        self.task_id = Some(task_id.to_string());
        self
    }

    /// Add an attribute
    pub fn with_attribute(mut self, key: &str, value: &str) -> Self {
        self.attributes.insert(key.to_string(), value.to_string());
        self
    }

    /// Get the span ID
    pub fn span_id(&self) -> &str {
        &self.span_id
    }

    /// Finish the span and convert to ExecutionSpan
    pub fn finish(self) -> ExecutionSpan {
        ExecutionSpan {
            span_id: self.span_id,
            parent_span_id: self.parent_span_id,
            span_type: self.span_type,
            workflow_id: self.workflow_id,
            task_id: self.task_id,
            start_time_unix_ns: self.start_time_unix_ns,
            end_time_unix_ns: now_unix_ns(),
            is_error: false,
            error_type: None,
            error_message: None,
            attributes: self.attributes,
        }
    }

    /// Finish the span with an error
    pub fn finish_with_error(self, error_type: &str, error_message: &str) -> ExecutionSpan {
        ExecutionSpan {
            span_id: self.span_id,
            parent_span_id: self.parent_span_id,
            span_type: self.span_type,
            workflow_id: self.workflow_id,
            task_id: self.task_id,
            start_time_unix_ns: self.start_time_unix_ns,
            end_time_unix_ns: now_unix_ns(),
            is_error: true,
            error_type: Some(error_type.to_string()),
            error_message: Some(error_message.to_string()),
            attributes: self.attributes,
        }
    }
}

/// Span collector for batching and reporting execution spans
#[derive(Clone)]
pub struct SpanCollector {
    inner: Arc<Mutex<SpanCollectorInner>>,
    sdk_info: SdkInfo,
    enabled: bool,
}

struct SpanCollectorInner {
    spans: Vec<ExecutionSpan>,
}

impl SpanCollector {
    /// Create a new span collector
    pub fn new(enabled: bool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SpanCollectorInner { spans: Vec::new() })),
            sdk_info: build_sdk_info(),
            enabled,
        }
    }

    /// Check if telemetry is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Record a completed span
    pub fn record(&self, span: ExecutionSpan) {
        if !self.enabled {
            return;
        }

        let mut inner = self.inner.lock();
        inner.spans.push(span);
    }

    /// Get number of pending spans
    pub fn pending_count(&self) -> usize {
        self.inner.lock().spans.len()
    }

    /// Check if we should flush (batch size reached)
    pub fn should_flush(&self) -> bool {
        self.pending_count() >= MAX_BATCH_SIZE
    }

    /// Take all pending spans for flushing
    fn take_spans(&self) -> Vec<ExecutionSpan> {
        let mut inner = self.inner.lock();
        std::mem::take(&mut inner.spans)
    }

    /// Flush all pending spans to the server
    ///
    /// This is fire-and-forget - telemetry failures don't fail workflow/task execution.
    pub async fn flush(&self, client: &mut WorkflowDispatch) {
        if !self.enabled {
            return;
        }

        let spans = self.take_spans();
        if spans.is_empty() {
            return;
        }

        tracing::debug!(span_count = spans.len(), "Flushing execution spans");

        // Fire-and-forget: don't fail if telemetry reporting fails
        match client
            .report_execution_spans(self.sdk_info.clone(), spans)
            .await
        {
            Ok(result) => {
                if result.rejected_count > 0 {
                    tracing::warn!(
                        accepted = result.accepted_count,
                        rejected = result.rejected_count,
                        "Some spans were rejected"
                    );
                } else {
                    tracing::debug!(accepted = result.accepted_count, "Spans reported");
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "Failed to report execution spans");
            }
        }
    }
}

impl Default for SpanCollector {
    fn default() -> Self {
        Self::new(false)
    }
}

/// Helper to create a workflow.execute span
pub fn workflow_execute_span(workflow_id: &str, workflow_kind: &str) -> RecordingSpan {
    RecordingSpan::new("workflow.execute", workflow_id)
        .with_attribute("workflow.kind", workflow_kind)
}

/// Helper to create a workflow.replay span
pub fn workflow_replay_span(workflow_id: &str, event_count: usize) -> RecordingSpan {
    RecordingSpan::new("workflow.replay", workflow_id)
        .with_attribute("replay.event_count", &event_count.to_string())
}

/// Helper to create a task.execute span
pub fn task_execute_span(workflow_id: &str, task_id: &str, task_type: &str) -> RecordingSpan {
    RecordingSpan::new("task.execute", workflow_id)
        .with_task_id(task_id)
        .with_attribute("task.type", task_type)
}

/// Helper to create a task.retry span
pub fn task_retry_span(
    workflow_id: &str,
    task_id: &str,
    task_type: &str,
    attempt: i32,
    max_attempts: i32,
) -> RecordingSpan {
    RecordingSpan::new("task.retry", workflow_id)
        .with_task_id(task_id)
        .with_attribute("task.type", task_type)
        .with_attribute("attempt", &attempt.to_string())
        .with_attribute("max_attempts", &max_attempts.to_string())
}

/// Helper to create an activity.execute span (for non-deterministic operations)
pub fn activity_execute_span(
    workflow_id: &str,
    activity_name: &str,
    parent_span_id: Option<&str>,
) -> RecordingSpan {
    let mut span = RecordingSpan::new("activity.execute", workflow_id)
        .with_attribute("activity.name", activity_name);

    if let Some(parent) = parent_span_id {
        span = span.with_parent(parent);
    }

    span
}

/// Helper to create a run.execute span (for cached side-effect operations)
pub fn run_execute_span(workflow_id: &str, operation_name: &str) -> RecordingSpan {
    RecordingSpan::new("run.execute", workflow_id).with_attribute("operation.name", operation_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdk_info() {
        let info = build_sdk_info();
        assert_eq!(info.language, "rust");
        assert!(!info.sdk_version.is_empty());
        assert!(info.os.is_some());
        assert!(info.arch.is_some());
    }

    #[test]
    fn test_recording_span() {
        let span = RecordingSpan::new("workflow.execute", "wf-123")
            .with_attribute("workflow.kind", "my-workflow");

        assert_eq!(span.span_type, "workflow.execute");
        assert_eq!(span.workflow_id, "wf-123");
        assert!(!span.span_id.is_empty());

        let finished = span.finish();
        assert!(!finished.is_error);
        assert!(finished.end_time_unix_ns > finished.start_time_unix_ns);
    }

    #[test]
    fn test_recording_span_with_error() {
        let span = RecordingSpan::new("task.execute", "wf-123").with_task_id("task-456");

        let finished = span.finish_with_error("RuntimeError", "Something went wrong");
        assert!(finished.is_error);
        assert_eq!(finished.error_type, Some("RuntimeError".to_string()));
        assert_eq!(
            finished.error_message,
            Some("Something went wrong".to_string())
        );
    }

    #[test]
    fn test_span_collector_disabled() {
        let collector = SpanCollector::new(false);
        assert!(!collector.is_enabled());

        let span = RecordingSpan::new("workflow.execute", "wf-123").finish();
        collector.record(span);

        assert_eq!(collector.pending_count(), 0);
    }

    #[test]
    fn test_span_collector_enabled() {
        let collector = SpanCollector::new(true);
        assert!(collector.is_enabled());

        let span = RecordingSpan::new("workflow.execute", "wf-123").finish();
        collector.record(span);

        assert_eq!(collector.pending_count(), 1);
    }

    #[test]
    fn test_span_collector_batch_threshold() {
        let collector = SpanCollector::new(true);

        for i in 0..MAX_BATCH_SIZE {
            let span = RecordingSpan::new("workflow.execute", &format!("wf-{}", i)).finish();
            collector.record(span);
        }

        assert!(collector.should_flush());
    }

    #[test]
    fn test_workflow_execute_span_helper() {
        let span = workflow_execute_span("wf-123", "my-workflow");
        assert_eq!(span.span_type, "workflow.execute");
        assert_eq!(span.workflow_id, "wf-123");
        assert_eq!(
            span.attributes.get("workflow.kind"),
            Some(&"my-workflow".to_string())
        );
    }

    #[test]
    fn test_task_execute_span_helper() {
        let span = task_execute_span("wf-123", "task-456", "my-task");
        assert_eq!(span.span_type, "task.execute");
        assert_eq!(span.workflow_id, "wf-123");
        assert_eq!(span.task_id, Some("task-456".to_string()));
        assert_eq!(
            span.attributes.get("task.type"),
            Some(&"my-task".to_string())
        );
    }
}
