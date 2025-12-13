//! Data Transformation Task
//!
//! Simulates transforming data according to specified rules.

use crate::models::*;
use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use tracing::info;

/// Task that transforms data
pub struct TransformationTask;

#[async_trait]
impl TaskDefinition for TransformationTask {
    type Input = TransformationTaskInput;
    type Output = TransformationResult;

    fn kind(&self) -> &str {
        "data-transformation-task"
    }

    fn name(&self) -> &str {
        "Data Transformation"
    }

    fn description(&self) -> Option<&str> {
        Some("Transforms data according to specified transformation type")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(180) // 3 minute timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let start_time = std::time::Instant::now();

        use crate::models::TransformationType;

        info!(
            pipeline_id = %input.pipeline_id,
            transformation = %input.transformation_type,
            records = input.record_count,
            "Starting transformation"
        );

        // Simulate transformation progress
        let transformation_desc = match input.transformation_type {
            TransformationType::Normalize => "Normalizing values",
            TransformationType::Deduplicate => "Deduplicating records",
            TransformationType::Enrich => "Enriching data",
            TransformationType::Aggregate => "Aggregating values",
            TransformationType::Filter => "Filtering records",
        };

        ctx.report_progress(0.2, Some("Loading data")).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(0.5, Some(transformation_desc)).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        ctx.report_progress(0.8, Some("Writing results")).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(1.0, Some("Transformation complete"))
            .await?;

        let duration_ms = start_time.elapsed().as_millis() as u64;
        let task_id = format!("xform-{}", uuid::Uuid::new_v4());

        info!(
            pipeline_id = %input.pipeline_id,
            task_id = %task_id,
            transformation = %input.transformation_type,
            records = input.record_count,
            duration_ms = duration_ms,
            "Transformation completed"
        );

        Ok(TransformationResult {
            task_id,
            status: TaskStatus::Completed,
            transformation_type: input.transformation_type,
            records_transformed: input.record_count,
            duration_ms,
            failure_reason: None,
        })
    }
}
