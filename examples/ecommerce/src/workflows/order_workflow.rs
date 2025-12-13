//! Order Processing Workflow (Saga Pattern)
//!
//! This workflow demonstrates the saga pattern for distributed transactions.
//! It processes an order through multiple steps:
//! 1. Payment processing
//! 2. Inventory reservation
//! 3. Shipment creation
//!
//! If any step fails, compensation actions are executed to rollback previous steps.

use crate::models::*;
use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use tracing::{error, info, warn};

/// Order processing workflow implementing the saga pattern
pub struct OrderWorkflow;

#[async_trait]
impl WorkflowDefinition for OrderWorkflow {
    type Input = OrderInput;
    type Output = OrderOutput;

    fn kind(&self) -> &str {
        "ecommerce-order"
    }

    fn name(&self) -> &str {
        "E-commerce Order Processing"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Processes e-commerce orders using the saga pattern with compensation")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(3600) // 1 hour timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "ecommerce".to_string(),
            "order".to_string(),
            "saga".to_string(),
            "payment".to_string(),
            "inventory".to_string(),
            "shipping".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(order_id = %input.order_id, "Starting order processing");

        // Track state for compensation (initialized to None, will be set if step succeeds)
        #[allow(unused_assignments)]
        let mut payment_id: Option<String> = None;
        #[allow(unused_assignments)]
        let mut inventory_reservation_id: Option<String> = None;

        // Set initial status
        ctx.set_raw("status", serde_json::to_value(OrderStatus::Created)?)
            .await?;
        ctx.set_raw("orderId", serde_json::to_value(&input.order_id)?)
            .await?;

        // =========================================================================
        // Step 1: Process Payment
        // =========================================================================
        ctx.set_raw(
            "status",
            serde_json::to_value(OrderStatus::PaymentProcessing)?,
        )
        .await?;
        info!(order_id = %input.order_id, "Processing payment");

        let payment_input = PaymentTaskInput {
            order_id: input.order_id.clone(),
            amount: input.total_amount,
            customer_id: input.customer_id.clone(),
        };
        let payment_input_value = serde_json::to_value(&payment_input)?;

        let payment_result = ctx
            .schedule_raw("payment-task", payment_input_value)
            .await?;
        let payment_response: PaymentResponse = serde_json::from_value(payment_result)?;

        if payment_response.status != PaymentStatus::Succeeded {
            error!(
                order_id = %input.order_id,
                reason = ?payment_response.failure_reason,
                "Payment failed"
            );
            ctx.set_raw("status", serde_json::to_value(OrderStatus::Failed)?)
                .await?;
            return Ok(OrderOutput::failed(
                &input.order_id,
                &format!("Payment failed: {:?}", payment_response.failure_reason),
            ));
        }

        payment_id = payment_response.transaction_id.clone();
        info!(
            order_id = %input.order_id,
            transaction_id = ?payment_id,
            "Payment successful"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(OrderStatus::PaymentCompleted)?,
        )
        .await?;
        if let Some(ref id) = payment_id {
            ctx.set_raw("paymentId", serde_json::to_value(id)?).await?;
        }

        // Check for cancellation before proceeding
        ctx.check_cancellation().await?;

        // =========================================================================
        // Step 2: Reserve Inventory
        // =========================================================================
        ctx.set_raw(
            "status",
            serde_json::to_value(OrderStatus::InventoryReserved)?,
        )
        .await?;
        info!(order_id = %input.order_id, "Reserving inventory");

        let inventory_input = InventoryTaskInput {
            order_id: input.order_id.clone(),
            items: input.items.clone(),
        };
        let inventory_input_value = serde_json::to_value(&inventory_input)?;

        let inventory_result = ctx
            .schedule_raw("inventory-task", inventory_input_value)
            .await?;
        let inventory_response: InventoryResponse = serde_json::from_value(inventory_result)?;

        if inventory_response.status != InventoryStatus::Reserved {
            error!(
                order_id = %input.order_id,
                reason = ?inventory_response.failure_reason,
                "Inventory reservation failed"
            );
            ctx.set_raw("status", serde_json::to_value(OrderStatus::Failed)?)
                .await?;

            // Compensation: Refund payment
            if let Some(ref pid) = payment_id {
                warn!(payment_id = %pid, "Compensating: Refunding payment");
                let refund_input = RefundTaskInput {
                    order_id: input.order_id.clone(),
                    payment_id: pid.clone(),
                    amount: input.total_amount,
                    reason: "Inventory reservation failed".to_string(),
                };
                let refund_value = serde_json::to_value(&refund_input)?;
                let _ = ctx.run_raw("refund-payment", refund_value).await;
            }

            return Ok(OrderOutput {
                order_id: input.order_id,
                status: OrderStatus::Failed,
                payment_id,
                inventory_reservation_id: None,
                shipment_id: None,
                tracking_number: None,
                message: format!(
                    "Inventory reservation failed: {:?}",
                    inventory_response.failure_reason
                ),
            });
        }

        inventory_reservation_id = Some(inventory_response.reservation_id.clone());
        info!(
            order_id = %input.order_id,
            reservation_id = %inventory_response.reservation_id,
            "Inventory reserved"
        );
        ctx.set_raw(
            "inventoryReservationId",
            serde_json::to_value(&inventory_response.reservation_id)?,
        )
        .await?;

        // Check for cancellation before proceeding
        ctx.check_cancellation().await?;

        // =========================================================================
        // Step 3: Create Shipment
        // =========================================================================
        info!(order_id = %input.order_id, "Creating shipment");

        let shipment_input = ShipmentTaskInput {
            order_id: input.order_id.clone(),
            items: input.items.clone(),
            shipping_address: input.shipping_address.clone(),
        };
        let shipment_input_value = serde_json::to_value(&shipment_input)?;

        let shipment_result = ctx
            .schedule_raw("shipment-task", shipment_input_value)
            .await?;
        let shipment_response: ShipmentResponse = serde_json::from_value(shipment_result)?;

        if shipment_response.status != ShipmentStatus::Created {
            error!(
                order_id = %input.order_id,
                reason = ?shipment_response.failure_reason,
                "Shipment creation failed"
            );
            ctx.set_raw("status", serde_json::to_value(OrderStatus::Failed)?)
                .await?;

            // Compensation: Release inventory
            if let Some(ref rid) = inventory_reservation_id {
                warn!(reservation_id = %rid, "Compensating: Releasing inventory");
                let release_input = ReleaseInventoryInput {
                    order_id: input.order_id.clone(),
                    reservation_id: rid.clone(),
                };
                let release_value = serde_json::to_value(&release_input)?;
                let _ = ctx.run_raw("release-inventory", release_value).await;
            }

            // Compensation: Refund payment
            if let Some(ref pid) = payment_id {
                warn!(payment_id = %pid, "Compensating: Refunding payment");
                let refund_input = RefundTaskInput {
                    order_id: input.order_id.clone(),
                    payment_id: pid.clone(),
                    amount: input.total_amount,
                    reason: "Shipment creation failed".to_string(),
                };
                let refund_value = serde_json::to_value(&refund_input)?;
                let _ = ctx.run_raw("refund-payment", refund_value).await;
            }

            return Ok(OrderOutput {
                order_id: input.order_id,
                status: OrderStatus::Failed,
                payment_id,
                inventory_reservation_id,
                shipment_id: None,
                tracking_number: None,
                message: format!(
                    "Shipment creation failed: {:?}",
                    shipment_response.failure_reason
                ),
            });
        }

        info!(
            order_id = %input.order_id,
            shipment_id = %shipment_response.shipment_id,
            tracking = ?shipment_response.tracking_number,
            "Shipment created"
        );
        ctx.set_raw(
            "status",
            serde_json::to_value(OrderStatus::ShipmentCreated)?,
        )
        .await?;
        ctx.set_raw(
            "shipmentId",
            serde_json::to_value(&shipment_response.shipment_id)?,
        )
        .await?;

        // =========================================================================
        // Step 4: Complete Order
        // =========================================================================
        ctx.set_raw("status", serde_json::to_value(OrderStatus::Completed)?)
            .await?;
        info!(order_id = %input.order_id, "Order completed successfully");

        Ok(OrderOutput {
            order_id: input.order_id,
            status: OrderStatus::Completed,
            payment_id,
            inventory_reservation_id,
            shipment_id: Some(shipment_response.shipment_id),
            tracking_number: shipment_response.tracking_number,
            message: "Order completed successfully".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_workflow_kind() {
        let workflow = OrderWorkflow;
        assert_eq!(workflow.kind(), "ecommerce-order");
    }

    #[test]
    fn test_order_workflow_version() {
        let workflow = OrderWorkflow;
        assert_eq!(workflow.version(), SemanticVersion::new(1, 0, 0));
    }

    #[test]
    fn test_order_workflow_cancellable() {
        let workflow = OrderWorkflow;
        assert!(workflow.cancellable());
    }

    #[test]
    fn test_order_workflow_timeout() {
        let workflow = OrderWorkflow;
        assert_eq!(workflow.timeout_seconds(), Some(3600));
    }

    #[test]
    fn test_order_workflow_tags() {
        let workflow = OrderWorkflow;
        let tags = workflow.tags();
        assert!(tags.contains(&"ecommerce".to_string()));
        assert!(tags.contains(&"saga".to_string()));
    }
}

/// Integration tests using SDK testing utilities
#[cfg(test)]
mod integration_tests {
    use super::*;
    use flovyn_sdk::testing::MockWorkflowContext;
    use serde_json::json;

    fn create_test_order_input() -> OrderInput {
        OrderInput {
            order_id: "ORD-TEST-001".to_string(),
            customer_id: "CUST-001".to_string(),
            items: vec![OrderItem {
                sku: "SKU-001".to_string(),
                quantity: 2,
                price: 29.99,
            }],
            total_amount: 59.98,
            shipping_address: Address {
                street: "123 Main St".to_string(),
                city: "San Francisco".to_string(),
                country: "USA".to_string(),
                postal_code: "94102".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_order_workflow_successful_execution() {
        // Set up mock context with all required task results
        // Note: Enum variants serialize as PascalCase (e.g., "Succeeded", not "succeeded")
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "payment-task",
                json!({
                    "status": "Succeeded",
                    "transaction_id": "TXN-12345",
                    "failure_reason": null
                }),
            )
            .task_result(
                "inventory-task",
                json!({
                    "status": "Reserved",
                    "reservation_id": "RES-67890",
                    "failure_reason": null
                }),
            )
            .task_result(
                "shipment-task",
                json!({
                    "status": "Created",
                    "shipment_id": "SHIP-11111",
                    "tracking_number": "TRACK-99999",
                    "failure_reason": null
                }),
            )
            .build();

        let workflow = OrderWorkflow;
        let input = create_test_order_input();
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.status, OrderStatus::Completed);
        assert_eq!(output.order_id, "ORD-TEST-001");
        assert!(output.payment_id.is_some());
        assert!(output.inventory_reservation_id.is_some());
        assert!(output.shipment_id.is_some());

        // Verify all tasks were scheduled
        assert!(ctx.was_task_scheduled("payment-task"));
        assert!(ctx.was_task_scheduled("inventory-task"));
        assert!(ctx.was_task_scheduled("shipment-task"));

        // Verify state was set
        let state = ctx.state_snapshot();
        assert!(state.contains_key("status"));
        assert!(state.contains_key("orderId"));
    }

    #[tokio::test]
    async fn test_order_workflow_payment_failure() {
        // Set up mock context with payment failure
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "payment-task",
                json!({
                    "status": "Failed",
                    "transaction_id": null,
                    "failure_reason": "Insufficient funds"
                }),
            )
            .build();

        let workflow = OrderWorkflow;
        let input = create_test_order_input();
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.status, OrderStatus::Failed);
        assert!(output.message.contains("Payment failed"));

        // Verify only payment task was scheduled (workflow stops early)
        assert!(ctx.was_task_scheduled("payment-task"));
        assert!(!ctx.was_task_scheduled("inventory-task"));
        assert!(!ctx.was_task_scheduled("shipment-task"));
    }

    #[tokio::test]
    async fn test_order_workflow_inventory_failure_triggers_compensation() {
        // Set up mock context where payment succeeds but inventory fails
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "payment-task",
                json!({
                    "status": "Succeeded",
                    "transaction_id": "TXN-12345",
                    "failure_reason": null
                }),
            )
            .task_result(
                "inventory-task",
                json!({
                    "status": "Failed",
                    "reservation_id": "",
                    "failure_reason": "Out of stock"
                }),
            )
            .build();

        let workflow = OrderWorkflow;
        let input = create_test_order_input();
        let result = workflow.execute(&ctx, input).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.status, OrderStatus::Failed);
        assert!(output.message.contains("Inventory reservation failed"));

        // Verify payment task was scheduled
        assert!(ctx.was_task_scheduled("payment-task"));
        assert!(ctx.was_task_scheduled("inventory-task"));
        // Shipment should not be scheduled
        assert!(!ctx.was_task_scheduled("shipment-task"));

        // Verify compensation was executed (refund-payment run_raw)
        assert!(ctx.was_operation_recorded("refund-payment"));
    }

    #[tokio::test]
    async fn test_order_workflow_cancellation() {
        // Create a context that will report cancellation
        let ctx = MockWorkflowContext::builder()
            .task_result(
                "payment-task",
                json!({
                    "status": "Succeeded",
                    "transaction_id": "TXN-12345",
                    "failure_reason": null
                }),
            )
            .build();

        // Request cancellation
        ctx.request_cancellation();

        let workflow = OrderWorkflow;
        let input = create_test_order_input();
        let result = workflow.execute(&ctx, input).await;

        // Should return error due to cancellation
        assert!(result.is_err());
    }
}
