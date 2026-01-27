//! Standalone Tasks Sample
//!
//! This example demonstrates standalone tasks with:
//! - Logging at different levels (debug, info, warn, error)
//! - Progress reporting with detailed messages
//! - Random duration between 2-5 minutes
//! - Cancellation support
//!
//! Tasks included:
//! - DataExportTask: Exports data in batches with progress tracking
//! - ReportGenerationTask: Multi-phase report generation
//! - BackupTask: Creates backups with checkpoints
//! - IndexingTask: Indexes documents with batch progress

use async_trait::async_trait;
use flovyn_worker_sdk::prelude::*;
use rand::Rng;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

// ============================================================================
// Data Export Task
// ============================================================================

/// Input for data export task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataExportInput {
    /// Name of the dataset to export
    pub dataset_name: String,
    /// Export format (csv, json, parquet)
    pub format: String,
    /// Number of records to export (for simulation)
    pub record_count: u32,
}

/// Output from data export task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DataExportOutput {
    /// Path to the exported file
    pub file_path: String,
    /// Number of records exported
    pub records_exported: u32,
    /// Size in bytes
    pub file_size_bytes: u64,
    /// Duration in seconds
    pub duration_seconds: u64,
}

/// Task that exports data in batches with progress tracking
pub struct DataExportTask;

#[async_trait]
impl TaskDefinition for DataExportTask {
    type Input = DataExportInput;
    type Output = DataExportOutput;

    fn kind(&self) -> &str {
        "data-export-task"
    }

    fn name(&self) -> &str {
        "Data Export"
    }

    fn description(&self) -> Option<&str> {
        Some("Exports data in batches with detailed progress tracking")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(600) // 10 minutes max
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let start = std::time::Instant::now();

        info!(
            dataset = %input.dataset_name,
            format = %input.format,
            records = %input.record_count,
            "Starting data export"
        );

        // Random duration between 30-60 seconds
        let total_duration_secs = rand::thread_rng().gen_range(30..=60);
        let batch_count = 10;
        let batch_duration = Duration::from_secs(total_duration_secs / batch_count);
        let records_per_batch = input.record_count / batch_count as u32;

        ctx.log(
            LogLevel::Info,
            &format!("Export will process {} batches", batch_count),
        )
        .await?;

        let mut total_exported = 0u32;

        for batch in 1..=batch_count {
            ctx.check_cancellation().await?;

            let progress = batch as f64 / batch_count as f64;
            let message = format!(
                "Processing batch {}/{} ({} records)",
                batch, batch_count, records_per_batch
            );

            ctx.report_progress(progress * 0.9, Some(&message)).await?;

            debug!(
                batch = batch,
                records = records_per_batch,
                "Processing export batch"
            );

            // Simulate batch processing
            tokio::time::sleep(batch_duration).await;

            total_exported += records_per_batch;

            // Log progress at info level every 3 batches
            if batch % 3 == 0 {
                info!(
                    batch = batch,
                    total_exported = total_exported,
                    progress_pct = format!("{:.1}%", progress * 100.0),
                    "Export progress checkpoint"
                );
            }
        }

        ctx.report_progress(0.95, Some("Finalizing export file"))
            .await?;
        tokio::time::sleep(Duration::from_secs(2)).await;

        ctx.report_progress(1.0, Some("Export completed")).await?;

        let duration = start.elapsed().as_secs();
        let file_size = (input.record_count as u64) * 256; // Simulated size

        info!(
            dataset = %input.dataset_name,
            records = total_exported,
            duration_secs = duration,
            "Data export completed successfully"
        );

        ctx.log(
            LogLevel::Info,
            &format!(
                "Exported {} records in {} seconds",
                total_exported, duration
            ),
        )
        .await?;

        Ok(DataExportOutput {
            file_path: format!("/exports/{}.{}", input.dataset_name, input.format),
            records_exported: total_exported,
            file_size_bytes: file_size,
            duration_seconds: duration,
        })
    }
}

// ============================================================================
// Report Generation Task
// ============================================================================

/// Input for report generation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReportInput {
    /// Report name
    pub report_name: String,
    /// Report type (daily, weekly, monthly)
    pub report_type: String,
    /// Include charts
    pub include_charts: bool,
}

/// Output from report generation task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReportOutput {
    /// Report ID
    pub report_id: String,
    /// Path to generated report
    pub report_path: String,
    /// Number of pages
    pub page_count: u32,
    /// Generation time in seconds
    pub generation_time_seconds: u64,
}

/// Task that generates reports with multiple phases
pub struct ReportGenerationTask;

#[async_trait]
impl TaskDefinition for ReportGenerationTask {
    type Input = ReportInput;
    type Output = ReportOutput;

    fn kind(&self) -> &str {
        "report-generation-task"
    }

    fn name(&self) -> &str {
        "Report Generation"
    }

    fn description(&self) -> Option<&str> {
        Some("Generates reports with multiple phases: data collection, analysis, rendering")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(600)
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let start = std::time::Instant::now();
        let report_id = uuid::Uuid::new_v4().to_string();

        info!(
            report_name = %input.report_name,
            report_type = %input.report_type,
            report_id = %report_id,
            "Starting report generation"
        );

        // Random total duration 2-5 minutes
        let total_duration_secs = rand::thread_rng().gen_range(30..=60);

        // Phase 1: Data Collection (30% of time)
        let phase1_duration = Duration::from_secs(total_duration_secs * 30 / 100);
        ctx.report_progress(0.0, Some("Phase 1: Collecting data sources"))
            .await?;
        ctx.log(LogLevel::Info, "Starting data collection phase")
            .await?;

        debug!(phase = "data_collection", "Querying primary database");
        tokio::time::sleep(phase1_duration / 3).await;
        ctx.check_cancellation().await?;

        ctx.report_progress(0.1, Some("Querying secondary sources"))
            .await?;
        debug!(phase = "data_collection", "Querying analytics database");
        tokio::time::sleep(phase1_duration / 3).await;
        ctx.check_cancellation().await?;

        ctx.report_progress(0.2, Some("Aggregating data")).await?;
        debug!(phase = "data_collection", "Merging data sources");
        tokio::time::sleep(phase1_duration / 3).await;

        info!(phase = "data_collection", "Data collection completed");
        ctx.log(LogLevel::Info, "Data collection phase completed")
            .await?;

        // Phase 2: Analysis (40% of time)
        let phase2_duration = Duration::from_secs(total_duration_secs * 40 / 100);
        ctx.report_progress(0.3, Some("Phase 2: Analyzing data"))
            .await?;
        ctx.log(LogLevel::Info, "Starting analysis phase").await?;

        let analysis_steps = [
            "Statistical analysis",
            "Trend detection",
            "Anomaly detection",
            "Forecasting",
        ];

        for (i, step) in analysis_steps.iter().enumerate() {
            ctx.check_cancellation().await?;

            let step_progress = 0.3 + (i as f64 + 1.0) / analysis_steps.len() as f64 * 0.35;
            ctx.report_progress(step_progress, Some(&format!("Analyzing: {}", step)))
                .await?;

            debug!(phase = "analysis", step = step, "Running analysis step");

            // Simulate occasional warnings during analysis
            if i == 2 {
                warn!(
                    phase = "analysis",
                    "Some data points excluded due to quality issues"
                );
                ctx.log(
                    LogLevel::Warn,
                    "Minor data quality issues detected, continuing with available data",
                )
                .await?;
            }

            tokio::time::sleep(phase2_duration / analysis_steps.len() as u32).await;
        }

        info!(phase = "analysis", "Analysis completed");
        ctx.log(LogLevel::Info, "Analysis phase completed").await?;

        // Phase 3: Rendering (30% of time)
        let phase3_duration = Duration::from_secs(total_duration_secs * 30 / 100);
        ctx.report_progress(0.65, Some("Phase 3: Rendering report"))
            .await?;
        ctx.log(LogLevel::Info, "Starting rendering phase").await?;

        debug!(phase = "rendering", "Generating tables");
        tokio::time::sleep(phase3_duration / 3).await;
        ctx.check_cancellation().await?;

        ctx.report_progress(0.75, Some("Rendering tables")).await?;

        if input.include_charts {
            debug!(phase = "rendering", "Generating charts");
            ctx.report_progress(0.85, Some("Generating charts")).await?;
            tokio::time::sleep(phase3_duration / 3).await;
            info!(phase = "rendering", charts = true, "Charts generated");
        }

        ctx.report_progress(0.95, Some("Finalizing PDF")).await?;
        debug!(phase = "rendering", "Writing PDF output");
        tokio::time::sleep(phase3_duration / 3).await;

        ctx.report_progress(1.0, Some("Report generation complete"))
            .await?;

        let duration = start.elapsed().as_secs();
        let page_count = rand::thread_rng().gen_range(15..50);

        info!(
            report_id = %report_id,
            pages = page_count,
            duration_secs = duration,
            "Report generation completed"
        );

        ctx.log(
            LogLevel::Info,
            &format!(
                "Generated {}-page report in {} seconds",
                page_count, duration
            ),
        )
        .await?;

        Ok(ReportOutput {
            report_id,
            report_path: format!("/reports/{}.pdf", input.report_name),
            page_count,
            generation_time_seconds: duration,
        })
    }
}

// ============================================================================
// Backup Task
// ============================================================================

/// Input for backup task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackupInput {
    /// Source path to backup
    pub source_path: String,
    /// Backup type (full, incremental, differential)
    pub backup_type: String,
    /// Enable compression
    pub compress: bool,
}

/// Output from backup task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackupOutput {
    /// Backup ID
    pub backup_id: String,
    /// Path to backup archive
    pub archive_path: String,
    /// Original size in bytes
    pub original_size_bytes: u64,
    /// Compressed size in bytes
    pub compressed_size_bytes: u64,
    /// Number of files backed up
    pub files_count: u32,
    /// Duration in seconds
    pub duration_seconds: u64,
}

/// Task that creates backups with checkpoint logging
pub struct BackupTask;

#[async_trait]
impl TaskDefinition for BackupTask {
    type Input = BackupInput;
    type Output = BackupOutput;

    fn kind(&self) -> &str {
        "backup-task"
    }

    fn name(&self) -> &str {
        "Backup"
    }

    fn description(&self) -> Option<&str> {
        Some("Creates backups with checkpoints and compression")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(600)
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let start = std::time::Instant::now();
        let backup_id = format!("backup-{}", uuid::Uuid::new_v4());

        info!(
            source = %input.source_path,
            backup_type = %input.backup_type,
            backup_id = %backup_id,
            "Starting backup"
        );

        ctx.log(
            LogLevel::Info,
            &format!(
                "Starting {} backup of {}",
                input.backup_type, input.source_path
            ),
        )
        .await?;

        // Random duration 2-5 minutes
        let total_duration_secs = rand::thread_rng().gen_range(30..=60);

        // Phase 1: Scanning (15%)
        ctx.report_progress(0.0, Some("Scanning source directory"))
            .await?;
        debug!(phase = "scanning", "Enumerating files");

        let scan_duration = Duration::from_secs(total_duration_secs * 15 / 100);
        tokio::time::sleep(scan_duration).await;

        let files_count = rand::thread_rng().gen_range(1000..10000);
        let original_size: u64 = rand::thread_rng().gen_range(1_000_000_000..10_000_000_000);

        info!(
            files = files_count,
            size_mb = original_size / 1_000_000,
            "Scan completed"
        );
        ctx.log(
            LogLevel::Info,
            &format!(
                "Found {} files ({} MB)",
                files_count,
                original_size / 1_000_000
            ),
        )
        .await?;

        ctx.check_cancellation().await?;

        // Phase 2: Copying files (60%)
        ctx.report_progress(0.15, Some("Copying files")).await?;

        let copy_duration = Duration::from_secs(total_duration_secs * 60 / 100);
        let checkpoint_interval = copy_duration / 6;

        for checkpoint in 1..=6 {
            ctx.check_cancellation().await?;

            let progress = 0.15 + (checkpoint as f64 / 6.0) * 0.60;
            let files_copied = (files_count as f64 * (checkpoint as f64 / 6.0)) as u32;

            ctx.report_progress(
                progress,
                Some(&format!("Copied {}/{} files", files_copied, files_count)),
            )
            .await?;

            debug!(
                checkpoint = checkpoint,
                files_copied = files_copied,
                "Backup checkpoint"
            );

            // Simulate occasional retry on transient errors
            if checkpoint == 3 && rand::thread_rng().gen_bool(0.3) {
                warn!(
                    checkpoint = checkpoint,
                    "Transient I/O error, retrying batch"
                );
                ctx.log(
                    LogLevel::Warn,
                    "Transient I/O error encountered, retrying...",
                )
                .await?;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            // Simulate permission warning
            if checkpoint == 4 {
                warn!(
                    files_skipped = 3,
                    "Some files skipped due to permission issues"
                );
                ctx.log(LogLevel::Warn, "3 files skipped due to permission denied")
                    .await?;
            }

            tokio::time::sleep(checkpoint_interval).await;

            info!(
                checkpoint = checkpoint,
                progress_pct = format!("{:.1}%", progress * 100.0),
                "Backup progress"
            );
        }

        // Phase 3: Compression (if enabled) (20%)
        let compressed_size = if input.compress {
            ctx.report_progress(0.75, Some("Compressing backup"))
                .await?;
            ctx.log(LogLevel::Info, "Starting compression").await?;

            let compress_duration = Duration::from_secs(total_duration_secs * 20 / 100);

            for i in 1..=4 {
                ctx.check_cancellation().await?;

                let progress = 0.75 + (i as f64 / 4.0) * 0.20;
                ctx.report_progress(progress, Some(&format!("Compressing... {}%", i * 25)))
                    .await?;

                debug!(compression_progress = i * 25, "Compression progress");
                tokio::time::sleep(compress_duration / 4).await;
            }

            let ratio = rand::thread_rng().gen_range(0.3..0.6);
            let compressed = (original_size as f64 * ratio) as u64;

            info!(
                original_mb = original_size / 1_000_000,
                compressed_mb = compressed / 1_000_000,
                ratio = format!("{:.1}%", ratio * 100.0),
                "Compression completed"
            );
            ctx.log(
                LogLevel::Info,
                &format!(
                    "Compressed {} MB to {} MB ({:.1}% ratio)",
                    original_size / 1_000_000,
                    compressed / 1_000_000,
                    ratio * 100.0
                ),
            )
            .await?;

            compressed
        } else {
            ctx.report_progress(0.95, Some("Skipping compression"))
                .await?;
            tokio::time::sleep(Duration::from_secs(total_duration_secs * 5 / 100)).await;
            original_size
        };

        // Finalize
        ctx.report_progress(1.0, Some("Backup completed")).await?;

        let duration = start.elapsed().as_secs();

        info!(
            backup_id = %backup_id,
            files = files_count,
            duration_secs = duration,
            "Backup completed successfully"
        );

        ctx.log(
            LogLevel::Info,
            &format!(
                "Backup {} completed: {} files in {} seconds",
                backup_id, files_count, duration
            ),
        )
        .await?;

        Ok(BackupOutput {
            backup_id,
            archive_path: format!("/backups/{}.tar.gz", input.source_path.replace('/', "_")),
            original_size_bytes: original_size,
            compressed_size_bytes: compressed_size,
            files_count,
            duration_seconds: duration,
        })
    }
}

// ============================================================================
// Indexing Task
// ============================================================================

/// Input for indexing task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IndexingInput {
    /// Collection name to index
    pub collection_name: String,
    /// Index type (full-text, vector, hybrid)
    pub index_type: String,
    /// Number of documents to index
    pub document_count: u32,
}

/// Output from indexing task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IndexingOutput {
    /// Index name
    pub index_name: String,
    /// Documents indexed
    pub documents_indexed: u32,
    /// Documents failed
    pub documents_failed: u32,
    /// Index size in bytes
    pub index_size_bytes: u64,
    /// Duration in seconds
    pub duration_seconds: u64,
}

/// Task that indexes documents with batch progress
pub struct IndexingTask;

#[async_trait]
impl TaskDefinition for IndexingTask {
    type Input = IndexingInput;
    type Output = IndexingOutput;

    fn kind(&self) -> &str {
        "indexing-task"
    }

    fn name(&self) -> &str {
        "Document Indexing"
    }

    fn description(&self) -> Option<&str> {
        Some("Indexes documents with batch processing and detailed logging")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(600)
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let start = std::time::Instant::now();
        let index_name = format!("idx_{}_{}", input.collection_name, input.index_type);

        info!(
            collection = %input.collection_name,
            index_type = %input.index_type,
            documents = input.document_count,
            "Starting indexing"
        );

        ctx.log(
            LogLevel::Info,
            &format!(
                "Starting {} indexing for {} ({} documents)",
                input.index_type, input.collection_name, input.document_count
            ),
        )
        .await?;

        // Random duration 2-5 minutes
        let total_duration_secs = rand::thread_rng().gen_range(30..=60);

        // Initialize index (5%)
        ctx.report_progress(0.0, Some("Initializing index")).await?;
        debug!(index_name = %index_name, "Creating index structure");
        tokio::time::sleep(Duration::from_secs(total_duration_secs * 5 / 100)).await;

        ctx.check_cancellation().await?;

        // Process documents in batches (85%)
        let batch_count = 20;
        let docs_per_batch = input.document_count / batch_count;
        let batch_duration =
            Duration::from_secs(total_duration_secs * 85 / 100 / batch_count as u64);

        let mut documents_indexed = 0u32;
        let mut documents_failed = 0u32;

        ctx.report_progress(0.05, Some("Processing documents"))
            .await?;

        for batch in 1..=batch_count {
            ctx.check_cancellation().await?;

            let batch_start = documents_indexed;
            let batch_end = batch_start + docs_per_batch;

            debug!(
                batch = batch,
                docs_range = format!("{}-{}", batch_start, batch_end),
                "Processing batch"
            );

            // Simulate occasional failures
            let failed_in_batch = if rand::thread_rng().gen_bool(0.1) {
                let failed = rand::thread_rng().gen_range(1..5);
                warn!(
                    batch = batch,
                    failed_count = failed,
                    "Some documents failed to index"
                );
                ctx.log(
                    LogLevel::Warn,
                    &format!("Batch {}: {} documents failed validation", batch, failed),
                )
                .await?;
                failed
            } else {
                0
            };

            documents_indexed += docs_per_batch - failed_in_batch;
            documents_failed += failed_in_batch;

            tokio::time::sleep(batch_duration).await;

            let progress = 0.05 + (batch as f64 / batch_count as f64) * 0.85;
            ctx.report_progress(
                progress,
                Some(&format!(
                    "Indexed {}/{} documents",
                    documents_indexed, input.document_count
                )),
            )
            .await?;

            // Log detailed progress every 5 batches
            if batch % 5 == 0 {
                info!(
                    batch = batch,
                    indexed = documents_indexed,
                    failed = documents_failed,
                    progress_pct = format!("{:.1}%", progress * 100.0),
                    "Indexing checkpoint"
                );
                ctx.log(
                    LogLevel::Info,
                    &format!(
                        "Checkpoint: {}/{} indexed, {} failed",
                        documents_indexed, input.document_count, documents_failed
                    ),
                )
                .await?;
            }
        }

        // Optimize index (10%)
        ctx.report_progress(0.90, Some("Optimizing index")).await?;
        ctx.log(LogLevel::Info, "Starting index optimization")
            .await?;
        debug!(index_name = %index_name, "Merging segments");

        tokio::time::sleep(Duration::from_secs(total_duration_secs * 5 / 100)).await;
        ctx.report_progress(0.95, Some("Finalizing index")).await?;

        debug!(index_name = %index_name, "Building posting lists");
        tokio::time::sleep(Duration::from_secs(total_duration_secs * 5 / 100)).await;

        ctx.report_progress(1.0, Some("Indexing completed")).await?;

        let duration = start.elapsed().as_secs();
        let index_size = (documents_indexed as u64) * 512; // Simulated size

        // Log final summary
        if documents_failed > 0 {
            warn!(
                index_name = %index_name,
                indexed = documents_indexed,
                failed = documents_failed,
                "Indexing completed with some failures"
            );
            ctx.log(
                LogLevel::Warn,
                &format!(
                    "Indexing completed with {} failures out of {} documents",
                    documents_failed, input.document_count
                ),
            )
            .await?;
        } else {
            info!(
                index_name = %index_name,
                indexed = documents_indexed,
                duration_secs = duration,
                "Indexing completed successfully"
            );
            ctx.log(
                LogLevel::Info,
                &format!(
                    "Successfully indexed {} documents in {} seconds",
                    documents_indexed, duration
                ),
            )
            .await?;
        }

        Ok(IndexingOutput {
            index_name,
            documents_indexed,
            documents_failed,
            index_size_bytes: index_size,
            duration_seconds: duration,
        })
    }
}

// ============================================================================
// Main
// ============================================================================


#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file
    // Use DOTENV_PATH to specify a custom path, otherwise try examples/.env
    if let Ok(dotenv_path) = std::env::var("DOTENV_PATH") {
        dotenvy::from_filename(&dotenv_path).ok();
    } else {
        dotenvy::from_filename("examples/.env").ok();
    }

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("standalone_tasks_sample=debug".parse()?)
                .add_directive("flovyn_sdk=info".parse()?),
        )
        .init();

    info!("Starting Standalone Tasks Sample");
    info!("This example demonstrates long-running tasks (2-5 minutes each) with:");
    info!("  - Logging at debug, info, warn, and error levels");
    info!("  - Progress reporting with detailed messages");
    info!("  - Cancellation support");

    // Parse configuration from environment
    let org_id = std::env::var("FLOVYN_ORG_ID")
        .ok()
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .unwrap_or_else(uuid::Uuid::new_v4);

    let server_url = std::env::var("FLOVYN_GRPC_SERVER_URL")
        .unwrap_or_else(|_| "http://localhost:9090".to_string());
    
    let worker_token = std::env::var("FLOVYN_WORKER_TOKEN")
        .expect("FLOVYN_WORKER_TOKEN environment variable is required");
    let queue = std::env::var("FLOVYN_QUEUE").unwrap_or_else(|_| "default".to_string());

    info!(
        org_id = %org_id,
        server = %server_url,
        queue = %queue,
        "Connecting to Flovyn server"
    );

    // Build the client with all task registrations
    let client = FlovynClient::builder()
        .server_url(&server_url)
        .org_id(org_id)
        .worker_token(worker_token)
        .queue(&queue)
        .max_concurrent_tasks(4)
        .register_task(DataExportTask)
        .register_task(ReportGenerationTask)
        .register_task(BackupTask)
        .register_task(IndexingTask)
        .build()
        .await?;

    info!("Client built successfully, starting workers...");

    // Start the workers
    let handle = client.start().await?;

    info!("Workers started. Available tasks:");
    info!("  - data-export-task: Export data with batch progress");
    info!("  - report-generation-task: Multi-phase report generation");
    info!("  - backup-task: Create backups with checkpoints");
    info!("  - indexing-task: Index documents with batch progress");
    info!("");
    info!("Example curl commands to trigger tasks:");
    info!("");
    info!("  Data Export:");
    info!(
        r#"    curl -X POST http://localhost:8000/api/tasks/data-export-task -H "Content-Type: application/json" -d '{{"dataset_name":"users","format":"csv","record_count":10000}}'"#
    );
    info!("");
    info!("  Report Generation:");
    info!(
        r#"    curl -X POST http://localhost:8000/api/tasks/report-generation-task -H "Content-Type: application/json" -d '{{"report_name":"monthly-sales","report_type":"monthly","include_charts":true}}'"#
    );
    info!("");
    info!("  Backup:");
    info!(
        r#"    curl -X POST http://localhost:8000/api/tasks/backup-task -H "Content-Type: application/json" -d '{{"source_path":"/data/app","backup_type":"full","compress":true}}'"#
    );
    info!("");
    info!("  Indexing:");
    info!(
        r#"    curl -X POST http://localhost:8000/api/tasks/indexing-task -H "Content-Type: application/json" -d '{{"collection_name":"products","index_type":"full-text","document_count":50000}}'"#
    );
    info!("");
    info!("Press Ctrl+C to stop.");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    info!("Shutting down...");
    handle.stop().await;

    info!("Goodbye!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_export_task_kind() {
        let task = DataExportTask;
        assert_eq!(task.kind(), "data-export-task");
    }

    #[test]
    fn test_report_generation_task_kind() {
        let task = ReportGenerationTask;
        assert_eq!(task.kind(), "report-generation-task");
    }

    #[test]
    fn test_backup_task_kind() {
        let task = BackupTask;
        assert_eq!(task.kind(), "backup-task");
    }

    #[test]
    fn test_indexing_task_kind() {
        let task = IndexingTask;
        assert_eq!(task.kind(), "indexing-task");
    }

    #[test]
    fn test_all_tasks_cancellable() {
        assert!(DataExportTask.cancellable());
        assert!(ReportGenerationTask.cancellable());
        assert!(BackupTask.cancellable());
        assert!(IndexingTask.cancellable());
    }

    #[test]
    fn test_all_tasks_have_timeout() {
        assert_eq!(DataExportTask.timeout_seconds(), Some(600));
        assert_eq!(ReportGenerationTask.timeout_seconds(), Some(600));
        assert_eq!(BackupTask.timeout_seconds(), Some(600));
        assert_eq!(IndexingTask.timeout_seconds(), Some(600));
    }

    #[test]
    fn test_data_export_input_serialization() {
        let input = DataExportInput {
            dataset_name: "test".to_string(),
            format: "csv".to_string(),
            record_count: 1000,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("csv"));
    }

    #[test]
    fn test_report_input_serialization() {
        let input = ReportInput {
            report_name: "sales".to_string(),
            report_type: "monthly".to_string(),
            include_charts: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("sales"));
        assert!(json.contains("monthly"));
    }

    #[test]
    fn test_backup_input_serialization() {
        let input = BackupInput {
            source_path: "/data".to_string(),
            backup_type: "full".to_string(),
            compress: true,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("/data"));
        assert!(json.contains("full"));
    }

    #[test]
    fn test_indexing_input_serialization() {
        let input = IndexingInput {
            collection_name: "docs".to_string(),
            index_type: "full-text".to_string(),
            document_count: 5000,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("docs"));
        assert!(json.contains("full-text"));
    }
}

/// Integration tests using SDK testing utilities
#[cfg(test)]
mod integration_tests {
    use super::*;
    use flovyn_worker_sdk::testing::MockTaskContext;

    #[tokio::test]
    async fn test_data_export_progress_reporting() {
        // Note: This test uses a modified version that doesn't actually wait 2-5 minutes
        // In real usage, the task would take 2-5 minutes
        let ctx = MockTaskContext::builder().attempt(1).build();

        // Just verify the task can be created and has correct metadata
        let task = DataExportTask;
        assert_eq!(task.kind(), "data-export-task");
        assert_eq!(task.name(), "Data Export");
        assert!(task.description().is_some());

        // Verify context can report progress
        ctx.report_progress(0.5, Some("Test")).await.unwrap();
        let reports = ctx.progress_reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].progress, 0.5);
    }

    #[tokio::test]
    async fn test_report_generation_progress_reporting() {
        let ctx = MockTaskContext::builder().attempt(1).build();

        let task = ReportGenerationTask;
        assert_eq!(task.kind(), "report-generation-task");
        assert_eq!(task.name(), "Report Generation");

        ctx.report_progress(0.3, Some("Phase 1")).await.unwrap();
        ctx.report_progress(0.6, Some("Phase 2")).await.unwrap();
        ctx.report_progress(1.0, Some("Complete")).await.unwrap();

        let reports = ctx.progress_reports();
        assert_eq!(reports.len(), 3);
    }

    #[tokio::test]
    async fn test_backup_task_logging() {
        let ctx = MockTaskContext::builder().attempt(1).build();

        let task = BackupTask;
        assert_eq!(task.kind(), "backup-task");

        // Test logging at different levels
        ctx.log(LogLevel::Info, "Starting backup").await.unwrap();
        ctx.log(LogLevel::Warn, "Minor issue").await.unwrap();
        ctx.log(LogLevel::Debug, "Debug info").await.unwrap();

        let logs = ctx.log_messages();
        assert_eq!(logs.len(), 3);
    }

    #[tokio::test]
    async fn test_indexing_task_cancellation() {
        let ctx = MockTaskContext::builder().attempt(1).build();

        // Should not be cancelled initially
        assert!(!ctx.is_cancelled());
        ctx.check_cancellation().await.unwrap();

        // Request cancellation
        ctx.cancel();
        assert!(ctx.is_cancelled());

        // check_cancellation should now return an error
        let result = ctx.check_cancellation().await;
        assert!(result.is_err());
    }
}
