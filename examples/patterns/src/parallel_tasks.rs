//! Task Definitions for Parallel Execution Patterns
//!
//! This module contains task definitions that work with the parallel execution
//! workflow patterns. In a real application, these would implement actual business logic.
//!
//! These tasks are referenced by the parallel workflows using `schedule_async_raw`:
//! - `process-item`: General item processing
//! - `fetch-data`: Fetch data from a URL
//! - `slow-operation`: Simulates a slow/expensive operation
//! - `run-operation`: Executes a named operation
//! - `fetch-items`: Fetches a dynamic list of items from a source

use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ============================================================================
// ProcessItemTask - Used by fan-out/fan-in and batch processing
// ============================================================================

/// Input for processing a single item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessItemInput {
    /// The item to process
    pub item: String,
}

/// Output from processing a single item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessItemOutput {
    /// The processed item result
    pub processed: String,
}

/// Task that processes a single item.
///
/// Used by:
/// - `FanOutFanInWorkflow`
/// - `BatchWithConcurrencyWorkflow`
/// - `DynamicParallelismWorkflow`
pub struct ProcessItemTask;

#[async_trait]
impl TaskDefinition for ProcessItemTask {
    type Input = ProcessItemInput;
    type Output = ProcessItemOutput;

    fn kind(&self) -> &str {
        "process-item"
    }

    fn name(&self) -> &str {
        "Process Item"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Processes a single item and returns the result")
    }

    async fn execute(&self, input: Self::Input, _ctx: &dyn TaskContext) -> Result<Self::Output> {
        // In a real implementation, this would do actual processing
        Ok(ProcessItemOutput {
            processed: format!("processed:{}", input.item),
        })
    }
}

// ============================================================================
// FetchDataTask - Used by racing workflow
// ============================================================================

/// Input for fetching data from a URL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchDataInput {
    /// The URL to fetch from
    pub url: String,
}

/// Output from fetching data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchDataOutput {
    /// The fetched data
    pub data: Value,
    /// The source URL
    pub source: String,
}

/// Task that fetches data from a URL.
///
/// Used by:
/// - `RacingWorkflow` (for primary and fallback fetches)
pub struct FetchDataTask;

#[async_trait]
impl TaskDefinition for FetchDataTask {
    type Input = FetchDataInput;
    type Output = FetchDataOutput;

    fn kind(&self) -> &str {
        "fetch-data"
    }

    fn name(&self) -> &str {
        "Fetch Data"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Fetches data from a specified URL")
    }

    async fn execute(&self, input: Self::Input, _ctx: &dyn TaskContext) -> Result<Self::Output> {
        // In a real implementation, this would make an HTTP request
        Ok(FetchDataOutput {
            data: json!({"response": "ok"}),
            source: input.url,
        })
    }
}

// ============================================================================
// SlowOperationTask - Used by timeout workflow
// ============================================================================

/// Input for a slow operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowOperationInput {
    /// The operation to perform
    pub op: String,
}

/// Output from a slow operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlowOperationOutput {
    /// The result of the operation
    pub result: Value,
}

/// Task that simulates a slow operation.
///
/// Used by:
/// - `TimeoutWorkflow`
pub struct SlowOperationTask;

#[async_trait]
impl TaskDefinition for SlowOperationTask {
    type Input = SlowOperationInput;
    type Output = SlowOperationOutput;

    fn kind(&self) -> &str {
        "slow-operation"
    }

    fn name(&self) -> &str {
        "Slow Operation"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Simulates a slow/expensive operation")
    }

    async fn execute(&self, input: Self::Input, _ctx: &dyn TaskContext) -> Result<Self::Output> {
        // In a real implementation, this would do expensive work
        Ok(SlowOperationOutput {
            result: json!({"operation": input.op, "completed": true}),
        })
    }
}

// ============================================================================
// RunOperationTask - Used by partial completion workflow
// ============================================================================

/// Input for running a named operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOperationInput {
    /// The operation name
    pub operation: String,
}

/// Output from running an operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOperationOutput {
    /// The operation result
    pub result: Value,
    /// Whether the operation succeeded
    pub success: bool,
}

/// Task that runs a named operation.
///
/// Used by:
/// - `PartialCompletionWorkflow`
pub struct RunOperationTask;

#[async_trait]
impl TaskDefinition for RunOperationTask {
    type Input = RunOperationInput;
    type Output = RunOperationOutput;

    fn kind(&self) -> &str {
        "run-operation"
    }

    fn name(&self) -> &str {
        "Run Operation"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Runs a named operation and returns the result")
    }

    async fn execute(&self, input: Self::Input, _ctx: &dyn TaskContext) -> Result<Self::Output> {
        Ok(RunOperationOutput {
            result: json!({"operation": input.operation}),
            success: true,
        })
    }
}

// ============================================================================
// FetchItemsTask - Used by dynamic parallelism workflow
// ============================================================================

/// Input for fetching a list of items
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchItemsInput {
    /// The source to fetch items from
    pub source: String,
}

/// Output containing a dynamic list of items
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchItemsOutput {
    /// The list of items to process (count determined at runtime)
    pub items: Vec<String>,
}

/// Task that fetches a dynamic list of items from a source.
///
/// This is key to the dynamic parallelism pattern - the number
/// of items (and thus parallel operations) is determined at runtime.
///
/// Used by:
/// - `DynamicParallelismWorkflow`
pub struct FetchItemsTask;

#[async_trait]
impl TaskDefinition for FetchItemsTask {
    type Input = FetchItemsInput;
    type Output = FetchItemsOutput;

    fn kind(&self) -> &str {
        "fetch-items"
    }

    fn name(&self) -> &str {
        "Fetch Items"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Fetches a dynamic list of items from a source")
    }

    async fn execute(&self, input: Self::Input, _ctx: &dyn TaskContext) -> Result<Self::Output> {
        // In a real implementation, this would query a database, API, etc.
        // The number of items is determined at runtime
        let items = match input.source.as_str() {
            "database" => vec![
                "item-1".to_string(),
                "item-2".to_string(),
                "item-3".to_string(),
                "item-4".to_string(),
                "item-5".to_string(),
            ],
            "api" => vec!["api-item-1".to_string(), "api-item-2".to_string()],
            _ => vec![],
        };

        Ok(FetchItemsOutput { items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_item_input_serialization() {
        let input = ProcessItemInput {
            item: "test-item".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["item"], "test-item");
    }

    #[test]
    fn test_fetch_data_input_serialization() {
        let input = FetchDataInput {
            url: "http://example.com".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["url"], "http://example.com");
    }

    #[test]
    fn test_slow_operation_input_serialization() {
        let input = SlowOperationInput {
            op: "compute".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["op"], "compute");
    }

    #[test]
    fn test_run_operation_input_serialization() {
        let input = RunOperationInput {
            operation: "validate".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["operation"], "validate");
    }

    #[test]
    fn test_fetch_items_input_serialization() {
        let input = FetchItemsInput {
            source: "database".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["source"], "database");
    }

    #[test]
    fn test_fetch_items_output_serialization() {
        let output = FetchItemsOutput {
            items: vec!["a".to_string(), "b".to_string()],
        };
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["items"], json!(["a", "b"]));
    }
}
