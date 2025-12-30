//! WorkerLifecycleClient - gRPC client for worker registration and heartbeat
//!
//! This module provides the client for the WorkerLifecycle gRPC service,
//! which handles worker registration and heartbeat operations.

use crate::client::auth::AuthInterceptor;
use crate::error::CoreResult;
use crate::generated::flovyn_v1::{
    self, TaskCapability, WorkerHeartbeatRequest, WorkerRegistrationRequest,
    WorkerRegistrationResponse, WorkflowCapability,
};
use crate::task::TaskMetadata;
use crate::worker::{TaskConflict, WorkflowConflict};
use crate::workflow::WorkflowMetadata;
use sha2::{Digest, Sha256};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use uuid::Uuid;

/// Type alias for authenticated client
type AuthClient = flovyn_v1::worker_lifecycle_client::WorkerLifecycleClient<
    InterceptedService<Channel, AuthInterceptor>,
>;

/// Worker types for registration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerType {
    /// Worker that only executes workflows
    Workflow,
    /// Worker that only executes tasks
    Task,
    /// Worker that executes both workflows and tasks
    Unified,
}

impl WorkerType {
    /// Convert to string representation for protobuf
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkerType::Workflow => "WORKFLOW",
            WorkerType::Task => "TASK",
            WorkerType::Unified => "UNIFIED",
        }
    }
}

impl std::fmt::Display for WorkerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Result from worker registration
#[derive(Debug, Clone)]
pub struct RegistrationResult {
    /// The server-assigned worker ID
    pub worker_id: Uuid,
    /// Whether registration was successful
    pub success: bool,
    /// Error message if registration failed
    pub error: Option<String>,
    /// Workflow conflicts if any
    pub workflow_conflicts: Vec<WorkflowConflict>,
    /// Task conflicts if any
    pub task_conflicts: Vec<TaskConflict>,
}

/// gRPC client for WorkerLifecycle service.
/// Handles worker registration and heartbeat operations.
pub struct WorkerLifecycleClient {
    stub: AuthClient,
}

impl WorkerLifecycleClient {
    /// Create a new WorkerLifecycleClient with authentication
    pub fn new(channel: Channel, token: &str) -> Self {
        let interceptor = AuthInterceptor::worker_token(token);
        Self {
            stub: flovyn_v1::worker_lifecycle_client::WorkerLifecycleClient::with_interceptor(
                channel,
                interceptor,
            ),
        }
    }

    /// Register worker with workflows and tasks
    ///
    /// # Arguments
    /// * `worker_name` - Human-readable name for the worker
    /// * `worker_version` - Version string for the worker
    /// * `worker_type` - Type of worker (Workflow, Task, or Unified)
    /// * `tenant_id` - Tenant ID for multi-tenancy
    /// * `space_id` - Optional space ID (None = tenant-level)
    /// * `workflows` - List of workflow metadata to register
    /// * `tasks` - List of task metadata to register
    #[allow(clippy::too_many_arguments)]
    pub async fn register_worker(
        &mut self,
        worker_name: &str,
        worker_version: &str,
        worker_type: WorkerType,
        tenant_id: Uuid,
        space_id: Option<Uuid>,
        workflows: Vec<WorkflowMetadata>,
        tasks: Vec<TaskMetadata>,
    ) -> CoreResult<RegistrationResult> {
        let host_name = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let process_id = std::process::id().to_string();

        let request = WorkerRegistrationRequest {
            worker_name: worker_name.to_string(),
            worker_version: worker_version.to_string(),
            worker_type: worker_type.as_str().to_string(),
            tenant_id: tenant_id.to_string(),
            space_id: space_id.map(|id| id.to_string()),
            host_name,
            process_id,
            workflows: workflows.into_iter().map(workflow_to_proto).collect(),
            tasks: tasks.into_iter().map(task_to_proto).collect(),
            metadata: Vec::new(),
        };

        let response = self.stub.register_worker(request).await?.into_inner();

        Ok(registration_response_to_result(response))
    }

    /// Send heartbeat to update worker status
    ///
    /// # Arguments
    /// * `worker_id` - The server-assigned worker ID from registration
    pub async fn send_heartbeat(&mut self, worker_id: Uuid) -> CoreResult<()> {
        let request = WorkerHeartbeatRequest {
            worker_id: worker_id.to_string(),
            workflow_kinds: Vec::new(),
            task_kinds: Vec::new(),
            health_info: Vec::new(),
        };

        self.stub.send_heartbeat(request).await?;

        Ok(())
    }

    /// Send heartbeat with current workflow and task kinds
    ///
    /// # Arguments
    /// * `worker_id` - The server-assigned worker ID from registration
    /// * `workflow_kinds` - Currently registered workflow kinds
    /// * `task_kinds` - Currently registered task kinds
    pub async fn send_heartbeat_with_kinds(
        &mut self,
        worker_id: Uuid,
        workflow_kinds: Vec<String>,
        task_kinds: Vec<String>,
    ) -> CoreResult<()> {
        let request = WorkerHeartbeatRequest {
            worker_id: worker_id.to_string(),
            workflow_kinds,
            task_kinds,
            health_info: Vec::new(),
        };

        self.stub.send_heartbeat(request).await?;

        Ok(())
    }
}

/// Convert WorkflowMetadata to protobuf WorkflowCapability
fn workflow_to_proto(metadata: WorkflowMetadata) -> WorkflowCapability {
    let version = metadata.version.unwrap_or_else(|| "1.0.0".to_string());

    // Use the content hash from metadata, or generate one from kind:version
    let content_hash = metadata
        .content_hash
        .unwrap_or_else(|| compute_content_hash(&metadata.kind, &version));

    WorkflowCapability {
        kind: metadata.kind,
        name: metadata.name,
        description: metadata.description.unwrap_or_default(),
        timeout_seconds: metadata.timeout_seconds.map(|s| s as i32),
        cancellable: metadata.cancellable,
        tags: metadata.tags,
        retry_policy: Vec::new(),
        metadata: Vec::new(),
        version,
        content_hash,
    }
}

/// Compute SHA-256 content hash for workflow versioning.
/// Uses kind:version as the hash input (simplified approach).
fn compute_content_hash(kind: &str, version: &str) -> String {
    let hash_source = format!("{}:{}", kind, version);
    let mut hasher = Sha256::new();
    hasher.update(hash_source.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Convert TaskMetadata to protobuf TaskCapability
fn task_to_proto(metadata: TaskMetadata) -> TaskCapability {
    TaskCapability {
        kind: metadata.kind,
        name: metadata.name,
        description: metadata.description.unwrap_or_default(),
        timeout_seconds: metadata.timeout_seconds.map(|s| s as i32),
        cancellable: metadata.cancellable,
        tags: metadata.tags,
        retry_policy: Vec::new(),
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        metadata: Vec::new(),
    }
}

/// Convert WorkerRegistrationResponse to RegistrationResult
fn registration_response_to_result(response: WorkerRegistrationResponse) -> RegistrationResult {
    let worker_id = Uuid::parse_str(&response.worker_id).unwrap_or_else(|_| Uuid::nil());

    RegistrationResult {
        worker_id,
        success: response.success,
        error: response.error,
        workflow_conflicts: response
            .workflow_conflicts
            .into_iter()
            .map(|c| WorkflowConflict {
                kind: c.kind,
                reason: c.reason,
                existing_worker_id: c.existing_worker_id,
            })
            .collect(),
        task_conflicts: response
            .task_conflicts
            .into_iter()
            .map(|c| TaskConflict {
                kind: c.kind,
                reason: c.reason,
                existing_worker_id: c.existing_worker_id,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_type_as_str() {
        assert_eq!(WorkerType::Workflow.as_str(), "WORKFLOW");
        assert_eq!(WorkerType::Task.as_str(), "TASK");
        assert_eq!(WorkerType::Unified.as_str(), "UNIFIED");
    }

    #[test]
    fn test_worker_type_display() {
        assert_eq!(format!("{}", WorkerType::Workflow), "WORKFLOW");
        assert_eq!(format!("{}", WorkerType::Task), "TASK");
        assert_eq!(format!("{}", WorkerType::Unified), "UNIFIED");
    }

    #[test]
    fn test_workflow_to_proto() {
        let metadata = WorkflowMetadata::new("order-workflow")
            .with_name("Order Workflow")
            .with_description("Processes orders")
            .with_version("1.2.0")
            .with_tags(vec!["order".to_string(), "payment".to_string()])
            .with_cancellable(true)
            .with_timeout_seconds(300);

        let proto = workflow_to_proto(metadata);

        assert_eq!(proto.kind, "order-workflow");
        assert_eq!(proto.name, "Order Workflow");
        assert_eq!(proto.description, "Processes orders");
        assert_eq!(proto.version, "1.2.0");
        assert!(proto.cancellable);
        assert_eq!(proto.timeout_seconds, Some(300));
        assert_eq!(proto.tags, vec!["order", "payment"]);
        // Content hash should be generated
        assert!(!proto.content_hash.is_empty());
    }

    #[test]
    fn test_workflow_to_proto_defaults() {
        let metadata = WorkflowMetadata::new("simple-workflow");

        let proto = workflow_to_proto(metadata);

        assert_eq!(proto.kind, "simple-workflow");
        assert_eq!(proto.name, "simple-workflow");
        assert_eq!(proto.description, "");
        assert_eq!(proto.version, "1.0.0"); // Default version
        assert!(proto.cancellable);
        assert_eq!(proto.timeout_seconds, None);
        assert!(proto.tags.is_empty());
        // Content hash should be generated even without explicit version
        assert!(!proto.content_hash.is_empty());
    }

    #[test]
    fn test_task_to_proto() {
        let metadata = TaskMetadata::new("process-image")
            .with_name("Process Image")
            .with_description("Processes images")
            .with_version("2.0.1")
            .with_tags(vec!["image".to_string(), "ml".to_string()])
            .with_timeout_seconds(600)
            .with_cancellable(true);

        let proto = task_to_proto(metadata);

        assert_eq!(proto.kind, "process-image");
        assert_eq!(proto.name, "Process Image");
        assert_eq!(proto.description, "Processes images");
        assert!(proto.cancellable);
        assert_eq!(proto.timeout_seconds, Some(600));
        assert_eq!(proto.tags, vec!["image", "ml"]);
    }

    #[test]
    fn test_task_to_proto_defaults() {
        let metadata = TaskMetadata::new("simple-task");

        let proto = task_to_proto(metadata);

        assert_eq!(proto.kind, "simple-task");
        assert_eq!(proto.name, "simple-task");
        assert_eq!(proto.description, "");
        assert!(proto.cancellable);
        assert_eq!(proto.timeout_seconds, None);
        assert!(proto.tags.is_empty());
    }

    #[test]
    fn test_registration_result_from_response() {
        let worker_id = Uuid::new_v4();
        let response = WorkerRegistrationResponse {
            worker_id: worker_id.to_string(),
            success: true,
            error: None,
            workflow_conflicts: vec![],
            task_conflicts: vec![],
        };

        let result = registration_response_to_result(response);

        assert_eq!(result.worker_id, worker_id);
        assert!(result.success);
        assert!(result.error.is_none());
        assert!(result.workflow_conflicts.is_empty());
        assert!(result.task_conflicts.is_empty());
    }

    #[test]
    fn test_registration_result_with_error() {
        let response = WorkerRegistrationResponse {
            worker_id: Uuid::nil().to_string(),
            success: false,
            error: Some("Registration failed".to_string()),
            workflow_conflicts: vec![],
            task_conflicts: vec![],
        };

        let result = registration_response_to_result(response);

        assert_eq!(result.worker_id, Uuid::nil());
        assert!(!result.success);
        assert_eq!(result.error, Some("Registration failed".to_string()));
    }

    #[test]
    fn test_registration_result_with_conflicts() {
        let response = WorkerRegistrationResponse {
            worker_id: Uuid::new_v4().to_string(),
            success: false,
            error: None,
            workflow_conflicts: vec![flovyn_v1::WorkflowConflict {
                kind: "order-workflow".to_string(),
                reason: "Already registered".to_string(),
                existing_worker_id: "worker-123".to_string(),
            }],
            task_conflicts: vec![flovyn_v1::TaskConflict {
                kind: "payment-task".to_string(),
                reason: "Version conflict".to_string(),
                existing_worker_id: "worker-456".to_string(),
            }],
        };

        let result = registration_response_to_result(response);

        assert!(!result.success);
        assert_eq!(result.workflow_conflicts.len(), 1);
        assert_eq!(result.workflow_conflicts[0].kind, "order-workflow");
        assert_eq!(result.workflow_conflicts[0].reason, "Already registered");
        assert_eq!(
            result.workflow_conflicts[0].existing_worker_id,
            "worker-123"
        );

        assert_eq!(result.task_conflicts.len(), 1);
        assert_eq!(result.task_conflicts[0].kind, "payment-task");
        assert_eq!(result.task_conflicts[0].reason, "Version conflict");
        assert_eq!(result.task_conflicts[0].existing_worker_id, "worker-456");
    }

    #[test]
    fn test_registration_result_debug() {
        let result = RegistrationResult {
            worker_id: Uuid::nil(),
            success: true,
            error: None,
            workflow_conflicts: vec![],
            task_conflicts: vec![],
        };

        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("RegistrationResult"));
        assert!(debug_str.contains("success: true"));
    }
}
