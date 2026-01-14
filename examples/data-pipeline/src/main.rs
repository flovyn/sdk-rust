//! Data Pipeline Sample
//!
//! This sample demonstrates the DAG (Directed Acyclic Graph) pattern for ETL processing.
//! It shows how to:
//!
//! 1. Execute sequential tasks (ingestion → validation → aggregation)
//! 2. Run parallel transformations
//! 3. Track progress through a multi-step pipeline
//! 4. Handle failures at each step

pub mod models;
pub mod tasks;
pub mod workflows;

use flovyn_worker_sdk::prelude::*;
use tasks::{AggregationTask, IngestionTask, TransformationTask, ValidationTask};
use tracing::info;
use workflows::DataPipelineWorkflow;

/// Parse a server URL into host and port components
fn parse_server_url(url: &str) -> (String, u16) {
    let url = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let mut parts = url.split(':');
    let host = parts.next().unwrap_or("localhost").to_string();
    let port = parts.next().and_then(|p| p.parse().ok()).unwrap_or(9090);
    (host, port)
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from examples/.env
    dotenvy::from_filename("examples/.env").ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("data_pipeline_sample=info".parse()?)
                .add_directive("flovyn_sdk=info".parse()?),
        )
        .init();

    info!("Starting Data Pipeline Sample");

    // Parse configuration from environment
    let org_id = std::env::var("FLOVYN_ORG_ID")
        .ok()
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .unwrap_or_else(uuid::Uuid::new_v4);

    let server_url = std::env::var("FLOVYN_GRPC_SERVER_URL")
        .unwrap_or_else(|_| "http://localhost:9090".to_string());
    let (server_host, server_port) = parse_server_url(&server_url);
    let worker_token = std::env::var("FLOVYN_WORKER_TOKEN")
        .expect("FLOVYN_WORKER_TOKEN environment variable is required");
    let queue = std::env::var("FLOVYN_QUEUE").unwrap_or_else(|_| "default".to_string());

    info!(
        org_id = %org_id,
        server = %format!("{}:{}", server_host, server_port),
        queue = %queue,
        "Connecting to Flovyn server"
    );

    // Build the client with fluent registration
    let client = FlovynClient::builder()
        .server_address(&server_host, server_port)
        .org_id(org_id)
        .worker_token(worker_token)
        .queue(&queue)
        .max_concurrent_workflows(5)
        .max_concurrent_tasks(20)
        .register_workflow(DataPipelineWorkflow)
        .register_task(IngestionTask)
        .register_task(ValidationTask)
        .register_task(TransformationTask)
        .register_task(AggregationTask)
        .build()
        .await?;

    info!(
        workflows = ?["data-pipeline-dag"],
        tasks = ?[
            "data-ingestion-task",
            "data-validation-task",
            "data-transformation-task",
            "data-aggregation-task"
        ],
        "Registered workflows and tasks"
    );

    // Start the workers
    let handle = client.start().await?;

    info!("Workers started. Press Ctrl+C to stop.");
    info!("");
    info!("To test, start a pipeline with:");
    info!("  curl -X POST http://localhost:8080/api/workflows/data-pipeline-dag \\");
    info!("    -H 'Content-Type: application/json' \\");
    info!("    -d '{{");
    info!("      \"pipeline_id\": \"PIPE-001\",");
    info!("      \"data_source_url\": \"s3://my-bucket/raw-data/\",");
    info!("      \"data_format\": \"csv\",");
    info!("      \"transformations\": [\"normalize\", \"dedupe\", \"enrich\"],");
    info!("      \"output_destination\": \"s3://my-bucket/processed-data/\"");
    info!("    }}'");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    info!("Shutting down...");
    handle.stop().await;

    info!("Goodbye!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::models::*;

    #[test]
    fn test_create_sample_pipeline() {
        let pipeline = DataPipelineInput {
            pipeline_id: "PIPE-001".to_string(),
            data_source_url: "s3://bucket/data/".to_string(),
            data_format: DataFormat::Parquet,
            transformations: vec![
                TransformationType::Normalize,
                TransformationType::Deduplicate,
                TransformationType::Aggregate,
            ],
            output_destination: "s3://bucket/output/".to_string(),
        };

        assert_eq!(pipeline.pipeline_id, "PIPE-001");
        assert_eq!(pipeline.transformations.len(), 3);
    }
}
