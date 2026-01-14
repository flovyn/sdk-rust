//! Shipment Task
//!
//! Simulates shipment creation for an order.

use crate::models::*;
use async_trait::async_trait;
use flovyn_worker_sdk::prelude::*;
use tracing::info;

/// Task that creates a shipment for an order
pub struct ShipmentTask;

#[async_trait]
impl TaskDefinition for ShipmentTask {
    type Input = ShipmentTaskInput;
    type Output = ShipmentResponse;

    fn kind(&self) -> &str {
        "shipment-task"
    }

    fn name(&self) -> &str {
        "Shipment Creation"
    }

    fn description(&self) -> Option<&str> {
        Some("Creates a shipment for an order")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(60) // 1 minute timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        info!(
            order_id = %input.order_id,
            city = %input.shipping_address.city,
            country = %input.shipping_address.country,
            "Creating shipment"
        );

        // Simulate shipment creation
        ctx.report_progress(0.2, Some("Validating shipping address"))
            .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(0.4, Some("Selecting carrier")).await?;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(0.6, Some("Creating shipping label"))
            .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(0.8, Some("Scheduling pickup")).await?;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        ctx.report_progress(1.0, Some("Shipment created")).await?;

        // Generate shipment ID and tracking number
        let shipment_id = format!("ship-{}", uuid::Uuid::new_v4());
        let tracking_number = format!("TRK{}", rand::random::<u32>() % 1_000_000_000);

        // Estimate delivery (5-7 business days)
        let delivery_days = 5 + (rand::random::<u32>() % 3);
        let estimated_delivery = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(delivery_days as i64))
            .map(|dt| dt.format("%Y-%m-%d").to_string());

        info!(
            order_id = %input.order_id,
            shipment_id = %shipment_id,
            tracking_number = %tracking_number,
            estimated_delivery = ?estimated_delivery,
            "Shipment created successfully"
        );

        Ok(ShipmentResponse {
            status: ShipmentStatus::Created,
            shipment_id,
            tracking_number: Some(tracking_number),
            estimated_delivery,
            failure_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shipment_task_kind() {
        let task = ShipmentTask;
        assert_eq!(task.kind(), "shipment-task");
    }

    #[test]
    fn test_shipment_task_timeout() {
        let task = ShipmentTask;
        assert_eq!(task.timeout_seconds(), Some(60));
    }
}
