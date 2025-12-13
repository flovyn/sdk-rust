//! E-commerce tasks module

pub mod inventory_task;
pub mod payment_task;
pub mod shipment_task;

pub use inventory_task::InventoryTask;
pub use payment_task::PaymentTask;
pub use shipment_task::ShipmentTask;
