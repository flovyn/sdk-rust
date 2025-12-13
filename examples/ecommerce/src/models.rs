//! E-commerce domain models
//!
//! This module contains all the data types used in the e-commerce order processing workflow.

use serde::{Deserialize, Serialize};

// =============================================================================
// Order Types
// =============================================================================

/// Input for the order processing workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderInput {
    pub order_id: String,
    pub customer_id: String,
    pub items: Vec<OrderItem>,
    pub total_amount: f64,
    pub shipping_address: Address,
}

/// A single item in an order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderItem {
    pub sku: String,
    pub quantity: u32,
    pub price: f64,
}

/// Shipping address
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub street: String,
    pub city: String,
    pub country: String,
    pub postal_code: String,
}

/// Output from the order processing workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderOutput {
    pub order_id: String,
    pub status: OrderStatus,
    pub payment_id: Option<String>,
    pub inventory_reservation_id: Option<String>,
    pub shipment_id: Option<String>,
    pub tracking_number: Option<String>,
    pub message: String,
}

impl OrderOutput {
    /// Create a failed order output
    pub fn failed(order_id: &str, message: &str) -> Self {
        Self {
            order_id: order_id.to_string(),
            status: OrderStatus::Failed,
            payment_id: None,
            inventory_reservation_id: None,
            shipment_id: None,
            tracking_number: None,
            message: message.to_string(),
        }
    }
}

/// Order processing status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderStatus {
    Created,
    PaymentProcessing,
    PaymentCompleted,
    InventoryReserved,
    ShipmentCreated,
    Completed,
    Failed,
    Cancelled,
}

// =============================================================================
// Payment Types
// =============================================================================

/// Input for the payment task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentTaskInput {
    pub order_id: String,
    pub amount: f64,
    pub customer_id: String,
}

/// Response from the payment task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentResponse {
    pub status: PaymentStatus,
    pub transaction_id: Option<String>,
    pub failure_reason: Option<String>,
}

/// Payment processing status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentStatus {
    Succeeded,
    Failed,
    Pending,
}

// =============================================================================
// Inventory Types
// =============================================================================

/// Input for the inventory task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryTaskInput {
    pub order_id: String,
    pub items: Vec<OrderItem>,
}

/// Response from the inventory task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryResponse {
    pub status: InventoryStatus,
    pub reservation_id: String,
    pub failure_reason: Option<String>,
}

/// Inventory reservation status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InventoryStatus {
    Reserved,
    OutOfStock,
    Failed,
}

// =============================================================================
// Shipment Types
// =============================================================================

/// Input for the shipment task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipmentTaskInput {
    pub order_id: String,
    pub items: Vec<OrderItem>,
    pub shipping_address: Address,
}

/// Response from the shipment task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipmentResponse {
    pub status: ShipmentStatus,
    pub shipment_id: String,
    pub tracking_number: Option<String>,
    pub estimated_delivery: Option<String>,
    pub failure_reason: Option<String>,
}

/// Shipment creation status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShipmentStatus {
    Created,
    Failed,
}

// =============================================================================
// Compensation Types
// =============================================================================

/// Input for refund task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefundTaskInput {
    pub order_id: String,
    pub payment_id: String,
    pub amount: f64,
    pub reason: String,
}

/// Response from refund task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefundResponse {
    pub status: RefundStatus,
    pub refund_id: Option<String>,
    pub failure_reason: Option<String>,
}

/// Refund status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefundStatus {
    Succeeded,
    Failed,
}

/// Input for inventory release task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInventoryInput {
    pub order_id: String,
    pub reservation_id: String,
}

/// Response from inventory release task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInventoryResponse {
    pub status: ReleaseStatus,
}

/// Inventory release status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseStatus {
    Released,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_input_serialization() {
        let input = OrderInput {
            order_id: "ORD-001".to_string(),
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
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: OrderInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.order_id, input.order_id);
    }

    #[test]
    fn test_order_status_serialization() {
        let status = OrderStatus::PaymentCompleted;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"PAYMENT_COMPLETED\"");
    }

    #[test]
    fn test_order_output_failed() {
        let output = OrderOutput::failed("ORD-001", "Payment declined");
        assert_eq!(output.status, OrderStatus::Failed);
        assert_eq!(output.message, "Payment declined");
    }
}
