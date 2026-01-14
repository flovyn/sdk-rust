//! Data Pipeline domain models
//!
//! This module contains all the data types used in the ETL pipeline workflow.

use flovyn_worker_sdk::prelude::*;
use std::fmt;

// =============================================================================
// Pipeline Types
// =============================================================================

/// Input for the data pipeline workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataPipelineInput {
    pub pipeline_id: String,
    pub data_source_url: String,
    #[serde(default, deserialize_with = "deserialize_data_format")]
    pub data_format: DataFormat,
    pub transformations: Vec<TransformationType>,
    pub output_destination: String,
}

/// Deserialize DataFormat, treating null as the default (Json)
fn deserialize_data_format<'de, D>(deserializer: D) -> std::result::Result<DataFormat, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Option::<DataFormat>::deserialize(deserializer).map(|opt| opt.unwrap_or_default())
}

/// Supported data formats (defaults to JSON)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum DataFormat {
    Csv,
    #[default]
    Json,
    Parquet,
    Avro,
}

impl fmt::Display for DataFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataFormat::Csv => write!(f, "csv"),
            DataFormat::Json => write!(f, "json"),
            DataFormat::Parquet => write!(f, "parquet"),
            DataFormat::Avro => write!(f, "avro"),
        }
    }
}

/// Available transformation types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransformationType {
    Normalize,
    Deduplicate,
    Filter,
    Enrich,
    Aggregate,
}

impl fmt::Display for TransformationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransformationType::Normalize => write!(f, "normalize"),
            TransformationType::Deduplicate => write!(f, "deduplicate"),
            TransformationType::Filter => write!(f, "filter"),
            TransformationType::Enrich => write!(f, "enrich"),
            TransformationType::Aggregate => write!(f, "aggregate"),
        }
    }
}

/// Output from the data pipeline workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataPipelineOutput {
    pub pipeline_id: String,
    pub status: PipelineStatus,
    pub ingestion_task_id: Option<String>,
    pub validation_task_id: Option<String>,
    pub transformation_task_ids: Vec<String>,
    pub aggregation_task_id: Option<String>,
    pub records_processed: Option<u64>,
    pub execution_time_ms: Option<u64>,
    pub message: String,
}

impl DataPipelineOutput {
    /// Create a failed pipeline output
    pub fn failed(pipeline_id: &str, message: &str) -> Self {
        Self {
            pipeline_id: pipeline_id.to_string(),
            status: PipelineStatus::Failed,
            ingestion_task_id: None,
            validation_task_id: None,
            transformation_task_ids: vec![],
            aggregation_task_id: None,
            records_processed: None,
            execution_time_ms: None,
            message: message.to_string(),
        }
    }
}

/// Pipeline processing status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PipelineStatus {
    Created,
    IngestingData,
    DataIngested,
    ValidatingData,
    DataValidated,
    TransformingData,
    TransformationsCompleted,
    AggregatingResults,
    Completed,
    Failed,
}

/// Generic task completion status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TaskStatus {
    Completed,
    Failed,
}

// =============================================================================
// Ingestion Task Types
// =============================================================================

/// Input for the ingestion task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IngestionTaskInput {
    pub pipeline_id: String,
    pub data_source_url: String,
    #[serde(default, deserialize_with = "deserialize_data_format")]
    pub data_format: DataFormat,
}

/// Result from the ingestion task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IngestionResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub records_ingested: u64,
    pub bytes_read: u64,
    pub duration_ms: u64,
    pub failure_reason: Option<String>,
}

// =============================================================================
// Validation Task Types
// =============================================================================

/// Input for the validation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidationTaskInput {
    pub pipeline_id: String,
    pub record_count: u64,
}

/// Result from the validation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidationResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub valid_records: u64,
    pub invalid_records: u64,
    pub validation_errors: Vec<String>,
    pub failure_reason: Option<String>,
}

// =============================================================================
// Transformation Task Types
// =============================================================================

/// Input for the transformation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TransformationTaskInput {
    pub pipeline_id: String,
    pub transformation_type: TransformationType,
    pub record_count: u64,
}

/// Result from the transformation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TransformationResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub transformation_type: TransformationType,
    pub records_transformed: u64,
    pub duration_ms: u64,
    pub failure_reason: Option<String>,
}

// =============================================================================
// Aggregation Task Types
// =============================================================================

/// Input for the aggregation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AggregationTaskInput {
    pub pipeline_id: String,
    pub transformation_task_ids: Vec<String>,
    pub output_destination: String,
}

/// Result from the aggregation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AggregationResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub records_aggregated: u64,
    pub bytes_written: u64,
    pub output_location: String,
    pub failure_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_input_serialization() {
        let input = DataPipelineInput {
            pipeline_id: "PIPE-001".to_string(),
            data_source_url: "s3://bucket/data.csv".to_string(),
            data_format: DataFormat::Csv,
            transformations: vec![
                TransformationType::Normalize,
                TransformationType::Deduplicate,
            ],
            output_destination: "s3://bucket/output/".to_string(),
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: DataPipelineInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pipeline_id, input.pipeline_id);
    }

    #[test]
    fn test_pipeline_status_serialization() {
        let status = PipelineStatus::DataValidated;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"DATA_VALIDATED\"");
    }

    #[test]
    fn test_pipeline_output_failed() {
        let output = DataPipelineOutput::failed("PIPE-001", "Connection timeout");
        assert_eq!(output.status, PipelineStatus::Failed);
        assert_eq!(output.message, "Connection timeout");
    }
}
