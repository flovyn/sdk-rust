//! Test harness for E2E tests using Testcontainers
//!
//! Provides container orchestration for PostgreSQL, NATS, and Flovyn server,
//! using static API key authentication.
//!
//! All containers are started by the harness - no external dependencies required.
//!
//! # Environment Variables
//!
//! - `FLOVYN_TEST_KEEP_CONTAINERS=1` - Skip cleanup on exit (for debugging).
//!   Containers will remain running. Use `./bin/dev/cleanup-test-containers.sh`
//!   to clean up manually.

use serde::Deserialize;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use testcontainers::{
    core::{ContainerPort, Host, Mount, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use uuid::Uuid;

/// Unique session ID for this test run (used for container labeling)
static TEST_SESSION_ID: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| Uuid::new_v4().to_string()[..8].to_string());

/// Container IDs for cleanup (postgres, nats, server)
static CONTAINER_IDS: Mutex<Option<(String, String, String)>> = Mutex::new(None);

/// Register atexit handler once
pub static ATEXIT_REGISTERED: std::sync::Once = std::sync::Once::new();

/// Flag to track if cleanup has already run
static CLEANUP_RAN: AtomicBool = AtomicBool::new(false);

/// Register the atexit cleanup handler.
/// Call this before creating any containers.
pub fn register_atexit_handler() {
    ATEXIT_REGISTERED.call_once(|| {
        eprintln!("[HARNESS] Registering atexit cleanup handler");
        unsafe {
            libc::atexit(cleanup_on_exit);
        }
    });
}

/// Cleanup function called on process exit.
/// Note: Uses eprintln! instead of tracing because this runs after main exits,
/// when the tracing subscriber may be dropped or in an inconsistent state.
extern "C" fn cleanup_on_exit() {
    // Prevent double cleanup
    if CLEANUP_RAN.swap(true, Ordering::SeqCst) {
        return;
    }

    // Check if cleanup should be skipped (for debugging)
    if std::env::var("FLOVYN_TEST_KEEP_CONTAINERS").is_ok() {
        eprintln!("[HARNESS] FLOVYN_TEST_KEEP_CONTAINERS set - skipping cleanup");
        eprintln!("[HARNESS] Run ./bin/dev/cleanup-test-containers.sh to clean up manually");
        return;
    }

    eprintln!("[HARNESS] atexit cleanup triggered");

    // Stop and remove containers using docker CLI (can't use async here)
    if let Ok(guard) = CONTAINER_IDS.lock() {
        if let Some((pg_id, nats_id, server_id)) = guard.as_ref() {
            eprintln!(
                "[HARNESS] Stopping containers: {}, {}, {}",
                pg_id, nats_id, server_id
            );
            let _ = std::process::Command::new("docker")
                .args(["stop", "-t", "1", pg_id, nats_id, server_id])
                .output();

            eprintln!(
                "[HARNESS] Removing containers: {}, {}, {}",
                pg_id, nats_id, server_id
            );
            let _ = std::process::Command::new("docker")
                .args(["rm", "-f", pg_id, nats_id, server_id])
                .output();
        }
    }

    eprintln!("[HARNESS] atexit cleanup complete");
}

/// Test harness managing all containers and providing test utilities.
pub struct TestHarness {
    #[allow(dead_code)]
    postgres: ContainerAsync<GenericImage>,
    #[allow(dead_code)]
    nats: ContainerAsync<GenericImage>,
    #[allow(dead_code)]
    server: ContainerAsync<GenericImage>,
    /// Config file (kept alive for server to read)
    #[allow(dead_code)]
    _config_file: tempfile::NamedTempFile,
    server_grpc_port: u16,
    server_http_port: u16,
    org_id: Uuid,
    org_slug: String,
    /// Static API key for HTTP requests (User principal)
    api_key: String,
    /// Static API key for gRPC/SDK connections (Worker principal)
    worker_token: String,
}

impl TestHarness {
    /// Create a new test harness starting all required containers.
    /// Starts PostgreSQL, NATS, and Flovyn server containers.
    pub async fn new() -> Self {
        let session_id = TEST_SESSION_ID.as_str();
        println!("[HARNESS] Starting test harness (session: {})", session_id);

        // Start PostgreSQL container with labels
        let postgres: ContainerAsync<GenericImage> = GenericImage::new("postgres", "18-alpine")
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_exposed_port(ContainerPort::Tcp(5432))
            .with_env_var("POSTGRES_USER", "flovyn")
            .with_env_var("POSTGRES_PASSWORD", "flovyn")
            .with_env_var("POSTGRES_DB", "flovyn")
            .with_label("flovyn-test", "true")
            .with_label("flovyn-test-session", session_id)
            .start()
            .await
            .expect("Failed to start PostgreSQL");

        let pg_host_port = postgres.get_host_port_ipv4(5432).await.unwrap();
        println!("PostgreSQL started on port {}", pg_host_port);

        // Start NATS container with labels
        let nats: ContainerAsync<GenericImage> = GenericImage::new("nats", "latest")
            .with_wait_for(WaitFor::message_on_stderr("Server is ready"))
            .with_exposed_port(ContainerPort::Tcp(4222))
            .with_label("flovyn-test", "true")
            .with_label("flovyn-test-session", session_id)
            .start()
            .await
            .expect("Failed to start NATS");

        let nats_host_port = nats.get_host_port_ipv4(4222).await.unwrap();
        println!("NATS started on port {}", nats_host_port);

        // Generate test org and API keys
        let org_id = Uuid::new_v4();
        let org_slug = format!("test-{}", &Uuid::new_v4().to_string()[..8]);
        let api_key = format!("flovyn_sk_test_{}", &Uuid::new_v4().to_string()[..16]);
        let worker_token = format!("flovyn_wk_test_{}", &Uuid::new_v4().to_string()[..16]);

        // Create config file with org and static API keys
        let config_file = create_config_file(&org_id, &org_slug, &api_key, &worker_token);
        let config_path = config_file.path().to_string_lossy().to_string();
        println!(
            "[HARNESS] Created config file with pre-configured org and API keys at: {}",
            config_path
        );

        // Start the Flovyn server container with labels
        let server_image = GenericImage::new("rg.fr-par.scw.cloud/flovyn/flovyn-server", "latest")
            .with_exposed_port(ContainerPort::Tcp(8000))
            .with_exposed_port(ContainerPort::Tcp(9090))
            .with_label("flovyn-test", "true")
            .with_label("flovyn-test-session", session_id);

        let server: ContainerAsync<GenericImage> = server_image
            // Add host.docker.internal mapping for Linux (required for container to reach host ports)
            .with_host("host.docker.internal", Host::HostGateway)
            .with_env_var(
                "DATABASE_URL",
                format!(
                    "postgres://flovyn:flovyn@host.docker.internal:{}/flovyn",
                    pg_host_port
                ),
            )
            .with_env_var("NATS__ENABLED", "true")
            .with_env_var(
                "NATS__URL",
                format!("nats://host.docker.internal:{}", nats_host_port),
            )
            .with_env_var("SERVER_PORT", "8000")
            .with_env_var("GRPC_SERVER_PORT", "9090")
            // Mount config file and point to it
            .with_mount(Mount::bind_mount(&config_path, "/app/config.toml"))
            .with_env_var("CONFIG_FILE", "/app/config.toml")
            .with_startup_timeout(Duration::from_secs(120))
            .start()
            .await
            .expect("Failed to start Flovyn server");

        let server_grpc_port = server.get_host_port_ipv4(9090).await.unwrap();
        let server_http_port = server.get_host_port_ipv4(8000).await.unwrap();
        println!(
            "Flovyn server started - HTTP: {}, gRPC: {}",
            server_http_port, server_grpc_port
        );

        // Store container IDs for atexit cleanup
        let pg_id = postgres.id().to_string();
        let nats_id = nats.id().to_string();
        let server_id = server.id().to_string();
        if let Ok(mut guard) = CONTAINER_IDS.lock() {
            *guard = Some((pg_id.clone(), nats_id.clone(), server_id.clone()));
        }
        println!(
            "[HARNESS] Container IDs stored for cleanup: pg={}, nats={}, server={}",
            &pg_id[..12],
            &nats_id[..12],
            &server_id[..12]
        );

        // Wait for server to be ready (30s timeout, logs on failure)
        Self::wait_for_health(&server, server_http_port).await;

        Self {
            postgres,
            nats,
            server,
            _config_file: config_file,
            server_grpc_port,
            server_http_port,
            org_id,
            org_slug,
            api_key,
            worker_token,
        }
    }

    /// Wait for server health endpoint to respond (max 30 seconds).
    async fn wait_for_health(server: &ContainerAsync<GenericImage>, http_port: u16) {
        let client = reqwest::Client::new();
        let url = format!("http://localhost:{}/_/health", http_port);

        for i in 0..15 {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    println!("Server is healthy after {} seconds", i * 2);
                    return;
                }
                Ok(resp) => {
                    println!("Health check returned: {}", resp.status());
                }
                Err(_) => {
                    // Connection refused - server not ready yet
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        // Timeout - tell user to check logs
        let container_id = server.id();
        panic!(
            "Server health check timed out after 30 seconds.\nCheck logs with: docker logs {}",
            container_id
        );
    }

    pub fn grpc_host(&self) -> &str {
        "localhost"
    }

    pub fn grpc_port(&self) -> u16 {
        self.server_grpc_port
    }

    pub fn grpc_url(&self) -> String {
        format!("http://{}:{}", self.grpc_host(), self.grpc_port())
    }

    pub fn http_port(&self) -> u16 {
        self.server_http_port
    }

    pub fn org_id(&self) -> Uuid {
        self.org_id
    }

    pub fn org_slug(&self) -> &str {
        &self.org_slug
    }

    pub fn worker_token(&self) -> &str {
        &self.worker_token
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Get workflow events from the server for a given workflow execution.
    /// Returns the events as JSON value for replay testing.
    pub async fn get_workflow_events(
        &self,
        workflow_execution_id: &str,
    ) -> Vec<WorkflowEventResponse> {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();

        let url = format!(
            "{}/api/orgs/{}/workflow-executions/{}/events",
            base_url, self.org_slug, workflow_execution_id
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .expect("Failed to get workflow events");

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            panic!("Failed to get workflow events: {} - {}", status, body);
        }

        response
            .json()
            .await
            .expect("Failed to parse workflow events response")
    }

    /// Get workflow execution details from the server.
    pub async fn get_workflow_execution(
        &self,
        workflow_execution_id: &str,
    ) -> WorkflowExecutionResponse {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();

        let url = format!(
            "{}/api/orgs/{}/workflow-executions/{}",
            base_url, self.org_slug, workflow_execution_id
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .expect("Failed to get workflow execution");

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            panic!("Failed to get workflow execution: {} - {}", status, body);
        }

        response
            .json()
            .await
            .expect("Failed to parse workflow execution response")
    }

    // ========================================================================
    // Agent Execution API Methods
    // ========================================================================

    /// Create a new agent execution via REST API.
    pub async fn create_agent_execution(
        &self,
        kind: &str,
        input: serde_json::Value,
        queue: &str,
    ) -> AgentExecutionResponse {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();

        let url = format!("{}/api/orgs/{}/agent-executions", base_url, self.org_slug);

        let body = serde_json::json!({
            "kind": kind,
            "input": input,
            "queue": queue,
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .expect("Failed to create agent execution");

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            panic!("Failed to create agent execution: {} - {}", status, body);
        }

        response
            .json()
            .await
            .expect("Failed to parse agent execution response")
    }

    /// Get agent execution details from the server.
    pub async fn get_agent_execution(&self, agent_execution_id: &str) -> AgentExecutionResponse {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();

        let url = format!(
            "{}/api/orgs/{}/agent-executions/{}",
            base_url, self.org_slug, agent_execution_id
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .expect("Failed to get agent execution");

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            panic!("Failed to get agent execution: {} - {}", status, body);
        }

        response
            .json()
            .await
            .expect("Failed to parse agent execution response")
    }

    /// Signal an agent execution.
    pub async fn signal_agent_execution(
        &self,
        agent_execution_id: &str,
        signal_name: &str,
        signal_value: serde_json::Value,
    ) -> SignalAgentResponse {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();

        let url = format!(
            "{}/api/orgs/{}/agent-executions/{}/signal",
            base_url, self.org_slug, agent_execution_id
        );

        let body = serde_json::json!({
            "signalName": signal_name,
            "signalValue": signal_value,
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .expect("Failed to signal agent execution");

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            panic!("Failed to signal agent execution: {} - {}", status, body);
        }

        response
            .json()
            .await
            .expect("Failed to parse signal response")
    }

    /// Get agent checkpoints.
    pub async fn list_agent_checkpoints(
        &self,
        agent_execution_id: &str,
    ) -> Vec<AgentCheckpointResponse> {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();

        let url = format!(
            "{}/api/orgs/{}/agent-executions/{}/checkpoints",
            base_url, self.org_slug, agent_execution_id
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .expect("Failed to list agent checkpoints");

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            panic!("Failed to list agent checkpoints: {} - {}", status, body);
        }

        #[derive(Deserialize)]
        struct ListResponse {
            checkpoints: Vec<AgentCheckpointResponse>,
        }

        let list: ListResponse = response
            .json()
            .await
            .expect("Failed to parse checkpoints response");
        list.checkpoints
    }

    /// List tasks associated with an agent execution.
    pub async fn list_agent_tasks(&self, agent_execution_id: &str) -> Vec<TaskExecutionResponse> {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();

        let url = format!(
            "{}/api/orgs/{}/agent-executions/{}/tasks",
            base_url, self.org_slug, agent_execution_id
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .expect("Failed to list agent tasks");

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            panic!("Failed to list agent tasks: {} - {}", status, body);
        }

        #[derive(Deserialize)]
        struct ListResponse {
            tasks: Vec<TaskExecutionResponse>,
        }

        let list: ListResponse = response
            .json()
            .await
            .expect("Failed to parse tasks response");
        list.tasks
    }

    /// Wait for agent execution to reach a specific status (with timeout).
    pub async fn wait_for_agent_status(
        &self,
        agent_execution_id: &str,
        expected_statuses: &[&str],
        timeout: Duration,
    ) -> AgentExecutionResponse {
        let start = std::time::Instant::now();
        loop {
            let agent = self.get_agent_execution(agent_execution_id).await;
            if expected_statuses
                .iter()
                .any(|s| s.eq_ignore_ascii_case(&agent.status))
            {
                return agent;
            }
            if start.elapsed() > timeout {
                panic!(
                    "Timeout waiting for agent {} to reach status {:?}, current: {}",
                    agent_execution_id, expected_statuses, agent.status
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// Workflow event response from the server API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEventResponse {
    pub sequence_number: i32,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: String,
}

/// Workflow execution response from the server API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExecutionResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub workflow_kind: String,
    pub status: String,
    pub input: serde_json::Value,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

// ============================================================================
// Agent Execution API Types
// ============================================================================

/// Agent execution response from the server API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
    pub queue: String,
    pub current_checkpoint_seq: i32,
    pub created_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// Agent checkpoint response from the server API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCheckpointResponse {
    pub id: Uuid,
    pub agent_execution_id: Uuid,
    pub sequence: i32,
    #[serde(default)]
    pub state: Option<serde_json::Value>,
    pub created_at: String,
}

/// Signal response from the server API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalAgentResponse {
    pub signaled: bool,
    pub agent_resumed: bool,
}

/// Task execution response from the server API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskExecutionResponse {
    pub id: Uuid,
    #[serde(default)]
    pub org_id: Option<Uuid>,
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub queue: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// Create a temporary config file with org and static API key configuration
fn create_config_file(
    org_id: &Uuid,
    org_slug: &str,
    api_key: &str,
    worker_api_key: &str,
) -> tempfile::NamedTempFile {
    let config_content = format!(
        r#"
# Pre-configured organizations
[[orgs]]
id = "{org_id}"
name = "Test Organization"
slug = "{org_slug}"
tier = "FREE"

# Authentication configuration
[auth]
enabled = true

# Static API keys
[auth.static_api_key]
keys = [
    {{ key = "{api_key}", org_id = "{org_id}", principal_type = "User", principal_id = "api:test", role = "ADMIN" }},
    {{ key = "{worker_api_key}", org_id = "{org_id}", principal_type = "Worker", principal_id = "worker:test" }}
]

# Endpoint authentication
[auth.endpoints.http]
authenticators = ["static_api_key"]
authorizer = "cedar"

[auth.endpoints.grpc]
authenticators = ["static_api_key"]
authorizer = "cedar"
"#
    );

    let mut file = tempfile::Builder::new()
        .prefix("flovyn-test-config-")
        .suffix(".toml")
        .tempfile()
        .expect("Failed to create temp config file");

    file.write_all(config_content.as_bytes())
        .expect("Failed to write config file");

    file
}
