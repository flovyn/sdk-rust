//! Test task definitions for E2E tests

#![allow(dead_code)] // Fixtures will be used when tests are implemented

use async_trait::async_trait;
use flovyn_worker_sdk::error::{FlovynError, Result};
use flovyn_worker_sdk::task::context::TaskContext;
use flovyn_worker_sdk::task::definition::{DynamicTask, DynamicTaskInput, DynamicTaskOutput};

/// Task that echoes its input.
pub struct EchoTask;

#[async_trait]
impl DynamicTask for EchoTask {
    fn kind(&self) -> &str {
        "echo-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        Ok(input)
    }
}

/// Task that sleeps for a configurable duration.
pub struct SlowTask;

#[async_trait]
impl DynamicTask for SlowTask {
    fn kind(&self) -> &str {
        "slow-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let sleep_ms = input
            .get("sleepMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "sleptMs".to_string(),
            serde_json::Value::Number(sleep_ms.into()),
        );
        Ok(output)
    }
}

/// Task that fails a configurable number of times before succeeding.
pub struct FailingTask {
    attempts: std::sync::atomic::AtomicU32,
    fail_count: u32,
}

impl FailingTask {
    pub fn new(fail_count: u32) -> Self {
        Self {
            attempts: std::sync::atomic::AtomicU32::new(0),
            fail_count,
        }
    }
}

impl Default for FailingTask {
    fn default() -> Self {
        Self::new(2) // Fail twice, succeed on third attempt
    }
}

#[async_trait]
impl DynamicTask for FailingTask {
    fn kind(&self) -> &str {
        "failing-task"
    }

    async fn execute(
        &self,
        _input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let attempt = self
            .attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if attempt < self.fail_count {
            return Err(FlovynError::Other(format!(
                "Intentional failure (attempt {})",
                attempt + 1
            )));
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "attempts".to_string(),
            serde_json::Value::Number((attempt + 1).into()),
        );
        Ok(output)
    }
}

/// Task that reports progress.
pub struct ProgressTask;

#[async_trait]
impl DynamicTask for ProgressTask {
    fn kind(&self) -> &str {
        "progress-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let steps = input.get("steps").and_then(|v| v.as_u64()).unwrap_or(5) as u32;

        for i in 0..steps {
            let progress = (i + 1) as f64 / steps as f64;
            let message = format!("Step {}/{}", i + 1, steps);
            ctx.report_progress(progress, Some(&message)).await?;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "completedSteps".to_string(),
            serde_json::Value::Number(steps.into()),
        );
        Ok(output)
    }
}

// ============================================================================
// Parallel Execution Task Fixtures
// ============================================================================

/// Helper to add prefix to a kind name
fn prefixed(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}:{}", prefix, name)
    }
}

/// Task that processes a single item.
pub struct ProcessItemTask {
    kind: String,
}

impl ProcessItemTask {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            kind: prefixed(prefix, "process-item"),
        }
    }
}

impl Default for ProcessItemTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicTask for ProcessItemTask {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let item = input
            .get("item")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let processed = format!("processed:{}", item);

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "processed".to_string(),
            serde_json::Value::String(processed),
        );
        output.insert(
            "originalItem".to_string(),
            serde_json::Value::String(item.to_string()),
        );
        Ok(output)
    }
}

/// Task that fetches data from a URL.
pub struct FetchDataTask {
    kind: String,
}

impl FetchDataTask {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            kind: prefixed(prefix, "fetch-data"),
        }
    }
}

impl Default for FetchDataTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicTask for FetchDataTask {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://example.com");
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "source".to_string(),
            serde_json::Value::String(url.to_string()),
        );
        output.insert(
            "name".to_string(),
            serde_json::Value::String(name.to_string()),
        );
        output.insert("data".to_string(), serde_json::json!({ "response": "ok" }));
        Ok(output)
    }
}

/// Task that simulates a slow operation.
pub struct SlowOperationTask {
    kind: String,
}

impl SlowOperationTask {
    pub fn new() -> Self {
        Self::with_prefix("")
    }

    pub fn with_prefix(prefix: &str) -> Self {
        Self {
            kind: prefixed(prefix, "slow-operation"),
        }
    }
}

impl Default for SlowOperationTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DynamicTask for SlowOperationTask {
    fn kind(&self) -> &str {
        &self.kind
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        _ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let op = input
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let delay_ms = input.get("delayMs").and_then(|v| v.as_u64()).unwrap_or(0);

        // Simulate slow operation
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "operation".to_string(),
            serde_json::Value::String(op.to_string()),
        );
        output.insert("completed".to_string(), serde_json::Value::Bool(true));
        output.insert(
            "result".to_string(),
            serde_json::json!({ "operation": op, "success": true }),
        );
        Ok(output)
    }
}

// ============================================================================
// Streaming Task Fixtures
// ============================================================================

/// Task that streams tokens (simulates LLM token streaming).
/// Streams configurable number of tokens, then returns the complete text.
pub struct StreamingTokenTask;

#[async_trait]
impl DynamicTask for StreamingTokenTask {
    fn kind(&self) -> &str {
        "streaming-token-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let tokens: Vec<&str> = input
            .get("tokens")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| vec!["Hello", ", ", "world", "!"]);

        // Stream each token
        for token in &tokens {
            ctx.stream_token(token).await?;
            // Small delay between tokens to simulate streaming
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "tokenCount".to_string(),
            serde_json::Value::Number(tokens.len().into()),
        );
        output.insert(
            "fullText".to_string(),
            serde_json::Value::String(tokens.join("")),
        );
        Ok(output)
    }

    fn uses_streaming(&self) -> bool {
        true
    }
}

/// Task that streams progress updates.
/// Streams progress 0%, 25%, 50%, 75%, 100% with details.
pub struct StreamingProgressTask;

#[async_trait]
impl DynamicTask for StreamingProgressTask {
    fn kind(&self) -> &str {
        "streaming-progress-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let steps = input.get("steps").and_then(|v| v.as_u64()).unwrap_or(4) as u32;

        for i in 0..=steps {
            let progress = i as f64 / steps as f64;
            let details = format!("Step {}/{}", i, steps);
            ctx.stream_progress(progress, Some(&details)).await?;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "completedSteps".to_string(),
            serde_json::Value::Number(steps.into()),
        );
        Ok(output)
    }

    fn uses_streaming(&self) -> bool {
        true
    }
}

/// Task that streams structured data events.
/// Streams a series of data events with structured payloads.
pub struct StreamingDataTask;

#[async_trait]
impl DynamicTask for StreamingDataTask {
    fn kind(&self) -> &str {
        "streaming-data-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let count = input.get("count").and_then(|v| v.as_u64()).unwrap_or(3) as u32;

        for i in 0..count {
            ctx.stream_data_value(serde_json::json!({
                "index": i,
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                "data": format!("record-{}", i)
            }))
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let mut output = DynamicTaskOutput::new();
        output.insert(
            "recordCount".to_string(),
            serde_json::Value::Number(count.into()),
        );
        Ok(output)
    }

    fn uses_streaming(&self) -> bool {
        true
    }
}

/// Task that streams error notifications.
/// Streams recoverable error events, then completes successfully.
pub struct StreamingErrorTask;

#[async_trait]
impl DynamicTask for StreamingErrorTask {
    fn kind(&self) -> &str {
        "streaming-error-task"
    }

    async fn execute(
        &self,
        input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        let error_count = input
            .get("errorCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(2) as u32;

        for i in 0..error_count {
            ctx.stream_error(
                &format!("Recoverable error {} (retrying...)", i + 1),
                Some(&format!("ERR_{:03}", i + 1)),
            )
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Task completes successfully despite streaming errors
        let mut output = DynamicTaskOutput::new();
        output.insert(
            "errorCount".to_string(),
            serde_json::Value::Number(error_count.into()),
        );
        output.insert("success".to_string(), serde_json::Value::Bool(true));
        Ok(output)
    }

    fn uses_streaming(&self) -> bool {
        true
    }
}

/// Task that streams all event types (tokens, progress, data, errors).
/// Used for comprehensive streaming tests.
pub struct StreamingAllTypesTask;

#[async_trait]
impl DynamicTask for StreamingAllTypesTask {
    fn kind(&self) -> &str {
        "streaming-all-types-task"
    }

    async fn execute(
        &self,
        _input: DynamicTaskInput,
        ctx: &dyn TaskContext,
    ) -> Result<DynamicTaskOutput> {
        // Stream progress
        ctx.stream_progress(0.0, Some("Starting")).await?;

        // Stream tokens
        ctx.stream_token("Processing").await?;
        ctx.stream_token("...").await?;

        // Stream data
        ctx.stream_data_value(serde_json::json!({"phase": "processing"}))
            .await?;

        // Stream progress
        ctx.stream_progress(0.5, Some("Halfway")).await?;

        // Stream an error notification (recoverable)
        ctx.stream_error("Minor issue encountered", Some("WARN_001"))
            .await?;

        // More tokens
        ctx.stream_token(" Done!").await?;

        // Final progress
        ctx.stream_progress(1.0, Some("Complete")).await?;

        let mut output = DynamicTaskOutput::new();
        output.insert("success".to_string(), serde_json::Value::Bool(true));
        Ok(output)
    }

    fn uses_streaming(&self) -> bool {
        true
    }
}
