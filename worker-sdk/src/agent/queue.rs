//! Queue resolution for child agent spawning
//!
//! Implements the hierarchical queue resolution order described in the design doc:
//!
//! 1. LLM specifies queue explicitly → use that
//! 2. Agent definition has fixed queue → use that
//! 3. Agent definition has queue scope → resolve from session context
//! 4. Ad-hoc agent with mode "local" → session's default user queue
//! 5. Ad-hoc agent with mode "remote" → "default"
//! 6. No matching worker available → error

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
}

impl QueueContext {
    /// Create a new QueueContext from the worker's queue name.
    ///
    /// Extracts the user from the queue name if it follows the `user.{name}...` pattern.
    pub fn from_worker_queue(worker_queue: String) -> Self {
        let user = extract_user_from_queue(&worker_queue);
        Self {
            worker_queue,
            user,
        }
    }

    /// Resolve the target queue for a child agent.
    ///
    /// Follows the resolution order:
    /// 1. Explicit queue (from SpawnOptions) → use directly
    /// 2. Otherwise, fall back to the worker's own queue for local mode,
    ///    or "default" for remote mode
    pub fn resolve_queue(
        &self,
        explicit_queue: Option<&str>,
        mode: &str,
    ) -> String {
        // 1. Explicit queue takes priority
        if let Some(q) = explicit_queue {
            return q.to_string();
        }

        // 2-5. Fall back based on mode
        match mode {
            "LOCAL" => self.worker_queue.clone(),
            "REMOTE" => "default".to_string(),
            _ => "default".to_string(),
        }
    }
}

/// Auto-generate a queue name from workspace context.
///
/// Format: `user.{username}.{hostname}.{workspace_dir_name}`
///
/// Segments are sanitized to contain only lowercase alphanumeric characters and hyphens.
pub fn generate_queue_name(workspace: &Path) -> String {
    let username = whoami::username().to_lowercase();
    let hostname = sanitize_segment(&whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()));
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
    if queue.starts_with("user.") {
        let rest = &queue[5..]; // after "user."
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
    }

    #[test]
    fn test_from_worker_queue_no_user_prefix() {
        let ctx = QueueContext::from_worker_queue("default".to_string());
        assert_eq!(ctx.user, None);
    }

    #[test]
    fn test_from_worker_queue_shared_prefix() {
        let ctx = QueueContext::from_worker_queue("shared.ci-runners".to_string());
        assert_eq!(ctx.user, None);
    }

    #[test]
    fn test_resolve_explicit_queue() {
        let ctx = QueueContext::from_worker_queue("user.alice.laptop".to_string());
        let result = ctx.resolve_queue(Some("shared.ci-runners"), "REMOTE");
        assert_eq!(result, "shared.ci-runners");
    }

    #[test]
    fn test_resolve_local_mode_uses_worker_queue() {
        let ctx = QueueContext::from_worker_queue("user.alice.laptop".to_string());
        let result = ctx.resolve_queue(None, "LOCAL");
        assert_eq!(result, "user.alice.laptop");
    }

    #[test]
    fn test_resolve_remote_mode_uses_default() {
        let ctx = QueueContext::from_worker_queue("user.alice.laptop".to_string());
        let result = ctx.resolve_queue(None, "REMOTE");
        assert_eq!(result, "default");
    }

    #[test]
    fn test_resolve_explicit_overrides_mode() {
        let ctx = QueueContext::from_worker_queue("user.alice.laptop".to_string());
        // Even in LOCAL mode, explicit queue wins
        let result = ctx.resolve_queue(Some("user.bob.desktop"), "LOCAL");
        assert_eq!(result, "user.bob.desktop");
    }

    #[test]
    fn test_generate_queue_name_format() {
        let queue = generate_queue_name(Path::new("/home/alice/projects/my-app"));
        // Should follow user.{username}.{hostname}.{workspace} format
        assert!(queue.starts_with("user."), "Queue should start with 'user.': {}", queue);
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
