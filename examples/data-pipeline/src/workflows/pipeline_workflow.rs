//! Data Pipeline Workflow (DAG Pattern)
//!
//! This workflow demonstrates the DAG (Directed Acyclic Graph) pattern for ETL processing.
//! It processes data through multiple steps:
//!
//! 1. \[Sequential\] Data Ingestion - Load data from source
//! 2. \[Sequential\] Data Validation - Validate loaded records
//! 3. \[Parallel\] Data Transformations - Run multiple transformations concurrently
//! 4. \[Sequential\] Aggregation - Combine and output results
//!
//! The parallel step demonstrates running multiple tasks concurrently.

use crate::models::*;
use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use tracing::{error, info};

/// Data pipeline workflow implementing the DAG pattern
pub struct DataPipelineWorkflow;

#[async_trait]
impl WorkflowDefinition for DataPipelineWorkflow {
    type Input = DataPipelineInput;
    type Output = DataPipelineOutput;

    fn kind(&self) -> &str {
        "data-pipeline-dag"
    }

    fn name(&self) -> &str {
        "Data Pipeline - Multi-Step DAG"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("ETL data pipeline using the DAG pattern with parallel transformations")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(900) // 15 minute timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "etl".to_string(),
            "data-pipeline".to_string(),
            "dag".to_string(),
            "parallel".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let start_time = ctx.current_time_millis();
        info!(pipeline_id = %input.pipeline_id, "Starting data pipeline");

        ctx.set_raw("status", serde_json::to_value(PipelineStatus::Created)?)
            .await?;
        ctx.set_raw("pipelineId", serde_json::to_value(&input.pipeline_id)?)
            .await?;

        // =========================================================================
        // Step 1: [Sequential] Data Ingestion
        // =========================================================================
        info!(
            pipeline_id = %input.pipeline_id,
            source = %input.data_source_url,
            format = %input.data_format,
            "Step 1: Ingesting data"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(PipelineStatus::IngestingData)?,
        )
        .await?;

        let ingestion_input = IngestionTaskInput {
            pipeline_id: input.pipeline_id.clone(),
            data_source_url: input.data_source_url.clone(),
            data_format: input.data_format,
        };
        let ingestion_input_value = serde_json::to_value(&ingestion_input)?;

        let ingestion_result_value = ctx
            .schedule_raw("data-ingestion-task", ingestion_input_value)
            .await?;
        let ingestion_result: IngestionResult = serde_json::from_value(ingestion_result_value)?;

        if ingestion_result.status != TaskStatus::Completed {
            error!(
                pipeline_id = %input.pipeline_id,
                reason = ?ingestion_result.failure_reason,
                "Data ingestion failed"
            );
            return Ok(DataPipelineOutput::failed(
                &input.pipeline_id,
                "Ingestion failed",
            ));
        }

        info!(
            pipeline_id = %input.pipeline_id,
            records = ingestion_result.records_ingested,
            duration_ms = ingestion_result.duration_ms,
            "Data ingestion completed"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(PipelineStatus::DataIngested)?,
        )
        .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        // =========================================================================
        // Step 2: [Sequential] Data Validation
        // =========================================================================
        info!(
            pipeline_id = %input.pipeline_id,
            records = ingestion_result.records_ingested,
            "Step 2: Validating data"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(PipelineStatus::ValidatingData)?,
        )
        .await?;

        let validation_input = ValidationTaskInput {
            pipeline_id: input.pipeline_id.clone(),
            record_count: ingestion_result.records_ingested,
        };
        let validation_input_value = serde_json::to_value(&validation_input)?;

        let validation_result_value = ctx
            .schedule_raw("data-validation-task", validation_input_value)
            .await?;
        let validation_result: ValidationResult = serde_json::from_value(validation_result_value)?;

        if validation_result.status != TaskStatus::Completed {
            error!(
                pipeline_id = %input.pipeline_id,
                reason = ?validation_result.failure_reason,
                "Data validation failed"
            );
            return Ok(DataPipelineOutput::failed(
                &input.pipeline_id,
                "Validation failed",
            ));
        }

        info!(
            pipeline_id = %input.pipeline_id,
            valid = validation_result.valid_records,
            invalid = validation_result.invalid_records,
            "Data validation completed"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(PipelineStatus::DataValidated)?,
        )
        .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        // =========================================================================
        // Step 3: [Parallel] Data Transformations
        // =========================================================================
        info!(
            pipeline_id = %input.pipeline_id,
            transformations = input.transformations.len(),
            "Step 3: Running transformations in parallel"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(PipelineStatus::TransformingData)?,
        )
        .await?;

        let mut transformation_results = Vec::new();
        let mut transformation_task_ids = Vec::new();

        // Execute transformations sequentially (in real implementation, could be parallel)
        // Note: The SDK currently executes these sequentially; true parallelism would
        // require schedule_async/Deferred pattern when implemented
        for transformation_type in input.transformations.iter() {
            ctx.check_cancellation().await?;

            let transform_input = TransformationTaskInput {
                pipeline_id: input.pipeline_id.clone(),
                transformation_type: transformation_type.clone(),
                record_count: validation_result.valid_records,
            };
            let transform_input_value = serde_json::to_value(&transform_input)?;

            let result_value = ctx
                .schedule_raw("data-transformation-task", transform_input_value)
                .await?;
            let result: TransformationResult = serde_json::from_value(result_value)?;

            transformation_task_ids.push(result.task_id.clone());
            transformation_results.push(result);
        }

        // Check for failed transformations
        let failed: Vec<_> = transformation_results
            .iter()
            .filter(|r| r.status != TaskStatus::Completed)
            .collect();

        if !failed.is_empty() {
            error!(
                pipeline_id = %input.pipeline_id,
                failed_count = failed.len(),
                "Some transformations failed"
            );
            return Ok(DataPipelineOutput::failed(
                &input.pipeline_id,
                &format!("{} transformation(s) failed", failed.len()),
            ));
        }

        info!(
            pipeline_id = %input.pipeline_id,
            completed = transformation_results.len(),
            "All transformations completed"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(PipelineStatus::TransformationsCompleted)?,
        )
        .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        // =========================================================================
        // Step 4: [Sequential] Aggregate Results
        // =========================================================================
        info!(
            pipeline_id = %input.pipeline_id,
            destination = %input.output_destination,
            "Step 4: Aggregating results"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(PipelineStatus::AggregatingResults)?,
        )
        .await?;

        let aggregation_input = AggregationTaskInput {
            pipeline_id: input.pipeline_id.clone(),
            transformation_task_ids: transformation_task_ids.clone(),
            output_destination: input.output_destination.clone(),
        };
        let aggregation_input_value = serde_json::to_value(&aggregation_input)?;

        let aggregation_result_value = ctx
            .schedule_raw("data-aggregation-task", aggregation_input_value)
            .await?;
        let aggregation_result: AggregationResult =
            serde_json::from_value(aggregation_result_value)?;

        if aggregation_result.status != TaskStatus::Completed {
            error!(
                pipeline_id = %input.pipeline_id,
                reason = ?aggregation_result.failure_reason,
                "Aggregation failed"
            );
            return Ok(DataPipelineOutput::failed(
                &input.pipeline_id,
                "Aggregation failed",
            ));
        }

        // =========================================================================
        // Complete Pipeline
        // =========================================================================
        let end_time = ctx.current_time_millis();
        let execution_time_ms = (end_time - start_time) as u64;

        ctx.set_raw("status", serde_json::to_value(PipelineStatus::Completed)?)
            .await?;

        info!(
            pipeline_id = %input.pipeline_id,
            records = aggregation_result.records_aggregated,
            output = %aggregation_result.output_location,
            execution_time_ms = execution_time_ms,
            "Pipeline completed successfully"
        );

        Ok(DataPipelineOutput {
            pipeline_id: input.pipeline_id,
            status: PipelineStatus::Completed,
            ingestion_task_id: Some(ingestion_result.task_id),
            validation_task_id: Some(validation_result.task_id),
            transformation_task_ids,
            aggregation_task_id: Some(aggregation_result.task_id),
            records_processed: Some(aggregation_result.records_aggregated),
            execution_time_ms: Some(execution_time_ms),
            message: format!(
                "Pipeline completed successfully. Output: {}",
                aggregation_result.output_location
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_workflow_kind() {
        let workflow = DataPipelineWorkflow;
        assert_eq!(workflow.kind(), "data-pipeline-dag");
    }

    #[test]
    fn test_pipeline_workflow_version() {
        let workflow = DataPipelineWorkflow;
        assert_eq!(workflow.version(), SemanticVersion::new(1, 0, 0));
    }

    #[test]
    fn test_pipeline_workflow_timeout() {
        let workflow = DataPipelineWorkflow;
        assert_eq!(workflow.timeout_seconds(), Some(900));
    }

    #[test]
    fn test_pipeline_workflow_tags() {
        let workflow = DataPipelineWorkflow;
        let tags = workflow.tags();
        assert!(tags.contains(&"etl".to_string()));
        assert!(tags.contains(&"dag".to_string()));
        assert!(tags.contains(&"parallel".to_string()));
    }
}

/// Integration tests using SDK testing utilities
#[cfg(test)]
mod integration_tests {
    use super::*;
    use flovyn_sdk::testing::MockWorkflowContext;
    use serde_json::json;

    fn create_test_pipeline_input() -> DataPipelineInput {
        DataPipelineInput {
            pipeline_id: "PIPE-TEST-001".to_string(),
            data_source_url: "s3://test-bucket/data/".to_string(),
            data_format: DataFormat::Csv,
            transformations: vec![
                TransformationType::Normalize,
                TransformationType::Deduplicate,
            ],
            output_destination: "s3://test-bucket/output/".to_string(),
        }
    }

    #[tokio::test]
    async fn test_pipeline_workflow_successful_execution() {
        let ctx = MockWorkflowContext::builder()
            .initial_time_millis(1700000000000)
            .task_result(
                "data-ingestion-task",
                json!({
                    "task_id": "ing-12345",
                    "status": "Completed",
                    "records_ingested": 50000,
                    "bytes_read": 12800000,
                    "duration_ms": 1500,
                    "failure_reason": null
                }),
            )
            .task_result(
                "data-validation-task",
                json!({
                    "task_id": "val-67890",
                    "status": "Completed",
                    "valid_records": 49500,
                    "invalid_records": 500,
                    "validation_errors": [],
                    "failure_reason": null
                }),
            )
            .task_result(
                "data-transformation-task",
                json!({
                    "task_id": "xform-11111",
                    "status": "Completed",
                    "transformation_type": "normalize",
                    "records_transformed": 49500,
                    "duration_ms": 800,
                    "failure_reason": null
                }),
            )
            .task_result(
                "data-aggregation-task",
                json!({
                    "task_id": "agg-22222",
                    "status": "Completed",
                    "records_aggregated": 49500,
                    "bytes_written": 6336000,
                    "output_location": "s3://test-bucket/output/result.parquet",
                    "failure_reason": null
                }),
            )
            .build();

        let workflow = DataPipelineWorkflow;
        let input = create_test_pipeline_input();
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.status, PipelineStatus::Completed);
        assert_eq!(output.pipeline_id, "PIPE-TEST-001");
        assert!(output.ingestion_task_id.is_some());
        assert!(output.validation_task_id.is_some());
        assert!(!output.transformation_task_ids.is_empty());
        assert!(output.aggregation_task_id.is_some());
        assert!(output.records_processed.is_some());
        assert!(output.execution_time_ms.is_some());

        // Verify all tasks were scheduled
        assert!(ctx.was_task_scheduled("data-ingestion-task"));
        assert!(ctx.was_task_scheduled("data-validation-task"));
        assert!(ctx.was_task_scheduled("data-transformation-task"));
        assert!(ctx.was_task_scheduled("data-aggregation-task"));

        // Verify state tracking
        let state = ctx.state_snapshot();
        assert!(state.contains_key("status"));
        assert!(state.contains_key("pipelineId"));
    }

    #[tokio::test]
    async fn test_pipeline_workflow_ingestion_failure() {
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "data-ingestion-task",
                json!({
                    "task_id": "ing-failed",
                    "status": "Failed",
                    "records_ingested": 0,
                    "bytes_read": 0,
                    "duration_ms": 100,
                    "failure_reason": "Connection timeout"
                }),
            )
            .build();

        let workflow = DataPipelineWorkflow;
        let input = create_test_pipeline_input();
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.status, PipelineStatus::Failed);
        assert!(output.message.contains("Ingestion failed"));

        // Only ingestion task should be scheduled
        assert!(ctx.was_task_scheduled("data-ingestion-task"));
        assert!(!ctx.was_task_scheduled("data-validation-task"));
    }

    #[tokio::test]
    async fn test_pipeline_workflow_validation_failure() {
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "data-ingestion-task",
                json!({
                    "task_id": "ing-ok",
                    "status": "Completed",
                    "records_ingested": 50000,
                    "bytes_read": 12800000,
                    "duration_ms": 1500,
                    "failure_reason": null
                }),
            )
            .task_result(
                "data-validation-task",
                json!({
                    "task_id": "val-failed",
                    "status": "Failed",
                    "valid_records": 0,
                    "invalid_records": 50000,
                    "validation_errors": ["Schema mismatch"],
                    "failure_reason": "All records failed validation"
                }),
            )
            .build();

        let workflow = DataPipelineWorkflow;
        let input = create_test_pipeline_input();
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.status, PipelineStatus::Failed);
        assert!(output.message.contains("Validation failed"));
    }

    #[tokio::test]
    async fn test_pipeline_workflow_with_multiple_transformations() {
        // Test with multiple transformations
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "data-ingestion-task",
                json!({
                    "task_id": "ing-12345",
                    "status": "Completed",
                    "records_ingested": 50000,
                    "bytes_read": 12800000,
                    "duration_ms": 1500,
                    "failure_reason": null
                }),
            )
            .task_result(
                "data-validation-task",
                json!({
                    "task_id": "val-67890",
                    "status": "Completed",
                    "valid_records": 50000,
                    "invalid_records": 0,
                    "validation_errors": [],
                    "failure_reason": null
                }),
            )
            .task_result(
                "data-transformation-task",
                json!({
                    "task_id": "xform-multi",
                    "status": "Completed",
                    "transformation_type": "normalize",
                    "records_transformed": 50000,
                    "duration_ms": 500,
                    "failure_reason": null
                }),
            )
            .task_result(
                "data-aggregation-task",
                json!({
                    "task_id": "agg-final",
                    "status": "Completed",
                    "records_aggregated": 50000,
                    "bytes_written": 6400000,
                    "output_location": "s3://test-bucket/output/final.parquet",
                    "failure_reason": null
                }),
            )
            .build();

        let mut input = create_test_pipeline_input();
        input.transformations = vec![
            TransformationType::Normalize,
            TransformationType::Deduplicate,
            TransformationType::Enrich,
        ];

        let workflow = DataPipelineWorkflow;
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.status, PipelineStatus::Completed);
        // 3 transformations = 3 task IDs
        assert_eq!(output.transformation_task_ids.len(), 3);

        // Verify transformation task was scheduled 3 times
        let scheduled = ctx.scheduled_tasks();
        let xform_count = scheduled
            .iter()
            .filter(|t| t.task_type == "data-transformation-task")
            .count();
        assert_eq!(xform_count, 3);
    }

    #[tokio::test]
    async fn test_pipeline_workflow_cancellation() {
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "data-ingestion-task",
                json!({
                    "task_id": "ing-ok",
                    "status": "Completed",
                    "records_ingested": 50000,
                    "bytes_read": 12800000,
                    "duration_ms": 1500,
                    "failure_reason": null
                }),
            )
            .build();

        // Request cancellation after ingestion
        ctx.request_cancellation();

        let workflow = DataPipelineWorkflow;
        let input = create_test_pipeline_input();
        let result = workflow.execute(&ctx, input).await;

        // Should error due to cancellation check
        assert!(result.is_err());
    }
}
