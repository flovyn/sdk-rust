//! Data Aggregation Task
//!
//! Simulates aggregating transformed data and writing to output.

use crate::models::*;
use async_trait::async_trait;
use flovyn_worker_sdk::prelude::*;
use tracing::info;

/// Task that aggregates transformed data
pub struct AggregationTask;

#[async_trait]
impl TaskDefinition for AggregationTask {
    type Input = AggregationTaskInput;
    type Output = AggregationResult;

    fn kind(&self) -> &str {
        "data-aggregation-task"
    }

    fn name(&self) -> &str {
        "Data Aggregation"
    }

    fn description(&self) -> Option<&str> {
        Some("Aggregates transformed data and writes to output destination")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(180) // 3 minute timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let _start_time = std::time::Instant::now();

        info!(
            pipeline_id = %input.pipeline_id,
            transformations = input.transformation_task_ids.len(),
            destination = %input.output_destination,
            "Starting aggregation"
        );

        // Simulate aggregation progress
        ctx.report_progress(0.2, Some("Reading transformation results"))
            .await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        ctx.report_progress(0.5, Some("Aggregating data")).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        ctx.report_progress(0.8, Some("Writing to output")).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        ctx.report_progress(1.0, Some("Aggregation complete"))
            .await?;

        let task_id = format!("agg-{}", uuid::Uuid::new_v4());
        let records_aggregated = 10_000 + (rand::random::<u64>() % 90_000);
        let bytes_written = records_aggregated * 128; // ~128 bytes per aggregated record

        // Generate output location
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let output_location = format!(
            "{}/pipeline_{}_output_{}.parquet",
            input.output_destination.trim_end_matches('/'),
            input.pipeline_id,
            timestamp
        );

        info!(
            pipeline_id = %input.pipeline_id,
            task_id = %task_id,
            records = records_aggregated,
            output = %output_location,
            "Aggregation completed"
        );

        Ok(AggregationResult {
            task_id,
            status: TaskStatus::Completed,
            records_aggregated,
            bytes_written,
            output_location,
            failure_reason: None,
        })
    }
}
