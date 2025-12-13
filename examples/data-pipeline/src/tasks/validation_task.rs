//! Data Validation Task
//!
//! Simulates validating ingested data.

use crate::models::*;
use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use tracing::info;

/// Task that validates ingested data
pub struct ValidationTask;

#[async_trait]
impl TaskDefinition for ValidationTask {
    type Input = ValidationTaskInput;
    type Output = ValidationResult;

    fn kind(&self) -> &str {
        "data-validation-task"
    }

    fn name(&self) -> &str {
        "Data Validation"
    }

    fn description(&self) -> Option<&str> {
        Some("Validates ingested data for quality and completeness")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(120) // 2 minute timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        info!(
            pipeline_id = %input.pipeline_id,
            records = input.record_count,
            "Starting data validation"
        );

        // Simulate validation progress
        ctx.report_progress(0.2, Some("Checking schema")).await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        ctx.report_progress(0.5, Some("Validating data types"))
            .await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        ctx.report_progress(0.8, Some("Checking constraints"))
            .await?;
        ctx.check_cancellation().await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(1.0, Some("Validation complete"))
            .await?;

        // Simulate validation results (1-5% invalid records)
        let invalid_rate = (rand::random::<f64>() * 0.04) + 0.01; // 1-5%
        let invalid_records = (input.record_count as f64 * invalid_rate) as u64;
        let valid_records = input.record_count - invalid_records;
        let task_id = format!("val-{}", uuid::Uuid::new_v4());

        info!(
            pipeline_id = %input.pipeline_id,
            task_id = %task_id,
            valid = valid_records,
            invalid = invalid_records,
            "Data validation completed"
        );

        Ok(ValidationResult {
            task_id,
            status: TaskStatus::Completed,
            valid_records,
            invalid_records,
            validation_errors: if invalid_records > 0 {
                vec![format!(
                    "{} records with missing required fields",
                    invalid_records
                )]
            } else {
                vec![]
            },
            failure_reason: None,
        })
    }
}
