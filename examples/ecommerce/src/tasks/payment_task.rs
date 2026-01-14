//! Payment Task
//!
//! Simulates payment processing for an order.

use crate::models::*;
use async_trait::async_trait;
use flovyn_worker_sdk::prelude::*;
use tracing::info;

/// Task that processes payments
pub struct PaymentTask;

#[async_trait]
impl TaskDefinition for PaymentTask {
    type Input = PaymentTaskInput;
    type Output = PaymentResponse;

    fn kind(&self) -> &str {
        "payment-task"
    }

    fn name(&self) -> &str {
        "Payment Processing"
    }

    fn description(&self) -> Option<&str> {
        Some("Processes payment for an order")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(60) // 1 minute timeout for payment
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        info!(
            order_id = %input.order_id,
            amount = %input.amount,
            customer_id = %input.customer_id,
            "Processing payment"
        );

        // Simulate payment processing steps
        ctx.report_progress(0.2, Some("Validating payment details"))
            .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        // Simulate network delay
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        ctx.report_progress(0.5, Some("Connecting to payment gateway"))
            .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        ctx.report_progress(0.8, Some("Charging card")).await?;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(1.0, Some("Payment completed")).await?;

        // Generate transaction ID
        let transaction_id = format!("txn-{}", uuid::Uuid::new_v4());

        info!(
            order_id = %input.order_id,
            transaction_id = %transaction_id,
            "Payment successful"
        );

        Ok(PaymentResponse {
            status: PaymentStatus::Succeeded,
            transaction_id: Some(transaction_id),
            failure_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payment_task_kind() {
        let task = PaymentTask;
        assert_eq!(task.kind(), "payment-task");
    }

    #[test]
    fn test_payment_task_timeout() {
        let task = PaymentTask;
        assert_eq!(task.timeout_seconds(), Some(60));
    }
}
