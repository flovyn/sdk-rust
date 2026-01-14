//! Retry Pattern Workflow
//!
//! Demonstrates retry patterns within workflows.
//! This pattern is useful for:
//! - Handling transient failures
//! - Exponential backoff strategies
//! - Circuit breaker patterns

use async_trait::async_trait;
use flovyn_worker_sdk::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

/// Input for retry workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryInput {
    pub operation_name: String,
    pub max_attempts: u32,
    /// Simulated failure probability (0.0 - 1.0)
    /// Note: Values >= 1.0 are capped at 0.99 to ensure at least 1% success chance
    pub failure_probability: f64,
}

/// Output from retry workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryOutput {
    pub operation_name: String,
    pub success: bool,
    pub attempts: u32,
    pub final_message: String,
}

/// Retry result from a single attempt
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct AttemptResult {
    success: bool,
    message: String,
}

/// Workflow demonstrating retry with exponential backoff
///
/// This workflow shows how to implement custom retry logic:
/// 1. Attempt an operation
/// 2. On failure, wait with exponential backoff
/// 3. Retry up to max_attempts times
pub struct RetryWorkflow;

#[async_trait]
impl WorkflowDefinition for RetryWorkflow {
    type Input = RetryInput;
    type Output = RetryOutput;

    fn kind(&self) -> &str {
        "retry-workflow"
    }

    fn name(&self) -> &str {
        "Retry with Exponential Backoff"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates retry pattern with exponential backoff")
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "retry".to_string(),
            "backoff".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            operation = %input.operation_name,
            max_attempts = input.max_attempts,
            "Retry workflow started"
        );

        ctx.set_raw("status", serde_json::to_value("RUNNING")?)
            .await?;
        ctx.set_raw("attempts", serde_json::to_value(0u32)?).await?;

        let base_delay_ms: u64 = 100;
        let max_delay_ms: u64 = 10000;

        // Cap failure probability at 0.99 to ensure there's always at least 1% chance of success
        // This prevents infinite retry loops when failure_probability = 1.0
        let effective_failure_prob = input.failure_probability.min(0.99);

        for attempt in 1..=input.max_attempts {
            ctx.check_cancellation().await?;

            info!(
                operation = %input.operation_name,
                attempt = attempt,
                max_attempts = input.max_attempts,
                "Attempting operation"
            );

            ctx.set_raw("attempts", serde_json::to_value(attempt)?)
                .await?;

            // Simulate the operation
            // Using deterministic random to decide if operation succeeds
            // random_value is in [0, 1), success when random >= failure_probability
            let random_value = ctx.random().next_double();
            let success = random_value >= effective_failure_prob;

            let result = AttemptResult {
                success,
                message: if success {
                    "Operation succeeded".to_string()
                } else {
                    "Operation failed (simulated)".to_string()
                },
            };

            // Record the attempt result
            let result_value = serde_json::to_value(&result)?;
            let _ = ctx
                .run_raw(&format!("attempt-{}", attempt), result_value)
                .await?;

            if result.success {
                info!(
                    operation = %input.operation_name,
                    attempt = attempt,
                    "Operation succeeded"
                );

                ctx.set_raw("status", serde_json::to_value("SUCCEEDED")?)
                    .await?;

                return Ok(RetryOutput {
                    operation_name: input.operation_name,
                    success: true,
                    attempts: attempt,
                    final_message: format!("Succeeded on attempt {}", attempt),
                });
            }

            // Operation failed, calculate backoff
            if attempt < input.max_attempts {
                // Exponential backoff: base_delay * 2^(attempt-1), capped at max_delay
                // Use saturating arithmetic to prevent overflow at high attempt counts
                let delay_ms = 1u64
                    .checked_shl(attempt - 1)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(base_delay_ms)
                    .min(max_delay_ms);

                warn!(
                    operation = %input.operation_name,
                    attempt = attempt,
                    delay_ms = delay_ms,
                    "Operation failed, backing off"
                );

                ctx.sleep(Duration::from_millis(delay_ms)).await?;
            }
        }

        // All attempts exhausted
        warn!(
            operation = %input.operation_name,
            attempts = input.max_attempts,
            "All attempts exhausted"
        );

        ctx.set_raw("status", serde_json::to_value("FAILED")?)
            .await?;

        Ok(RetryOutput {
            operation_name: input.operation_name,
            success: false,
            attempts: input.max_attempts,
            final_message: format!("Failed after {} attempts", input.max_attempts),
        })
    }
}

/// Input for circuit breaker workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CircuitBreakerInput {
    pub service_name: String,
    pub failure_threshold: u32,
    pub recovery_timeout_seconds: u64,
    pub operation_count: u32,
}

/// Output from circuit breaker workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CircuitBreakerOutput {
    pub service_name: String,
    pub successful_operations: u32,
    pub failed_operations: u32,
    pub circuit_trips: u32,
    pub final_state: String,
}

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Workflow demonstrating circuit breaker pattern
///
/// This workflow shows how to implement circuit breaker logic:
/// 1. Closed: Operations proceed normally
/// 2. Open: Operations fail fast (after too many failures)
/// 3. Half-Open: Allow one test operation after recovery timeout
pub struct CircuitBreakerWorkflow;

#[async_trait]
impl WorkflowDefinition for CircuitBreakerWorkflow {
    type Input = CircuitBreakerInput;
    type Output = CircuitBreakerOutput;

    fn kind(&self) -> &str {
        "circuit-breaker-workflow"
    }

    fn name(&self) -> &str {
        "Circuit Breaker Pattern"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates circuit breaker pattern for fault tolerance")
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            service = %input.service_name,
            operations = input.operation_count,
            threshold = input.failure_threshold,
            "Circuit breaker workflow started"
        );

        let mut state = CircuitState::Closed;
        let mut consecutive_failures = 0u32;
        let mut successful_operations = 0u32;
        let mut failed_operations = 0u32;
        let mut circuit_trips = 0u32;

        for op in 0..input.operation_count {
            ctx.check_cancellation().await?;

            // Check circuit state
            match state {
                CircuitState::Open => {
                    info!(
                        service = %input.service_name,
                        operation = op,
                        "Circuit OPEN - waiting for recovery"
                    );

                    // Wait for recovery timeout
                    ctx.sleep(Duration::from_secs(input.recovery_timeout_seconds))
                        .await?;

                    // Move to half-open
                    state = CircuitState::HalfOpen;
                    info!(
                        service = %input.service_name,
                        "Circuit moved to HALF-OPEN"
                    );
                }
                CircuitState::Closed | CircuitState::HalfOpen => {
                    // Attempt operation
                    let success = ctx.random().next_double() > 0.3; // 70% success rate

                    let result_value = serde_json::to_value(success)?;
                    let _ = ctx
                        .run_raw(&format!("operation-{}", op), result_value)
                        .await?;

                    if success {
                        successful_operations += 1;
                        consecutive_failures = 0;

                        if state == CircuitState::HalfOpen {
                            // Success in half-open means we can close the circuit
                            state = CircuitState::Closed;
                            info!(
                                service = %input.service_name,
                                "Circuit moved to CLOSED"
                            );
                        }
                    } else {
                        failed_operations += 1;
                        consecutive_failures += 1;

                        if consecutive_failures >= input.failure_threshold {
                            state = CircuitState::Open;
                            circuit_trips += 1;
                            warn!(
                                service = %input.service_name,
                                failures = consecutive_failures,
                                "Circuit TRIPPED to OPEN"
                            );
                        }
                    }
                }
            }

            // Update state
            ctx.set_raw(
                "circuitState",
                serde_json::to_value(format!("{:?}", state))?,
            )
            .await?;
            ctx.set_raw(
                "consecutiveFailures",
                serde_json::to_value(consecutive_failures)?,
            )
            .await?;
        }

        info!(
            service = %input.service_name,
            successful = successful_operations,
            failed = failed_operations,
            trips = circuit_trips,
            final_state = ?state,
            "Circuit breaker workflow completed"
        );

        Ok(CircuitBreakerOutput {
            service_name: input.service_name,
            successful_operations,
            failed_operations,
            circuit_trips,
            final_state: format!("{:?}", state),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_workflow_kind() {
        let workflow = RetryWorkflow;
        assert_eq!(workflow.kind(), "retry-workflow");
    }

    #[test]
    fn test_circuit_breaker_workflow_kind() {
        let workflow = CircuitBreakerWorkflow;
        assert_eq!(workflow.kind(), "circuit-breaker-workflow");
    }

    #[test]
    fn test_retry_input_serialization() {
        let input = RetryInput {
            operation_name: "external-api-call".to_string(),
            max_attempts: 5,
            failure_probability: 0.3,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("external-api-call"));
        assert!(json.contains("0.3"));
    }
}
