//! Parallel Execution Patterns
//!
//! This module demonstrates various parallel execution patterns using the
//! async workflow methods and combinators:
//!
//! 1. **Fan-Out/Fan-In** - Process multiple items in parallel and aggregate results
//! 2. **Racing** - Execute multiple operations and take the first result
//! 3. **Timeout** - Add timeouts to operations
//! 4. **Batch with Concurrency Limit** - Process items with controlled parallelism

use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use flovyn_sdk::workflow::combinators::{join_all, join_n, select, with_timeout};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

// ============================================================================
// Fan-Out/Fan-In Pattern
// ============================================================================

/// Input for the fan-out/fan-in workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanOutFanInInput {
    /// Items to process in parallel
    pub items: Vec<String>,
}

/// Output from the fan-out/fan-in workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanOutFanInOutput {
    /// Aggregated results
    pub results: Vec<String>,
    /// Total processing time in milliseconds
    pub total_time_ms: i64,
}

/// Demonstrates the fan-out/fan-in pattern.
///
/// This workflow:
/// 1. Fans out: Schedules multiple tasks in parallel
/// 2. Waits for all tasks to complete
/// 3. Fans in: Aggregates the results
///
/// # Example
/// ```ignore
/// let input = FanOutFanInInput {
///     items: vec!["item1".to_string(), "item2".to_string(), "item3".to_string()],
/// };
/// let output = client.execute_workflow::<FanOutFanInWorkflow>("fan-out-1", input).await?;
/// ```
pub struct FanOutFanInWorkflow;

#[async_trait]
impl WorkflowDefinition for FanOutFanInWorkflow {
    type Input = FanOutFanInInput;
    type Output = FanOutFanInOutput;

    fn kind(&self) -> &str {
        "fan-out-fan-in-workflow"
    }

    fn name(&self) -> &str {
        "Fan-Out/Fan-In Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates parallel task execution with join_all combinator")
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "parallel".to_string(),
            "fan-out".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let start_time = ctx.current_time_millis();

        // Fan out: Schedule all processing tasks in parallel
        let futures: Vec<_> = input
            .items
            .iter()
            .map(|item| ctx.schedule_async_raw("process-item", json!({ "item": item })))
            .collect();

        // Wait for all tasks to complete
        let results = join_all(futures).await?;

        // Fan in: Aggregate results
        let processed_results: Vec<String> = results
            .into_iter()
            .filter_map(|v| {
                v.get("processed")
                    .and_then(|p| p.as_str())
                    .map(String::from)
            })
            .collect();

        let end_time = ctx.current_time_millis();

        Ok(FanOutFanInOutput {
            results: processed_results,
            total_time_ms: end_time - start_time,
        })
    }
}

// ============================================================================
// Racing Pattern
// ============================================================================

/// Input for the racing workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RacingInput {
    /// Primary service URL
    pub primary_url: String,
    /// Fallback service URL
    pub fallback_url: String,
}

/// Output from the racing workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RacingOutput {
    /// Which service responded first (0 = primary, 1 = fallback)
    pub winner_index: usize,
    /// The response data
    pub response: Value,
}

/// Demonstrates the racing pattern.
///
/// This workflow:
/// 1. Starts multiple competing operations
/// 2. Returns the result of whichever completes first
/// 3. Other operations continue in the background (fire-and-forget)
///
/// Use cases:
/// - Hedged requests (send to multiple replicas, take first response)
/// - Primary/fallback patterns
/// - Speculative execution
pub struct RacingWorkflow;

#[async_trait]
impl WorkflowDefinition for RacingWorkflow {
    type Input = RacingInput;
    type Output = RacingOutput;

    fn kind(&self) -> &str {
        "racing-workflow"
    }

    fn name(&self) -> &str {
        "Racing Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates racing multiple operations with select combinator")
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "parallel".to_string(),
            "racing".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        // Start both requests in parallel
        let futures = vec![
            ctx.schedule_async_raw("fetch-data", json!({ "url": input.primary_url })),
            ctx.schedule_async_raw("fetch-data", json!({ "url": input.fallback_url })),
        ];

        // Race them - first to complete wins
        let (winner_index, response) = select(futures).await?;

        Ok(RacingOutput {
            winner_index,
            response,
        })
    }
}

// ============================================================================
// Timeout Pattern
// ============================================================================

/// Input for the timeout workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutInput {
    /// The operation to perform
    pub operation: String,
    /// Timeout in seconds
    pub timeout_seconds: u64,
}

/// Output from the timeout workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutOutput {
    /// Whether the operation completed in time
    pub completed: bool,
    /// The result (if completed)
    pub result: Option<Value>,
}

/// Demonstrates the timeout pattern.
///
/// This workflow:
/// 1. Starts a potentially slow operation
/// 2. Sets up a timeout timer
/// 3. Returns the result if it completes in time, or a timeout error
///
/// This is useful for:
/// - SLA enforcement
/// - Graceful degradation
/// - Preventing indefinite waits
pub struct TimeoutWorkflow;

#[async_trait]
impl WorkflowDefinition for TimeoutWorkflow {
    type Input = TimeoutInput;
    type Output = TimeoutOutput;

    fn kind(&self) -> &str {
        "timeout-workflow"
    }

    fn name(&self) -> &str {
        "Timeout Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates adding timeouts to operations with with_timeout combinator")
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "parallel".to_string(),
            "timeout".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        // Create the operation and timer futures
        let operation = ctx.schedule_async_raw("slow-operation", json!({ "op": input.operation }));
        let timer = ctx.sleep_async(Duration::from_secs(input.timeout_seconds));

        // Run with timeout
        match with_timeout(operation, timer).await {
            Ok(result) => Ok(TimeoutOutput {
                completed: true,
                result: Some(result),
            }),
            Err(FlovynError::Timeout(_)) => Ok(TimeoutOutput {
                completed: false,
                result: None,
            }),
            Err(e) => Err(e),
        }
    }
}

// ============================================================================
// Batch with Concurrency Limit Pattern
// ============================================================================

/// Input for the batch processing workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchInput {
    /// Items to process
    pub items: Vec<String>,
    /// Maximum concurrent operations
    pub max_concurrency: usize,
}

/// Output from the batch processing workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOutput {
    /// Number of successful items
    pub successful: usize,
    /// Number of failed items
    pub failed: usize,
    /// Processing results
    pub results: Vec<BatchItemResult>,
}

/// Result for a single batch item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchItemResult {
    pub item: String,
    pub success: bool,
    pub result: Option<Value>,
}

/// Demonstrates batch processing with concurrency limits.
///
/// This workflow:
/// 1. Has a large number of items to process
/// 2. Processes them in batches of N at a time
/// 3. Waits for N to complete before starting more
///
/// This is useful for:
/// - Rate limiting
/// - Resource management
/// - Preventing system overload
pub struct BatchWithConcurrencyWorkflow;

#[async_trait]
impl WorkflowDefinition for BatchWithConcurrencyWorkflow {
    type Input = BatchInput;
    type Output = BatchOutput;

    fn kind(&self) -> &str {
        "batch-with-concurrency-workflow"
    }

    fn name(&self) -> &str {
        "Batch with Concurrency Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates batch processing with controlled parallelism")
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "parallel".to_string(),
            "batch".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let mut results = Vec::new();
        let mut successful = 0;
        let mut failed = 0;

        // Process items in batches
        for chunk in input.items.chunks(input.max_concurrency) {
            // Schedule all items in this batch
            let futures: Vec<_> = chunk
                .iter()
                .map(|item| ctx.schedule_async_raw("process-item", json!({ "item": item })))
                .collect();

            // Wait for all in this batch to complete
            let batch_results = join_all(futures).await;

            // Process results
            match batch_results {
                Ok(values) => {
                    for (i, value) in values.into_iter().enumerate() {
                        successful += 1;
                        results.push(BatchItemResult {
                            item: chunk[i].clone(),
                            success: true,
                            result: Some(value),
                        });
                    }
                }
                Err(e) => {
                    // If join_all fails, one of the tasks failed
                    // In a real implementation, you might want to handle partial failures
                    failed += chunk.len();
                    for item in chunk {
                        results.push(BatchItemResult {
                            item: item.clone(),
                            success: false,
                            result: None,
                        });
                    }
                    tracing::warn!("Batch failed: {:?}", e);
                }
            }
        }

        Ok(BatchOutput {
            successful,
            failed,
            results,
        })
    }
}

// ============================================================================
// Partial Completion Pattern
// ============================================================================

/// Input for the partial completion workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialCompletionInput {
    /// Operations to run
    pub operations: Vec<String>,
    /// Minimum number of successful operations required
    pub min_required: usize,
}

/// Output from the partial completion workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialCompletionOutput {
    /// Results from the completed operations
    pub results: Vec<(usize, Value)>,
    /// Whether we got enough results
    pub sufficient: bool,
}

/// Demonstrates the partial completion pattern using join_n.
///
/// This workflow:
/// 1. Starts multiple operations in parallel
/// 2. Waits for N of them to complete (not all)
/// 3. Returns early once enough have succeeded
///
/// Use cases:
/// - Quorum operations (wait for majority)
/// - Best-effort processing
/// - Performance optimization (don't wait for stragglers)
pub struct PartialCompletionWorkflow;

#[async_trait]
impl WorkflowDefinition for PartialCompletionWorkflow {
    type Input = PartialCompletionInput;
    type Output = PartialCompletionOutput;

    fn kind(&self) -> &str {
        "partial-completion-workflow"
    }

    fn name(&self) -> &str {
        "Partial Completion Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates waiting for N of M operations with join_n combinator")
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "parallel".to_string(),
            "quorum".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        // Start all operations
        let futures: Vec<_> = input
            .operations
            .iter()
            .map(|op| ctx.schedule_async_raw("run-operation", json!({ "operation": op })))
            .collect();

        // Wait for N to complete
        let completed = join_n(futures, input.min_required).await?;

        Ok(PartialCompletionOutput {
            results: completed,
            sufficient: true,
        })
    }
}

// ============================================================================
// Dynamic Parallelism Pattern
// ============================================================================

/// Input for the dynamic parallelism workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicParallelismInput {
    /// Data source to fetch items from
    pub source: String,
}

/// Output from the dynamic parallelism workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicParallelismOutput {
    /// Number of items processed
    pub items_processed: usize,
    /// Processing results
    pub results: Vec<Value>,
}

/// Demonstrates dynamic parallelism where the number of operations is determined at runtime.
///
/// This workflow:
/// 1. Fetches a list of items from an external source (unknown count)
/// 2. Dynamically creates parallel operations based on the fetched data
/// 3. Waits for all operations to complete
///
/// Unlike the batch pattern which has a fixed input list, this pattern:
/// - Discovers what to process at runtime
/// - The degree of parallelism is data-driven
/// - Works with FuturesUnordered-style streaming results
///
/// Use cases:
/// - Processing paginated API results
/// - Fan-out to dynamically discovered workers
/// - Data-driven parallel processing
pub struct DynamicParallelismWorkflow;

#[async_trait]
impl WorkflowDefinition for DynamicParallelismWorkflow {
    type Input = DynamicParallelismInput;
    type Output = DynamicParallelismOutput;

    fn kind(&self) -> &str {
        "dynamic-parallelism-workflow"
    }

    fn name(&self) -> &str {
        "Dynamic Parallelism Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates runtime-determined parallel execution")
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "parallel".to_string(),
            "dynamic".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        // Step 1: Fetch the list of items to process (count unknown until runtime)
        let items_result = ctx
            .schedule_raw("fetch-items", json!({ "source": input.source }))
            .await?;

        // Parse the dynamic list of items
        let items: Vec<String> = items_result
            .get("items")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        if items.is_empty() {
            return Ok(DynamicParallelismOutput {
                items_processed: 0,
                results: vec![],
            });
        }

        // Step 2: Dynamically create parallel futures based on fetched data
        // The number of parallel operations is determined by the data
        let futures: Vec<_> = items
            .iter()
            .map(|item| ctx.schedule_async_raw("process-item", json!({ "item": item })))
            .collect();

        let items_count = futures.len();

        // Step 3: Wait for all dynamically created operations
        let results = join_all(futures).await?;

        Ok(DynamicParallelismOutput {
            items_processed: items_count,
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_fan_out_fan_in_input_serialization() {
        let input = FanOutFanInInput {
            items: vec!["a".to_string(), "b".to_string()],
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["items"], json!(["a", "b"]));
    }

    #[test]
    fn test_racing_input_serialization() {
        let input = RacingInput {
            primary_url: "http://primary".to_string(),
            fallback_url: "http://fallback".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["primary_url"], "http://primary");
        assert_eq!(json["fallback_url"], "http://fallback");
    }

    #[test]
    fn test_timeout_input_serialization() {
        let input = TimeoutInput {
            operation: "slow-op".to_string(),
            timeout_seconds: 30,
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["operation"], "slow-op");
        assert_eq!(json["timeout_seconds"], 30);
    }

    #[test]
    fn test_batch_input_serialization() {
        let input = BatchInput {
            items: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            max_concurrency: 2,
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["items"], json!(["x", "y", "z"]));
        assert_eq!(json["max_concurrency"], 2);
    }

    #[test]
    fn test_partial_completion_input_serialization() {
        let input = PartialCompletionInput {
            operations: vec!["op1".to_string(), "op2".to_string(), "op3".to_string()],
            min_required: 2,
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["operations"], json!(["op1", "op2", "op3"]));
        assert_eq!(json["min_required"], 2);
    }

    #[test]
    fn test_dynamic_parallelism_input_serialization() {
        let input = DynamicParallelismInput {
            source: "database".to_string(),
        };
        let json = serde_json::to_value(&input).unwrap();
        assert_eq!(json["source"], "database");
    }

    #[test]
    fn test_dynamic_parallelism_output_serialization() {
        let output = DynamicParallelismOutput {
            items_processed: 5,
            results: vec![json!({"processed": true}), json!({"processed": true})],
        };
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["items_processed"], 5);
        assert_eq!(json["results"].as_array().unwrap().len(), 2);
    }
}
