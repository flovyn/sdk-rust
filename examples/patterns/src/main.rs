//! Patterns Sample
//!
//! This sample demonstrates various workflow patterns:
//!
//! 1. **Durable Timers** - Timers that survive worker restarts
//! 2. **Signals** - External events and conversational workflows
//! 3. **Promises** - External signals and human-in-the-loop workflows (legacy)
//! 4. **Child Workflows** - Parent-child workflow orchestration
//! 5. **Retry Patterns** - Exponential backoff and circuit breaker
//! 6. **Parallel Execution** - Fan-out/fan-in, racing, timeouts, batch processing

pub mod child_workflow;
pub mod parallel_tasks;
pub mod parallel_workflow;
pub mod promise_workflow;
pub mod retry_workflow;
pub mod signal_workflow;
pub mod timer_workflow;

use child_workflow::{BatchProcessingWorkflow, ControlledParallelWorkflow, ItemProcessorWorkflow};
use flovyn_worker_sdk::prelude::*;
use parallel_tasks::{
    FetchDataTask, FetchItemsTask, ProcessItemTask, RunOperationTask, SlowOperationTask,
};
use parallel_workflow::{
    BatchWithConcurrencyWorkflow, DynamicParallelismWorkflow, FanOutFanInWorkflow,
    PartialCompletionWorkflow, RacingWorkflow, TimeoutWorkflow,
};
use promise_workflow::{ApprovalWorkflow, MultiApprovalWorkflow};
use retry_workflow::{CircuitBreakerWorkflow, RetryWorkflow};
use signal_workflow::{ConversationWorkflow, EventCollectorWorkflow, WaitForSignalWorkflow};
use timer_workflow::{MultiStepTimerWorkflow, ReminderWorkflow};
use tracing::info;

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
                .add_directive("patterns_sample=info".parse()?)
                .add_directive("flovyn_sdk=info".parse()?),
        )
        .init();

    info!("Starting Patterns Sample");

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

    // Build the client with fluent registration
    let client = FlovynClient::builder()
        .server_url(&server_url)
        .org_id(org_id)
        .worker_token(worker_token)
        .queue(&queue)
        .max_concurrent_workflows(10)
        // Timer workflows
        .register_workflow(ReminderWorkflow)
        .register_workflow(MultiStepTimerWorkflow)
        // Signal workflows
        .register_workflow(WaitForSignalWorkflow)
        .register_workflow(ConversationWorkflow)
        .register_workflow(EventCollectorWorkflow)
        // Promise workflows (legacy, prefer signals)
        .register_workflow(ApprovalWorkflow)
        .register_workflow(MultiApprovalWorkflow)
        // Child workflow patterns
        .register_workflow(BatchProcessingWorkflow)
        .register_workflow(ItemProcessorWorkflow)
        .register_workflow(ControlledParallelWorkflow)
        // Retry pattern workflows
        .register_workflow(RetryWorkflow)
        .register_workflow(CircuitBreakerWorkflow)
        // Parallel execution workflows
        .register_workflow(FanOutFanInWorkflow)
        .register_workflow(RacingWorkflow)
        .register_workflow(TimeoutWorkflow)
        .register_workflow(BatchWithConcurrencyWorkflow)
        .register_workflow(PartialCompletionWorkflow)
        .register_workflow(DynamicParallelismWorkflow)
        // Parallel execution tasks
        .register_task(ProcessItemTask)
        .register_task(FetchDataTask)
        .register_task(SlowOperationTask)
        .register_task(RunOperationTask)
        .register_task(FetchItemsTask)
        .build()
        .await?;

    info!(
        workflows = ?[
            "reminder-workflow",
            "multi-step-timer-workflow",
            "wait-for-signal-workflow",
            "conversation-workflow",
            "event-collector-workflow",
            "approval-workflow",
            "multi-approval-workflow",
            "batch-processing-workflow",
            "item-processor-workflow",
            "controlled-parallel-workflow",
            "retry-workflow",
            "circuit-breaker-workflow",
            "fan-out-fan-in-workflow",
            "racing-workflow",
            "timeout-workflow",
            "batch-with-concurrency-workflow",
            "partial-completion-workflow",
            "dynamic-parallelism-workflow",
        ],
        "Registered pattern showcase workflows"
    );

    // Start the workers
    let handle = client.start().await?;

    info!("Workers started. Press Ctrl+C to stop.");
    info!("");
    info!("Available patterns:");
    info!("");
    info!("1. DURABLE TIMERS");
    info!("   - reminder-workflow: Schedule a reminder with delay");
    info!("   - multi-step-timer-workflow: Multiple checkpoints with timers");
    info!("");
    info!("2. SIGNALS (External Events)");
    info!("   - wait-for-signal-workflow: Wait for a single external signal");
    info!("   - conversation-workflow: Multi-turn conversational pattern (chatbot)");
    info!("   - event-collector-workflow: Collect multiple events before processing");
    info!("");
    info!("3. PROMISES (Legacy - prefer Signals)");
    info!("   - approval-workflow: Wait for external approval");
    info!("   - multi-approval-workflow: Require multiple approvers");
    info!("");
    info!("4. CHILD WORKFLOWS");
    info!("   - batch-processing-workflow: Fan-out/fan-in pattern");
    info!("   - controlled-parallel-workflow: Controlled parallelism");
    info!("");
    info!("5. RETRY PATTERNS");
    info!("   - retry-workflow: Exponential backoff retry");
    info!("   - circuit-breaker-workflow: Circuit breaker pattern");
    info!("");
    info!("6. PARALLEL EXECUTION");
    info!("   - fan-out-fan-in-workflow: Process items in parallel, aggregate results");
    info!("   - racing-workflow: Race multiple operations, take first result");
    info!("   - timeout-workflow: Add timeouts to operations");
    info!("   - batch-with-concurrency-workflow: Process with controlled parallelism");
    info!("   - partial-completion-workflow: Wait for N of M to complete");
    info!("   - dynamic-parallelism-workflow: Runtime-determined parallelism");
    info!("");
    info!("Example: Start a conversation workflow with SignalWithStart:");
    info!("  curl -X POST http://localhost:8000/api/orgs/dev/workflow-executions/signal-with-start \\");
    info!("    -H 'Content-Type: application/json' \\");
    info!(
        "    -d '{{\"workflowKind\": \"conversation-workflow\", \"workflowId\": \"chat-123\", \\"
    );
    info!("         \"workflowInput\": {{}}, \"signalName\": \"message\", \\");
    info!("         \"signalValue\": {{\"content\": \"Hello!\"}}}}'");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    info!("Shutting down...");
    handle.stop().await;

    info!("Goodbye!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::timer_workflow::{ReminderInput, ReminderOutput};

    #[test]
    fn test_reminder_input_serialization() {
        let input = ReminderInput {
            message: "Test".to_string(),
            delay_seconds: 60,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("Test"));
    }

    #[test]
    fn test_reminder_output_serialization() {
        let output = ReminderOutput {
            delivered: true,
            delivered_at: 1234567890,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("true"));
    }
}
