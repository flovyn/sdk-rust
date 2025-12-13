//! Child Workflow Orchestration
//!
//! Demonstrates parent-child workflow relationships.
//! This pattern is useful for:
//! - Breaking down complex workflows into reusable components
//! - Fan-out/fan-in processing patterns
//! - Parallel processing of independent items

use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::info;

/// Input for batch processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchInput {
    pub batch_id: String,
    pub items: Vec<String>,
}

/// Output from batch processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOutput {
    pub batch_id: String,
    pub total_items: usize,
    pub successful: usize,
    pub failed: usize,
    pub results: Vec<ItemResult>,
}

/// Input for processing a single item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemInput {
    pub item: String,
}

/// Result from processing a single item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemResult {
    pub item: String,
    pub success: bool,
    pub processed_at: i64,
    pub message: String,
}

/// Parent workflow that orchestrates child workflows
///
/// This workflow demonstrates the fan-out/fan-in pattern:
/// 1. Fan-out: Schedule a child workflow for each item in the batch
/// 2. Fan-in: Collect and aggregate results from all children
pub struct BatchProcessingWorkflow;

#[async_trait]
impl WorkflowDefinition for BatchProcessingWorkflow {
    type Input = BatchInput;
    type Output = BatchOutput;

    fn kind(&self) -> &str {
        "batch-processing-workflow"
    }

    fn name(&self) -> &str {
        "Batch Processing Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates fan-out/fan-in pattern with child workflows")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(3600) // 1 hour
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "batch".to_string(),
            "child-workflow".to_string(),
            "fan-out".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            batch_id = %input.batch_id,
            items = input.items.len(),
            "Batch processing workflow started"
        );

        ctx.set_raw("status", serde_json::to_value("PROCESSING")?)
            .await?;
        ctx.set_raw("batchId", serde_json::to_value(&input.batch_id)?)
            .await?;
        ctx.set_raw("totalItems", serde_json::to_value(input.items.len())?)
            .await?;

        let mut results = Vec::new();

        // Fan-out: Process each item through a child workflow
        for (idx, item) in input.items.iter().enumerate() {
            // Check for cancellation between items
            ctx.check_cancellation().await?;

            info!(
                batch_id = %input.batch_id,
                item = %item,
                index = idx,
                "Scheduling child workflow"
            );

            // Schedule child workflow
            let child_input = ItemInput { item: item.clone() };
            let child_input_value = serde_json::to_value(&child_input)?;

            let child_result_value = ctx
                .schedule_workflow_raw(
                    &format!("process-item-{}", idx),
                    "item-processor-workflow",
                    child_input_value,
                )
                .await?;

            let result: ItemResult = serde_json::from_value(child_result_value)?;
            results.push(result);

            // Update progress in state
            ctx.set_raw("processedItems", serde_json::to_value(idx + 1)?)
                .await?;
        }

        // Aggregate results
        let successful = results.iter().filter(|r| r.success).count();
        let failed = results.len() - successful;

        ctx.set_raw("status", serde_json::to_value("COMPLETED")?)
            .await?;

        info!(
            batch_id = %input.batch_id,
            total = input.items.len(),
            successful = successful,
            failed = failed,
            "Batch processing completed"
        );

        Ok(BatchOutput {
            batch_id: input.batch_id,
            total_items: input.items.len(),
            successful,
            failed,
            results,
        })
    }
}

/// Child workflow that processes a single item
pub struct ItemProcessorWorkflow;

#[async_trait]
impl WorkflowDefinition for ItemProcessorWorkflow {
    type Input = ItemInput;
    type Output = ItemResult;

    fn kind(&self) -> &str {
        "item-processor-workflow"
    }

    fn name(&self) -> &str {
        "Item Processor Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Processes a single item - used as child workflow")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(60) // 1 minute per item
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(item = %input.item, "Processing item");

        // Simulate item processing
        let process_result = serde_json::to_value(format!("Processing: {}", input.item))?;
        let _ = ctx.run_raw("process-item", process_result).await?;

        // Simulate some processing time
        ctx.sleep(std::time::Duration::from_millis(100)).await?;

        let processed_at = ctx.current_time_millis();

        info!(
            item = %input.item,
            processed_at = processed_at,
            "Item processed successfully"
        );

        Ok(ItemResult {
            item: input.item,
            success: true,
            processed_at,
            message: "Processed successfully".to_string(),
        })
    }
}

/// Input for parallel child workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelBatchInput {
    pub batch_id: String,
    pub items: Vec<String>,
    pub max_parallel: usize,
}

/// Parent workflow demonstrating controlled parallelism
///
/// This workflow shows how to control the degree of parallelism
/// when processing items through child workflows.
pub struct ControlledParallelWorkflow;

#[async_trait]
impl WorkflowDefinition for ControlledParallelWorkflow {
    type Input = ParallelBatchInput;
    type Output = BatchOutput;

    fn kind(&self) -> &str {
        "controlled-parallel-workflow"
    }

    fn name(&self) -> &str {
        "Controlled Parallel Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates controlled parallelism with child workflows")
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            batch_id = %input.batch_id,
            items = input.items.len(),
            max_parallel = input.max_parallel,
            "Controlled parallel workflow started"
        );

        let mut results = Vec::new();

        // Process items in batches based on max_parallel
        for (batch_idx, chunk) in input.items.chunks(input.max_parallel).enumerate() {
            ctx.check_cancellation().await?;

            info!(
                batch_id = %input.batch_id,
                parallel_batch = batch_idx,
                items = chunk.len(),
                "Processing parallel batch"
            );

            // In a real implementation with schedule_async/Deferred,
            // these would be scheduled in parallel and awaited together.
            // For now, we process sequentially.
            for (idx, item) in chunk.iter().enumerate() {
                let global_idx = batch_idx * input.max_parallel + idx;

                let child_input = ItemInput { item: item.clone() };
                let child_input_value = serde_json::to_value(&child_input)?;

                let result_value = ctx
                    .schedule_workflow_raw(
                        &format!("process-item-{}", global_idx),
                        "item-processor-workflow",
                        child_input_value,
                    )
                    .await?;

                let result: ItemResult = serde_json::from_value(result_value)?;
                results.push(result);
            }
        }

        let successful = results.iter().filter(|r| r.success).count();
        let failed = results.len() - successful;

        info!(
            batch_id = %input.batch_id,
            successful = successful,
            failed = failed,
            "Controlled parallel workflow completed"
        );

        Ok(BatchOutput {
            batch_id: input.batch_id,
            total_items: input.items.len(),
            successful,
            failed,
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_processing_workflow_kind() {
        let workflow = BatchProcessingWorkflow;
        assert_eq!(workflow.kind(), "batch-processing-workflow");
    }

    #[test]
    fn test_item_processor_workflow_kind() {
        let workflow = ItemProcessorWorkflow;
        assert_eq!(workflow.kind(), "item-processor-workflow");
    }

    #[test]
    fn test_batch_input_serialization() {
        let input = BatchInput {
            batch_id: "BATCH-001".to_string(),
            items: vec!["item1".to_string(), "item2".to_string()],
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("BATCH-001"));
        assert!(json.contains("item1"));
    }
}
