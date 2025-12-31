//! Promise Workflow (External Signals)
//!
//! Demonstrates the use of durable promises for handling external signals.
//! This pattern is useful for:
//! - Human approval workflows
//! - Waiting for external events
//! - Integration with external systems

use async_trait::async_trait;
use flovyn_sdk::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

/// Request for approval
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub requester: String,
    pub amount: f64,
    pub description: String,
}

/// Decision made by approver
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalDecision {
    pub approved: bool,
    pub approver: String,
    pub comment: Option<String>,
}

/// Result of the approval workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalResult {
    pub request_id: String,
    pub approved: bool,
    pub approver: String,
    pub message: String,
}

/// A workflow that waits for external approval
///
/// This workflow:
/// 1. Sends an approval request notification
/// 2. Waits for an external signal (approval decision)
/// 3. Processes the decision and returns result
///
/// The promise is durable - if the worker restarts while waiting,
/// the workflow will continue waiting for the signal.
pub struct ApprovalWorkflow;

#[async_trait]
impl WorkflowDefinition for ApprovalWorkflow {
    type Input = ApprovalRequest;
    type Output = ApprovalResult;

    fn kind(&self) -> &str {
        "approval-workflow"
    }

    fn name(&self) -> &str {
        "Human Approval Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Demonstrates durable promises for human-in-the-loop workflows")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(86400) // 24 hour timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn tags(&self) -> Vec<String> {
        vec![
            "approval".to_string(),
            "promise".to_string(),
            "pattern".to_string(),
        ]
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            request_id = %input.request_id,
            requester = %input.requester,
            amount = input.amount,
            "Approval workflow started"
        );

        // Set workflow state
        ctx.set_raw("status", serde_json::to_value("PENDING_APPROVAL")?)
            .await?;
        ctx.set_raw("requestId", serde_json::to_value(&input.request_id)?)
            .await?;
        ctx.set_raw("requestedAmount", serde_json::to_value(input.amount)?)
            .await?;

        // Send approval request notification
        let notification = serde_json::to_value(format!(
            "Approval requested: {} for ${:.2} - {}",
            input.request_id, input.amount, input.description
        ))?;
        let _ = ctx.run_raw("send-approval-request", notification).await?;

        info!(
            request_id = %input.request_id,
            "Waiting for external approval signal"
        );

        // Wait for external approval signal with 24 hour timeout
        // In real usage, an external system or user would call the resolve API
        let approval_value = ctx
            .promise_with_timeout_raw("user-approval", Duration::from_secs(86400))
            .await?;

        let approval: ApprovalDecision = serde_json::from_value(approval_value)?;

        info!(
            request_id = %input.request_id,
            approved = approval.approved,
            approver = %approval.approver,
            "Received approval decision"
        );

        // Update state
        ctx.set_raw(
            "status",
            serde_json::to_value(if approval.approved {
                "APPROVED"
            } else {
                "REJECTED"
            })?,
        )
        .await?;
        ctx.set_raw("approver", serde_json::to_value(&approval.approver)?)
            .await?;

        // Process the decision
        let message = if approval.approved {
            format!(
                "Request {} approved by {}{}",
                input.request_id,
                approval.approver,
                approval
                    .comment
                    .map(|c| format!(": {}", c))
                    .unwrap_or_default()
            )
        } else {
            format!(
                "Request {} denied by {}{}",
                input.request_id,
                approval.approver,
                approval
                    .comment
                    .map(|c| format!(": {}", c))
                    .unwrap_or_default()
            )
        };

        // Record the decision processing
        let result_notification = serde_json::to_value(&message)?;
        let _ = ctx
            .run_raw("send-decision-notification", result_notification)
            .await?;

        Ok(ApprovalResult {
            request_id: input.request_id,
            approved: approval.approved,
            approver: approval.approver,
            message,
        })
    }
}

/// Input for multi-approval workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MultiApprovalRequest {
    pub request_id: String,
    pub required_approvers: Vec<String>,
    pub description: String,
}

/// Output from multi-approval workflow
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MultiApprovalResult {
    pub request_id: String,
    pub all_approved: bool,
    pub approvals: Vec<ApprovalDecision>,
    pub message: String,
}

/// A workflow requiring multiple approvals
///
/// Demonstrates waiting for multiple external signals before proceeding.
pub struct MultiApprovalWorkflow;

#[async_trait]
impl WorkflowDefinition for MultiApprovalWorkflow {
    type Input = MultiApprovalRequest;
    type Output = MultiApprovalResult;

    fn kind(&self) -> &str {
        "multi-approval-workflow"
    }

    fn name(&self) -> &str {
        "Multi-Approver Workflow"
    }

    fn version(&self) -> SemanticVersion {
        SemanticVersion::new(1, 0, 0)
    }

    fn description(&self) -> Option<&str> {
        Some("Requires approval from multiple parties")
    }

    fn timeout_seconds(&self) -> Option<u32> {
        Some(86400 * 7) // 7 day timeout
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        info!(
            request_id = %input.request_id,
            approvers = ?input.required_approvers,
            "Multi-approval workflow started"
        );

        let mut approvals = Vec::new();
        let mut all_approved = true;

        for approver in &input.required_approvers {
            // Check for cancellation between approvals
            ctx.check_cancellation().await?;

            info!(
                request_id = %input.request_id,
                approver = %approver,
                "Waiting for approval"
            );

            // Create unique promise name for each approver
            let promise_name = format!("approval-{}", approver);
            let approval_value = ctx
                .promise_with_timeout_raw(&promise_name, Duration::from_secs(86400))
                .await?;

            let decision: ApprovalDecision = serde_json::from_value(approval_value)?;

            info!(
                request_id = %input.request_id,
                approver = %approver,
                approved = decision.approved,
                "Received decision"
            );

            if !decision.approved {
                all_approved = false;
            }

            approvals.push(decision);
        }

        let message = if all_approved {
            format!(
                "Request {} approved by all {} approvers",
                input.request_id,
                approvals.len()
            )
        } else {
            let rejected_by: Vec<_> = approvals
                .iter()
                .filter(|a| !a.approved)
                .map(|a| a.approver.clone())
                .collect();
            format!(
                "Request {} rejected by: {}",
                input.request_id,
                rejected_by.join(", ")
            )
        };

        Ok(MultiApprovalResult {
            request_id: input.request_id,
            all_approved,
            approvals,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_approval_workflow_kind() {
        let workflow = ApprovalWorkflow;
        assert_eq!(workflow.kind(), "approval-workflow");
    }

    #[test]
    fn test_approval_request_serialization() {
        let request = ApprovalRequest {
            request_id: "REQ-001".to_string(),
            requester: "alice".to_string(),
            amount: 5000.0,
            description: "New laptop".to_string(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("REQ-001"));
        assert!(json.contains("5000"));
    }

    #[test]
    fn test_multi_approval_workflow_kind() {
        let workflow = MultiApprovalWorkflow;
        assert_eq!(workflow.kind(), "multi-approval-workflow");
    }
}
