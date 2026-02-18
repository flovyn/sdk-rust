//! Queue resolution for child agent spawning
//!
//! Resolution order:
//!
//! 1. Explicit queue in SpawnOptions → use that
//! 2. Template has default_queue → use that (resolved server-side)
//! 3. No template, no explicit queue → inherit parent's queue (worker_queue)

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Session context for queue resolution.
///
/// Populated by the worker when creating the agent context, based on the
/// worker's registration and the user's session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueContext {
    /// The worker's own queue (e.g., "user.alice.laptop.project-x")
    pub worker_queue: String,
    /// Username extracted from the worker queue (e.g., "alice")
    pub user: Option<String>,
    /// The session's default user queue, propagated from the original agent
    /// execution. For local workers this equals `worker_queue`; for remote
    /// workers it comes from the agent execution metadata so that a remote
    /// parent can spawn LOCAL children on the user's local queue.
    pub session_default_queue: Option<String>,
}

impl QueueContext {
    /// Create a new QueueContext from the worker's queue name.
    ///
    /// Extracts the user from the queue name if it follows the `user.{name}...` pattern.
    /// For user queues, `session_default_queue` is set to the worker queue automatically.
    pub fn from_worker_queue(worker_queue: String) -> Self {
        let user = extract_user_from_queue(&worker_queue);
        // If this is a user queue, it IS the session's default user queue
        let session_default_queue = if user.is_some() {
            Some(worker_queue.clone())
        } else {
            None
        };
        Self {
            worker_queue,
            user,
            session_default_queue,
        }
    }

    /// Set the session default queue explicitly (e.g., propagated from server).
    pub fn with_session_queue(mut self, queue: String) -> Self {
        self.session_default_queue = Some(queue);
        self
    }

    /// Resolve the target queue for a child agent.
    ///
    /// Resolution order:
    /// 1. Explicit queue (from SpawnOptions) → use directly
    /// 2. No explicit queue → inherit worker queue (parent's queue)
    ///
    /// Template-based routing (template's `default_queue`) is resolved
    /// server-side in `spawn_child_agent`, not here.
    pub fn resolve_queue(&self, explicit_queue: Option<&str>) -> String {
        if let Some(q) = explicit_queue {
            return q.to_string();
        }

        // Inherit parent's queue
        self.worker_queue.clone()
    }
}

/// Auto-generate a queue name from workspace context.
///
/// Format: `user.{username}.{hostname}.{workspace_dir_name}`
///
/// Segments are sanitized to contain only lowercase alphanumeric characters and hyphens.
pub fn generate_queue_name(workspace: &Path) -> String {
    let username = whoami::username().to_lowercase();
    let hostname =
        sanitize_segment(&whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()));
    let workspace_name = workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");
    let workspace_name = sanitize_segment(workspace_name);

    format!("user.{username}.{hostname}.{workspace_name}")
}

/// Sanitize a string for use as a queue segment.
///
/// Replaces non-alphanumeric characters (except hyphens) with hyphens,
/// converts to lowercase, and trims leading/trailing hyphens.
fn sanitize_segment(s: &str) -> String {
    let sanitized: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    sanitized.trim_matches('-').to_string()
}

/// Extract username from a queue name following the `user.{name}...` pattern.
fn extract_user_from_queue(queue: &str) -> Option<String> {
    if let Some(rest) = queue.strip_prefix("user.") {
        // Take everything up to the next dot (or end of string)
        let user = rest.split('.').next()?;
        if !user.is_empty() {
            return Some(user.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_worker_queue_extracts_user() {
        let ctx = QueueContext::from_worker_queue("user.alice.laptop.project-x".to_string());
        assert_eq!(ctx.user, Some("alice".to_string()));
        assert_eq!(ctx.worker_queue, "user.alice.laptop.project-x");
        // User queue → session_default_queue is set automatically
        assert_eq!(
            ctx.session_default_queue,
            Some("user.alice.laptop.project-x".to_string())
        );
    }

    #[test]
    fn test_from_worker_queue_no_user_prefix() {
        let ctx = QueueContext::from_worker_queue("default".to_string());
        assert_eq!(ctx.user, None);
        // Non-user queue → no session_default_queue
        assert_eq!(ctx.session_default_queue, None);
    }

    #[test]
    fn test_from_worker_queue_shared_prefix() {
        let ctx = QueueContext::from_worker_queue("shared.ci-runners".to_string());
        assert_eq!(ctx.user, None);
        assert_eq!(ctx.session_default_queue, None);
    }

    #[test]
    fn test_resolve_explicit_queue() {
        let ctx = QueueContext::from_worker_queue("user.alice.laptop".to_string());
        let result = ctx.resolve_queue(Some("shared.ci-runners"));
        assert_eq!(result, "shared.ci-runners");
    }

    #[test]
    fn test_resolve_explicit_overrides_default() {
        let ctx = QueueContext::from_worker_queue("user.alice.laptop".to_string());
        let result = ctx.resolve_queue(Some("user.bob.desktop"));
        assert_eq!(result, "user.bob.desktop");
    }

    // --- Inherit parent queue tests ---

    #[test]
    fn test_resolve_inherits_worker_queue() {
        // No explicit queue → inherit worker (parent) queue
        let ctx = QueueContext::from_worker_queue("user.alice.laptop".to_string());
        let result = ctx.resolve_queue(None);
        assert_eq!(result, "user.alice.laptop");
    }

    #[test]
    fn test_resolve_inherits_non_user_queue() {
        // Non-user queue also inherited
        let ctx = QueueContext::from_worker_queue("manager".to_string());
        let result = ctx.resolve_queue(None);
        assert_eq!(result, "manager");
    }

    #[test]
    fn test_resolve_inherits_default_queue() {
        let ctx = QueueContext::from_worker_queue("default".to_string());
        let result = ctx.resolve_queue(None);
        assert_eq!(result, "default");
    }

    #[test]
    fn test_generate_queue_name_format() {
        let queue = generate_queue_name(Path::new("/home/alice/projects/my-app"));
        // Should follow user.{username}.{hostname}.{workspace} format
        assert!(
            queue.starts_with("user."),
            "Queue should start with 'user.': {}",
            queue
        );
        let parts: Vec<&str> = queue.split('.').collect();
        assert_eq!(parts.len(), 4, "Queue should have 4 segments: {}", queue);
        assert_eq!(parts[0], "user");
        // parts[1] = username, parts[2] = hostname, parts[3] = workspace dir name
        assert_eq!(parts[3], "my-app");
    }

    #[test]
    fn test_sanitize_segment() {
        assert_eq!(sanitize_segment("My-Project"), "my-project");
        assert_eq!(sanitize_segment("project_name"), "project-name");
        assert_eq!(sanitize_segment("Hello World!"), "hello-world");
        assert_eq!(sanitize_segment("  spaces  "), "spaces");
    }
}
