//! E-commerce Order Processing Sample
//!
//! This sample demonstrates the saga pattern for distributed transactions.
//! It processes orders through multiple steps with compensation on failure:
//!
//! 1. Payment processing
//! 2. Inventory reservation
//! 3. Shipment creation
//!
//! If any step fails, compensation actions are executed to rollback previous steps.

pub mod models;
pub mod tasks;
pub mod workflows;

use flovyn_sdk::prelude::*;
use tasks::{InventoryTask, PaymentTask, ShipmentTask};
use tracing::info;
use workflows::OrderWorkflow;

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
                .add_directive("ecommerce_sample=info".parse()?)
                .add_directive("flovyn_sdk=info".parse()?),
        )
        .init();

    info!("Starting E-commerce Order Processing Sample");

    // Parse configuration from environment
    let tenant_id = std::env::var("FLOVYN_TENANT_ID")
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
        tenant_id = %tenant_id,
        server = %format!("{}:{}", server_host, server_port),
        queue = %queue,
        "Connecting to Flovyn server"
    );

    // Build the client with fluent registration
    let client = FlovynClient::builder()
        .server_address(&server_host, server_port)
        .tenant_id(tenant_id)
        .worker_token(worker_token)
        .queue(&queue)
        .max_concurrent_workflows(10)
        .max_concurrent_tasks(20)
        .register_workflow(OrderWorkflow)
        .register_task(PaymentTask)
        .register_task(InventoryTask)
        .register_task(ShipmentTask)
        .build()
        .await?;

    info!(
        workflows = ?["ecommerce-order"],
        tasks = ?["payment-task", "inventory-task", "shipment-task"],
        "Registered workflows and tasks"
    );

    // Start the workers
    let handle = client.start().await?;

    info!("Workers started. Press Ctrl+C to stop.");
    info!("");
    info!("To test, start a workflow with:");
    info!("  curl -X POST http://localhost:8080/api/workflows/ecommerce-order \\");
    info!("    -H 'Content-Type: application/json' \\");
    info!("    -d '{{");
    info!("      \"order_id\": \"ORD-001\",");
    info!("      \"customer_id\": \"CUST-001\",");
    info!("      \"items\": [{{\"sku\": \"SKU-001\", \"quantity\": 2, \"price\": 29.99}}],");
    info!("      \"total_amount\": 59.98,");
    info!("      \"shipping_address\": {{");
    info!("        \"street\": \"123 Main St\",");
    info!("        \"city\": \"San Francisco\",");
    info!("        \"country\": \"USA\",");
    info!("        \"postal_code\": \"94102\"");
    info!("      }}");
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
    fn test_create_sample_order() {
        let order = OrderInput {
            order_id: "ORD-001".to_string(),
            customer_id: "CUST-001".to_string(),
            items: vec![
                OrderItem {
                    sku: "SKU-001".to_string(),
                    quantity: 2,
                    price: 29.99,
                },
                OrderItem {
                    sku: "SKU-002".to_string(),
                    quantity: 1,
                    price: 49.99,
                },
            ],
            total_amount: 109.97,
            shipping_address: Address {
                street: "123 Main St".to_string(),
                city: "San Francisco".to_string(),
                country: "USA".to_string(),
                postal_code: "94102".to_string(),
            },
        };

        assert_eq!(order.order_id, "ORD-001");
        assert_eq!(order.items.len(), 2);
    }
}
