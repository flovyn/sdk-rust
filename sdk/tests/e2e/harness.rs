//! Test harness for E2E tests using Testcontainers
//!
//! Provides container orchestration for PostgreSQL, NATS, and Flovyn server,
//! plus JWT generation and REST API client for test setup.
//!
//! All containers are started by the harness - no external dependencies required.
//!
//! # Environment Variables
//!
//! - `FLOVYN_TEST_KEEP_CONTAINERS=1` - Skip cleanup on exit (for debugging).
//!   Containers will remain running. Use `./bin/dev/cleanup-test-containers.sh`
//!   to clean up manually.

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use testcontainers::{
    core::{ContainerPort, WaitFor},
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
    server_grpc_port: u16,
    server_http_port: u16,
    tenant_id: Uuid,
    tenant_slug: String,
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

        // Start the Flovyn server container with labels
        // Image name can be overridden via FLOVYN_SERVER_IMAGE env var
        let image_name = std::env::var("FLOVYN_SERVER_IMAGE")
            .unwrap_or_else(|_| "flovyn-server-test".to_string());
        let server_image = GenericImage::new(image_name, "latest".to_string())
            .with_exposed_port(ContainerPort::Tcp(8000))
            .with_exposed_port(ContainerPort::Tcp(9090))
            .with_label("flovyn-test", "true")
            .with_label("flovyn-test-session", session_id);

        let server: ContainerAsync<GenericImage> = server_image
            .with_env_var(
                "SPRING_DATASOURCE_URL",
                format!(
                    "jdbc:postgresql://host.docker.internal:{}/flovyn",
                    pg_host_port
                ),
            )
            .with_env_var("SPRING_DATASOURCE_USERNAME", "flovyn")
            .with_env_var("SPRING_DATASOURCE_PASSWORD", "flovyn")
            .with_env_var("NATS_ENABLED", "true")
            .with_env_var(
                "NATS_SERVERS_0",
                format!("nats://host.docker.internal:{}", nats_host_port),
            )
            .with_env_var("SERVER_PORT", "8000")
            .with_env_var("GRPC_SERVER_PORT", "9090")
            // Enable security with signature verification skipped (for self-signed JWTs)
            .with_env_var("FLOVYN_SECURITY_ENABLED", "true")
            .with_env_var("FLOVYN_SECURITY_JWT_SKIP_SIGNATURE_VERIFICATION", "true")
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

        // Create test tenant and worker token via REST API
        let (tenant_id, tenant_slug, worker_token) =
            Self::setup_test_tenant(server_http_port).await;

        Self {
            postgres,
            nats,
            server,
            server_grpc_port,
            server_http_port,
            tenant_id,
            tenant_slug,
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

    /// Setup test tenant and worker token via REST API.
    /// Uses a self-signed JWT for authentication.
    async fn setup_test_tenant(http_port: u16) -> (Uuid, String, String) {
        let base_url = format!("http://localhost:{}", http_port);
        let client = reqwest::Client::new();

        // Generate JWT with userId claim
        let jwt = Self::generate_test_jwt();

        // Create tenant via REST API
        let tenant_slug = format!("test-{}", &Uuid::new_v4().to_string()[..8]);
        let tenant_response = client
            .post(format!("{}/api/tenants", base_url))
            .header("Authorization", format!("Bearer {}", jwt))
            .json(&serde_json::json!({
                "name": "Test Tenant",
                "slug": tenant_slug,
                "tier": "FREE",
                "region": "us-west-2",
            }))
            .send()
            .await
            .expect("Failed to create tenant");

        if !tenant_response.status().is_success() {
            let status = tenant_response.status();
            let body = tenant_response.text().await.unwrap_or_default();
            panic!("Failed to create tenant: {} - {}", status, body);
        }

        let tenant: TenantResponse = tenant_response
            .json()
            .await
            .expect("Failed to parse tenant response");

        // Create worker token via REST API
        let token_response = client
            .post(format!(
                "{}/api/tenants/{}/worker-tokens",
                base_url, tenant.slug
            ))
            .header("Authorization", format!("Bearer {}", jwt))
            .json(&serde_json::json!({
                "displayName": "e2e-test-worker",
            }))
            .send()
            .await
            .expect("Failed to create worker token");

        if !token_response.status().is_success() {
            let status = token_response.status();
            let body = token_response.text().await.unwrap_or_default();
            panic!("Failed to create worker token: {} - {}", status, body);
        }

        let token: WorkerTokenResponse = token_response
            .json()
            .await
            .expect("Failed to parse worker token response");

        (tenant.id, tenant.slug, token.token)
    }

    pub fn grpc_host(&self) -> &str {
        "localhost"
    }

    pub fn grpc_port(&self) -> u16 {
        self.server_grpc_port
    }

    pub fn http_port(&self) -> u16 {
        self.server_http_port
    }

    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    pub fn tenant_slug(&self) -> &str {
        &self.tenant_slug
    }

    pub fn worker_token(&self) -> &str {
        &self.worker_token
    }

    /// Get workflow events from the server for a given workflow execution.
    /// Returns the events as JSON value for replay testing.
    pub async fn get_workflow_events(
        &self,
        workflow_execution_id: &str,
    ) -> Vec<WorkflowEventResponse> {
        let base_url = format!("http://localhost:{}", self.server_http_port);
        let client = reqwest::Client::new();
        let jwt = Self::generate_test_jwt();

        let url = format!(
            "{}/api/tenants/{}/workflow-executions/{}/events",
            base_url, self.tenant_slug, workflow_execution_id
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", jwt))
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
        let jwt = Self::generate_test_jwt();

        let url = format!(
            "{}/api/tenants/{}/workflow-executions/{}",
            base_url, self.tenant_slug, workflow_execution_id
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", jwt))
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

    /// Generate a self-signed JWT for REST API authentication.
    /// The server with security disabled accepts any JWT with valid structure.
    fn generate_test_jwt() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Use a random string ID like Better Auth does
        let user_id = format!("test-user-{}", &Uuid::new_v4().to_string()[..8]);

        let claims = JwtClaims {
            sub: user_id.clone(),
            id: user_id,
            name: "E2E Test User".to_string(),
            email: "e2e-test@example.com".to_string(),
            iss: "http://localhost:3000".to_string(),
            aud: "flovyn-server".to_string(),
            exp: now + 3600, // 1 hour from now
            iat: now,
        };

        let key = EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY.as_bytes())
            .expect("Failed to create encoding key");

        encode(&Header::new(Algorithm::RS256), &claims, &key).expect("Failed to encode JWT")
    }
}

#[derive(Debug, Serialize)]
struct JwtClaims {
    sub: String,
    id: String,
    name: String,
    email: String,
    iss: String,
    aud: String,
    exp: i64,
    iat: i64,
}

#[derive(Debug, Deserialize)]
struct TenantResponse {
    id: Uuid,
    slug: String,
}

#[derive(Debug, Deserialize)]
struct WorkerTokenResponse {
    token: String,
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
    pub tenant_id: Uuid,
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

/// Pre-generated RSA private key for JWT signing (test purposes only).
const RSA_PRIVATE_KEY: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEpQIBAAKCAQEA4eT5a8zavPvqpUDELVYEDrT2AYkl/3/PvHSXye1DoId0bszP
NzMEDzJzN6+DybKajRkvyGe8yvBsWTGVajLbFGF/qDkSqWyu3IDqv4C2tOGcQumd
q8eAYCmfAEieuh6MPtuotPnFy0qG6/14nYM9ZZofCxNk1sHMoIZem9EDJIUCsnga
zp58Gp9YHLK/L5T5/03up5WRZeFzwaIb9rQ9jt7ILkbYFKQGS81EJjj9YBEAYfJs
NjnpH79kPQDh1Juo31DeTpF/+mRH8BzRIv8+GB0NZ5t/2YVn+ndPDQPYP4O6K/Gu
3XaUG4VwTI8lPTTgnx203Znjd+kZTFNqljAfkQIDAQABAoIBADokihB1q22ON+Cu
EXCL3cJ9UH6nsuiXGLysk+8tC0WT5+OnAsT19BsHRMG2AulU99POgk6GaQEhLfot
OYSar2oJCGcfvY5vQ3jNE98TvbNECMjuQZ+X25Kk0+CqUHSebUG2ny9pxL/lIGI4
nSWJxLFUoJ3ksYVXX5iHzW00uKbau5Woh9vLD8FehbNleCH5nj/ASX+pwaOh5Kg8
z7WUIaF1FKwGLKZZuwjxHGTL2Vk6anlOuV6LTjbUlU5hiQB8ccySV1VsNb9/Kfss
adCRf50NwwSPGOzdIy303UW8Nvzf86BFoJypU/M9VuUwYwk1XiKBmseIiZ8Ni3Qa
EgxJaSsCgYEA/ZN5/cTi5kzqaIN022RpzAxcg4SR1TZEhL7hsDZWi+ti2mGX545x
AXpa5AIoRzVWJdcYrGDhn2ABN2f2EWo3lqf5NqD7Toobfyt5rkANk7m6pvz+wtom
LqXT1tE1v9YVru8YQs1JT4m25szlkPrkPYW7hFcLtZZCLcnSM54mMEMCgYEA5A3C
HyCepLUnsJiEr7mRaxK8lhgD+x2ouJ4BO1NnNruQx3CoOK/OhB0XJW7fbLSMduor
XUZjzYt2wPfArhoSU+YKkZ9+sp99w8cRKf0/SgGqzA/XcKtbARnvsFDY2hbfRgy6
sSnotSVMzwsTDtnfrOHTzz50ufLvaqlXHwfUjZsCgYEAltVNgDTIHuNrn6VqMkJF
aDmGIjkOIfw4v5lnV9DKpEnssCfTGsqwz4c/X1clLE4+ox2SMJ8kNg/+ST3Osccz
r6rU47jYI3ylJHzw0USKju+wZjohNDhc8+xx2NrzFNw8Y6UXEk1YKTaqlBkXCKkk
cLAGvY6liWsKjH/7R/bvkk8CgYEAx2JIABLy4KoJg1o1V7V0MBr3inqAsIIjyxVJ
mma27KFcWSJj0PvUIKmWXQHskQvhau4c77Xk+AYg02FIsm7U60lKoDrD+MN8nzhi
B0YEmV2PyE1pXHZUYEgeyRZGIZaxqnrilpY/gHCWEMZr6SYPawUdvCmswA5nx+c5
5kVgTlUCgYEAzUgbAafuJTV3vr2SbeJkJbiKEPGU0JwsKfcPkiGWJ7uB6gcDsiOT
OlbEUYIHsdfF8sfJ0EinuNJAjSV90RzKuCh8hebW05+GeZgnEnihq9OzDkbiWK3Y
bjfaFh9uJuX8lyyLafs0BfdftZEgygIOvUJ0YUYw//t55Ilwk5vVQ9Q=
-----END RSA PRIVATE KEY-----"#;
