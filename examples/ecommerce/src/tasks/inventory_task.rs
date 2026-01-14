//! Inventory Task
//!
//! Simulates inventory reservation for an order.

use crate::models::*;
use async_trait::async_trait;
use flovyn_worker_sdk::prelude::*;
use tracing::info;

/// Task that reserves inventory for an order
pub struct InventoryTask;

#[async_trait]
impl TaskDefinition for InventoryTask {
    type Input = InventoryTaskInput;
    type Output = InventoryResponse;

    fn kind(&self) -> &str {
        "inventory-task"
    }

    fn name(&self) -> &str {
        "Inventory Reservation"
    }

    fn description(&self) -> Option<&str> {
        Some("Reserves inventory for an order")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(30) // 30 second timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        let item_count: u32 = input.items.iter().map(|i| i.quantity).sum();

        info!(
            order_id = %input.order_id,
            items = input.items.len(),
            total_quantity = item_count,
            "Reserving inventory"
        );

        // Simulate inventory check
        ctx.report_progress(0.3, Some("Checking stock levels"))
            .await?;

        // Check for cancellation
        ctx.check_cancellation().await?;

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        ctx.report_progress(0.6, Some("Reserving items")).await?;

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        ctx.report_progress(1.0, Some("Reservation complete"))
            .await?;

        // Generate reservation ID
        let reservation_id = format!("res-{}", uuid::Uuid::new_v4());

        info!(
            order_id = %input.order_id,
            reservation_id = %reservation_id,
            "Inventory reserved successfully"
        );

        Ok(InventoryResponse {
            status: InventoryStatus::Reserved,
            reservation_id,
            failure_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inventory_task_kind() {
        let task = InventoryTask;
        assert_eq!(task.kind(), "inventory-task");
    }

    #[test]
    fn test_inventory_task_timeout() {
        let task = InventoryTask;
        assert_eq!(task.timeout_seconds(), Some(30));
    }
}
