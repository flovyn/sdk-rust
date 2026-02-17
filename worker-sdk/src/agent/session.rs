//! Session management for local agents.
//!
//! Sessions provide resume capability for local agents by tracking agent
//! state in a JSON file alongside the SQLite database.
//!
//! ## Session Directory Structure
//!
//! ```text
//! ~/.flovyn/sessions/{session_id}/
//!     session.json    # Session metadata
//!     agent.db        # SQLite database (managed by SqliteStorage)
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use flovyn_worker_sdk::agent::session::Session;
//!
//! // Create a new session
//! let session = Session::create("coder", "/home/user/project").await?;
//!
//! // List existing sessions
//! let sessions = Session::list().await?;
//!
//! // Resume a session
//! let (session, storage) = Session::resume(&session.session_id).await?;
//! ```

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::storage::SqliteStorage;
use crate::error::FlovynError;

/// Session status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Session is currently running
    Running,
    /// Session is suspended (waiting for signal or checkpoint)
    Suspended,
    /// Session completed successfully
    Completed,
    /// Session failed with an error
    Failed,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Suspended => write!(f, "suspended"),
            SessionStatus::Completed => write!(f, "completed"),
            SessionStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Session metadata for a local agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier
    pub session_id: String,
    /// Agent kind (e.g., "coder", "analyzer")
    pub agent_kind: String,
    /// Path to the SQLite database
    pub db_path: PathBuf,
    /// Workspace directory the agent operates in
    pub workspace: PathBuf,
    /// When the session was created
    pub created_at: DateTime<Utc>,
    /// When the session was last active
    pub last_activity: DateTime<Utc>,
    /// Current session status
    pub status: SessionStatus,
}

impl Session {
    /// Get the default sessions directory.
    ///
    /// Uses `~/.flovyn/sessions/` on all platforms.
    pub fn sessions_dir() -> Result<PathBuf, FlovynError> {
        let home = dirs_path()?;
        Ok(home.join(".flovyn").join("sessions"))
    }

    /// Get the directory for a specific session.
    fn session_dir(session_id: &str) -> Result<PathBuf, FlovynError> {
        Ok(Self::sessions_dir()?.join(session_id))
    }

    /// Get the path to the session.json file.
    fn session_file(session_id: &str) -> Result<PathBuf, FlovynError> {
        Ok(Self::session_dir(session_id)?.join("session.json"))
    }

    /// Create a new session.
    ///
    /// Creates the session directory, writes `session.json`, and initializes
    /// the SQLite database.
    pub async fn create(
        agent_kind: &str,
        workspace: impl AsRef<Path>,
    ) -> Result<(Self, SqliteStorage), FlovynError> {
        let session_id = Uuid::new_v4().to_string();
        let session_dir = Self::session_dir(&session_id)?;
        let db_path = session_dir.join("agent.db");

        // Create session directory
        tokio::fs::create_dir_all(&session_dir)
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to create session dir: {e}")))?;

        // Initialize SQLite database
        let storage = SqliteStorage::open(&db_path).await?;

        let now = Utc::now();
        let session = Session {
            session_id,
            agent_kind: agent_kind.to_string(),
            db_path,
            workspace: workspace.as_ref().to_path_buf(),
            created_at: now,
            last_activity: now,
            status: SessionStatus::Running,
        };

        // Write session.json
        session.save().await?;

        Ok((session, storage))
    }

    /// Create a session in a specific directory (for testing).
    pub async fn create_in(
        base_dir: impl AsRef<Path>,
        agent_kind: &str,
        workspace: impl AsRef<Path>,
    ) -> Result<(Self, SqliteStorage), FlovynError> {
        let session_id = Uuid::new_v4().to_string();
        let session_dir = base_dir.as_ref().join(&session_id);
        let db_path = session_dir.join("agent.db");

        tokio::fs::create_dir_all(&session_dir)
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to create session dir: {e}")))?;

        let storage = SqliteStorage::open(&db_path).await?;

        let now = Utc::now();
        let session = Session {
            session_id,
            agent_kind: agent_kind.to_string(),
            db_path,
            workspace: workspace.as_ref().to_path_buf(),
            created_at: now,
            last_activity: now,
            status: SessionStatus::Running,
        };

        session.save_to(&session_dir.join("session.json")).await?;

        Ok((session, storage))
    }

    /// Load a session from a session.json file in a directory.
    pub async fn load_from(session_dir: impl AsRef<Path>) -> Result<Self, FlovynError> {
        let session_file = session_dir.as_ref().join("session.json");
        let content = tokio::fs::read_to_string(&session_file)
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to read session file: {e}")))?;

        serde_json::from_str(&content)
            .map_err(|e| FlovynError::Other(format!("Failed to parse session file: {e}")))
    }

    /// Load a session by ID from the default sessions directory.
    pub async fn load(session_id: &str) -> Result<Self, FlovynError> {
        let session_file = Self::session_file(session_id)?;
        let content = tokio::fs::read_to_string(&session_file)
            .await
            .map_err(|e| FlovynError::Other(format!("Session '{session_id}' not found: {e}")))?;

        serde_json::from_str(&content)
            .map_err(|e| FlovynError::Other(format!("Failed to parse session file: {e}")))
    }

    /// List all sessions in the default sessions directory.
    pub async fn list() -> Result<Vec<Self>, FlovynError> {
        Self::list_in(&Self::sessions_dir()?).await
    }

    /// List all sessions in a specific directory.
    pub async fn list_in(sessions_dir: impl AsRef<Path>) -> Result<Vec<Self>, FlovynError> {
        let sessions_dir = sessions_dir.as_ref();
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = tokio::fs::read_dir(sessions_dir)
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to read sessions dir: {e}")))?;

        let mut sessions = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to read dir entry: {e}")))?
        {
            let path = entry.path();
            if path.is_dir() {
                let session_file = path.join("session.json");
                if session_file.exists() {
                    match Self::load_from(&path).await {
                        Ok(session) => sessions.push(session),
                        Err(_) => continue, // Skip invalid sessions
                    }
                }
            }
        }

        // Sort by last_activity descending (most recent first)
        sessions.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));

        Ok(sessions)
    }

    /// Resume a session from the default sessions directory.
    ///
    /// Loads the session metadata and opens the SQLite database.
    pub async fn resume(session_id: &str) -> Result<(Self, SqliteStorage), FlovynError> {
        let mut session = Self::load(session_id).await?;
        let storage = SqliteStorage::open(&session.db_path).await?;

        session.status = SessionStatus::Running;
        session.last_activity = Utc::now();
        session.save().await?;

        Ok((session, storage))
    }

    /// Resume a session from a specific directory.
    pub async fn resume_from(
        session_dir: impl AsRef<Path>,
    ) -> Result<(Self, SqliteStorage), FlovynError> {
        let mut session = Self::load_from(&session_dir).await?;
        let storage = SqliteStorage::open(&session.db_path).await?;

        session.status = SessionStatus::Running;
        session.last_activity = Utc::now();
        session.save_to(&session_dir.as_ref().join("session.json")).await?;

        Ok((session, storage))
    }

    /// Update the session status and save.
    pub async fn update_status(&mut self, status: SessionStatus) -> Result<(), FlovynError> {
        self.status = status;
        self.last_activity = Utc::now();
        self.save().await
    }

    /// Save session to the default location.
    async fn save(&self) -> Result<(), FlovynError> {
        let session_file = Self::session_file(&self.session_id)?;
        self.save_to(&session_file).await
    }

    /// Save session to a specific file.
    async fn save_to(&self, path: &Path) -> Result<(), FlovynError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| FlovynError::Other(format!("Failed to serialize session: {e}")))?;

        tokio::fs::write(path, content)
            .await
            .map_err(|e| FlovynError::Other(format!("Failed to write session file: {e}")))?;

        Ok(())
    }
}

/// Get the home directory path.
fn dirs_path() -> Result<PathBuf, FlovynError> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| FlovynError::Other("HOME environment variable not set".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_create_and_load() {
        let dir = tempfile::tempdir().unwrap();

        // Create a session
        let (session, _storage) = Session::create_in(dir.path(), "coder", "/tmp/project")
            .await
            .unwrap();

        assert_eq!(session.agent_kind, "coder");
        assert_eq!(session.workspace, PathBuf::from("/tmp/project"));
        assert_eq!(session.status, SessionStatus::Running);
        assert!(session.db_path.exists());

        // Load it back
        let session_dir = dir.path().join(&session.session_id);
        let loaded = Session::load_from(&session_dir).await.unwrap();
        assert_eq!(loaded.session_id, session.session_id);
        assert_eq!(loaded.agent_kind, "coder");
        assert_eq!(loaded.status, SessionStatus::Running);
    }

    #[tokio::test]
    async fn test_session_list() {
        let dir = tempfile::tempdir().unwrap();

        // Create multiple sessions
        let (s1, _) = Session::create_in(dir.path(), "coder", "/tmp/project1")
            .await
            .unwrap();
        let (s2, _) = Session::create_in(dir.path(), "analyzer", "/tmp/project2")
            .await
            .unwrap();

        // List sessions
        let sessions = Session::list_in(dir.path()).await.unwrap();
        assert_eq!(sessions.len(), 2);

        // Verify both sessions are present
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&s1.session_id.as_str()));
        assert!(ids.contains(&s2.session_id.as_str()));
    }

    #[tokio::test]
    async fn test_session_resume() {
        let dir = tempfile::tempdir().unwrap();

        // Create a session
        let (mut session, _storage) = Session::create_in(dir.path(), "coder", "/tmp/project")
            .await
            .unwrap();

        // Update status to suspended
        let session_dir = dir.path().join(&session.session_id);
        session.status = SessionStatus::Suspended;
        session
            .save_to(&session_dir.join("session.json"))
            .await
            .unwrap();

        // Resume it
        let (resumed, _storage) = Session::resume_from(&session_dir).await.unwrap();
        assert_eq!(resumed.session_id, session.session_id);
        assert_eq!(resumed.status, SessionStatus::Running);
    }

    #[test]
    fn test_session_status_display() {
        assert_eq!(SessionStatus::Running.to_string(), "running");
        assert_eq!(SessionStatus::Suspended.to_string(), "suspended");
        assert_eq!(SessionStatus::Completed.to_string(), "completed");
        assert_eq!(SessionStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_session_status_serialization() {
        let status = SessionStatus::Suspended;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"suspended\"");

        let parsed: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SessionStatus::Suspended);
    }
}
