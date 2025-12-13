//! Data Ingestion Task
//!
//! Simulates loading data from a source.

use crate::models::*;
use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use tracing::info;

/// Task that ingests data from a source
pub struct IngestionTask;

#[async_trait]
impl TaskDefinition for IngestionTask {
    type Input = IngestionTaskInput;
    type Output = IngestionResult;

    fn kind(&self) -> &str {
        "data-ingestion-task"
    }

    fn name(&self) -> &str {
        "Data Ingestion"
    }

    fn description(&self) -> Option<&str> {
        Some("Ingests data from a source location")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(300) // 5 minute timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let start_time = std::time::Instant::now();

        info!(
            pipeline_id = %input.pipeline_id,
            source = %input.data_source_url,
            format = %input.data_format,
            "Starting data ingestion"
        );

        // Simulate ingestion progress
        ctx.report_progress(0.1, Some("Connecting to data source"))
            .await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        ctx.report_progress(0.3, Some("Reading data")).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        ctx.report_progress(0.6, Some("Parsing records")).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        ctx.report_progress(0.9, Some("Staging data")).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(1.0, Some("Ingestion complete")).await?;

        // Simulate ingested record count (10k-100k records)
        let records_ingested = 10_000 + (rand::random::<u64>() % 90_000);
        let bytes_read = records_ingested * 256; // ~256 bytes per record
        let duration_ms = start_time.elapsed().as_millis() as u64;
        let task_id = format!("ing-{}", uuid::Uuid::new_v4());

        info!(
            pipeline_id = %input.pipeline_id,
            task_id = %task_id,
            records = records_ingested,
            duration_ms = duration_ms,
            "Data ingestion completed"
        );

        Ok(IngestionResult {
            task_id,
            status: TaskStatus::Completed,
            records_ingested,
            bytes_read,
            duration_ms,
            failure_reason: None,
        })
    }
}
