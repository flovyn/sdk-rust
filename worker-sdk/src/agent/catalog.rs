//! Agent catalog — merges server and local registries into a unified catalog.
//!
//! The catalog provides the LLM with a list of available agents for spawning,
//! including metadata about each agent's mode, capabilities, and description.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::registry::AgentMetadata;

/// An entry in the agent catalog visible to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Agent kind identifier
    pub kind: String,
    /// Execution mode (local or remote)
    pub mode: String,
    /// Human-readable description
    pub description: Option<String>,
}

/// Merged agent catalog from server + local registries.
#[derive(Debug, Clone, Default)]
pub struct AgentCatalog {
    entries: Vec<CatalogEntry>,
}

impl AgentCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add entries from the local worker's agent registry.
    pub fn add_local_agents(&mut self, metadata: &[AgentMetadata]) {
        for m in metadata {
            self.entries.push(CatalogEntry {
                kind: m.kind.clone(),
                mode: "local".to_string(),
                description: m.description.clone(),
            });
        }
    }

    /// Add entries from the server's agent definitions.
    ///
    /// Takes a list of (kind, description) pairs from the server.
    pub fn add_server_agents(&mut self, agents: &[(String, Option<String>)]) {
        for (kind, description) in agents {
            // Skip if already registered locally (local takes priority)
            if self.entries.iter().any(|e| e.kind == *kind) {
                continue;
            }
            self.entries.push(CatalogEntry {
                kind: kind.clone(),
                mode: "remote".to_string(),
                description: description.clone(),
            });
        }
    }

    /// Get all catalog entries.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Generate the `available_agents` JSON for injection into system prompts
    /// or the `spawn-agent` tool schema.
    pub fn to_available_agents_json(&self) -> Value {
        let agents: Vec<Value> = self
            .entries
            .iter()
            .map(|e| {
                json!({
                    "kind": e.kind,
                    "mode": e.mode,
                    "description": e.description.as_deref().unwrap_or("")
                })
            })
            .collect();
        json!({"available_agents": agents})
    }

    /// Generate the `agent_kind` enum values for the `spawn-agent` tool schema.
    pub fn agent_kind_enum(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.kind.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_local_metadata(kind: &str, description: &str) -> AgentMetadata {
        AgentMetadata {
            kind: kind.to_string(),
            name: kind.to_string(),
            description: Some(description.to_string()),
            version: None,
            tags: vec![],
            timeout_seconds: None,
            cancellable: true,
            input_schema: None,
            output_schema: None,
        }
    }

    #[test]
    fn test_catalog_local_agents() {
        let mut catalog = AgentCatalog::new();
        let metadata = vec![
            make_local_metadata("coder", "Coding agent"),
            make_local_metadata("reviewer", "Code review"),
        ];
        catalog.add_local_agents(&metadata);

        assert_eq!(catalog.entries().len(), 2);
        assert_eq!(catalog.entries()[0].mode, "local");
        assert_eq!(catalog.entries()[1].kind, "reviewer");
    }

    #[test]
    fn test_catalog_server_agents() {
        let mut catalog = AgentCatalog::new();
        catalog.add_server_agents(&[
            ("researcher".to_string(), Some("Web research".to_string())),
            ("summarizer".to_string(), None),
        ]);

        assert_eq!(catalog.entries().len(), 2);
        assert_eq!(catalog.entries()[0].mode, "remote");
    }

    #[test]
    fn test_catalog_merge_local_takes_priority() {
        let mut catalog = AgentCatalog::new();
        let metadata = vec![make_local_metadata("coder", "Local coder")];
        catalog.add_local_agents(&metadata);

        // Server also has "coder" — should be skipped
        catalog.add_server_agents(&[("coder".to_string(), Some("Remote coder".to_string()))]);

        assert_eq!(catalog.entries().len(), 1);
        assert_eq!(catalog.entries()[0].mode, "local");
        assert_eq!(
            catalog.entries()[0].description.as_deref(),
            Some("Local coder")
        );
    }

    #[test]
    fn test_catalog_to_json() {
        let mut catalog = AgentCatalog::new();
        let metadata = vec![make_local_metadata("coder", "Coding agent")];
        catalog.add_local_agents(&metadata);
        catalog.add_server_agents(&[("researcher".to_string(), Some("Research".to_string()))]);

        let json = catalog.to_available_agents_json();
        let agents = json["available_agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["kind"], "coder");
        assert_eq!(agents[0]["mode"], "local");
        assert_eq!(agents[1]["kind"], "researcher");
        assert_eq!(agents[1]["mode"], "remote");
    }

    #[test]
    fn test_agent_kind_enum() {
        let mut catalog = AgentCatalog::new();
        let metadata = vec![
            make_local_metadata("coder", ""),
            make_local_metadata("reviewer", ""),
        ];
        catalog.add_local_agents(&metadata);

        let kinds = catalog.agent_kind_enum();
        assert_eq!(kinds, vec!["coder".to_string(), "reviewer".to_string()]);
    }
}
