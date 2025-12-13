# Rust SDK E2E Test Implementation Plan

This plan outlines the implementation of end-to-end tests for the Rust SDK using Testcontainers to run a real Flovyn server, PostgreSQL, and NATS.

## Overview

E2E tests validate the complete SDK functionality against a real server, ensuring:
- gRPC communication works correctly
- Workflow execution, replay, and determinism
- Task scheduling and execution
- Timers, promises, and child workflows
- Error handling and recovery scenarios

## Prerequisites

- Docker installed and running
- Flovyn server Docker image built (from `../flovyn/server/app/Dockerfile`)
- Access to PostgreSQL and NATS Docker images

## Test Infrastructure

### Dependencies

Update `sdk/Cargo.toml`:

```toml
[dev-dependencies]
testcontainers = "0.23"
testcontainers-modules = { version = "0.11", features = ["postgres"] }
tokio = { version = "1", features = ["full", "test-util"] }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
reqwest = { version = "0.12", features = ["json"] }
jsonwebtoken = "9"
```

### Test Harness

**File: `tests/e2e/harness.rs`**

```rust
use testcontainers::{core::WaitFor, runners::AsyncRunner, ContainerAsync, GenericImage, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

pub struct TestHarness {
    postgres: ContainerAsync<Postgres>,
    nats: ContainerAsync<GenericImage>,
    server: ContainerAsync<GenericImage>,
    server_grpc_port: u16,
    server_http_port: u16,
    tenant_id: Uuid,
    worker_token: String,
}

impl TestHarness {
    pub async fn new() -> Self {
        // 1. Start PostgreSQL
        let postgres = Postgres::default()
            .with_db_name("flovyn")
            .with_user("flovyn")
            .with_password("flovyn")
            .start()
            .await
            .expect("Failed to start PostgreSQL");

        let pg_port = postgres.get_host_port_ipv4(5432).await.unwrap();

        // 2. Start NATS
        let nats = GenericImage::new("nats", "2.12-alpine")
            .with_wait_for(WaitFor::message_on_stderr("Server is ready"))
            .with_exposed_port(4222.into())
            .start()
            .await
            .expect("Failed to start NATS");

        let nats_port = nats.get_host_port_ipv4(4222).await.unwrap();

        // 3. Start Flovyn server
        let server = GenericImage::new("flovyn/server", "latest")
            .with_env_var("SPRING_DATASOURCE_URL", format!("jdbc:postgresql://host.docker.internal:{}/flovyn", pg_port))
            .with_env_var("SPRING_DATASOURCE_USERNAME", "flovyn")
            .with_env_var("SPRING_DATASOURCE_PASSWORD", "flovyn")
            .with_env_var("NATS_ENABLED", "true")
            .with_env_var("NATS_SERVERS_0", format!("nats://host.docker.internal:{}", nats_port))
            .with_env_var("SERVER_PORT", "8080")
            .with_env_var("GRPC_SERVER_PORT", "9090")
            .with_env_var("SPRING_FLYWAY_ENABLED", "true")
            .with_env_var("SPRING_FLYWAY_BASELINE_ON_MIGRATE", "true")
            // Disable Spring OAuth2 JWT verification (skip OIDC provider)
            // Worker token authentication still works
            .with_env_var("SPRING_AUTOCONFIGURE_EXCLUDE", "org.springframework.boot.autoconfigure.security.oauth2.resource.servlet.OAuth2ResourceServerAutoConfiguration")
            .with_exposed_port(8080.into())
            .with_exposed_port(9090.into())
            .with_wait_for(WaitFor::http(
                WaitFor::Http::new("/actuator/health")
                    .with_port(8080.into())
                    .with_expected_status_code(200_u16),
            ))
            .start()
            .await
            .expect("Failed to start Flovyn server");

        let server_grpc_port = server.get_host_port_ipv4(9090).await.unwrap();
        let server_http_port = server.get_host_port_ipv4(8080).await.unwrap();

        // 4. Create test tenant and worker token via REST API
        let (tenant_id, tenant_slug, worker_token) =
            Self::setup_test_tenant(server_http_port).await;

        Self {
            postgres,
            nats,
            server,
            server_grpc_port,
            server_http_port,
            tenant_id,
            worker_token,
        }
    }

    /// Setup test tenant and worker token via REST API.
    /// Uses a self-signed JWT for authentication (signature not verified).
    async fn setup_test_tenant(http_port: u16) -> (Uuid, String, String) {
        let base_url = format!("http://localhost:{}", http_port);

        // 1. Generate self-signed JWT (any key works, verification disabled)
        let jwt = Self::generate_test_jwt();

        // 2. Create tenant via REST API
        // POST /api/tenants with Authorization: Bearer <jwt>
        let tenant_response = reqwest::Client::new()
            .post(format!("{}/api/tenants", base_url))
            .header("Authorization", format!("Bearer {}", jwt))
            .json(&json!({
                "name": "Test Tenant",
                "slug": format!("test-{}", Uuid::new_v4().to_string()[..8]),
            }))
            .send()
            .await
            .unwrap();

        let tenant: serde_json::Value = tenant_response.json().await.unwrap();
        let tenant_id = Uuid::parse_str(tenant["id"].as_str().unwrap()).unwrap();
        let tenant_slug = tenant["slug"].as_str().unwrap().to_string();

        // 3. Create worker token via REST API
        // POST /api/tenants/{slug}/worker-tokens
        let token_response = reqwest::Client::new()
            .post(format!("{}/api/tenants/{}/worker-tokens", base_url, tenant_slug))
            .header("Authorization", format!("Bearer {}", jwt))
            .json(&json!({
                "displayName": "e2e-test-worker",
            }))
            .send()
            .await
            .unwrap();

        let token: serde_json::Value = token_response.json().await.unwrap();
        let worker_token = token["token"].as_str().unwrap().to_string();

        (tenant_id, tenant_slug, worker_token)
    }

    /// Generate a self-signed JWT for REST API authentication.
    /// Since OAuth2ResourceServerAutoConfiguration is excluded, signature is not verified.
    fn generate_test_jwt() -> String {
        // JWT with required claims: sub, iss, aud, exp
        // Sign with any RSA key (not validated by server)
        let claims = json!({
            "sub": Uuid::new_v4().to_string(),
            "iss": "http://localhost:3000",
            "aud": "flovyn-server",
            "exp": chrono::Utc::now().timestamp() + 3600,
            "iat": chrono::Utc::now().timestamp(),
        });

        // Use jsonwebtoken crate with any RSA key
        todo!("Generate JWT with jsonwebtoken crate")
    }

    pub fn grpc_host(&self) -> &str {
        "localhost"
    }

    pub fn grpc_port(&self) -> u16 {
        self.server_grpc_port
    }

    pub fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    pub fn worker_token(&self) -> &str {
        &self.worker_token
    }
}
```

### Server Environment Variables

Based on the actual server configuration:

| Variable | Required | Description |
|----------|----------|-------------|
| `SPRING_DATASOURCE_URL` | Yes | JDBC PostgreSQL connection URL |
| `SPRING_DATASOURCE_USERNAME` | Yes | Database username |
| `SPRING_DATASOURCE_PASSWORD` | Yes | Database password |
| `NATS_ENABLED` | Yes | Enable NATS messaging (`true`) |
| `NATS_SERVERS_0` | Yes | NATS server URL |
| `SERVER_PORT` | No | HTTP port (default: 8080) |
| `GRPC_SERVER_PORT` | No | gRPC port (default: 9090) |
| `SPRING_FLYWAY_ENABLED` | No | Auto-run migrations (default: true) |
| `SPRING_FLYWAY_BASELINE_ON_MIGRATE` | No | Baseline on migrate (default: false) |
| `SPRING_AUTOCONFIGURE_EXCLUDE` | No | Exclude auto-configurations (set to `...OAuth2ResourceServerAutoConfiguration` to skip JWT issuer verification) |

### Health Check

Server health endpoint: `GET http://localhost:8080/actuator/health`

Expected response: HTTP 200 with `{"status": "UP"}`

## Test Scenarios

### Phase 1: Basic Workflow Tests

**File: `tests/e2e/workflow_tests.rs`**

| Test | Description |
|------|-------------|
| `test_simple_workflow_execution` | Start workflow, wait for completion, verify result |
| `test_workflow_with_run_operation` | Execute `ctx.run()` and verify cached result |
| `test_workflow_input_output_serialization` | Verify JSON serialization roundtrip |
| `test_multiple_operations_workflow` | Multiple `ctx.run()` calls execute in order |

### Phase 2: Task Execution Tests

**File: `tests/e2e/task_tests.rs`**

| Test | Description |
|------|-------------|
| `test_basic_task_scheduling` | Workflow schedules task, receives result |
| `test_task_with_progress_reporting` | Task reports progress via `ctx.report_progress()` |
| `test_task_failure_and_retry` | Task fails, retries, eventually succeeds |
| `test_task_max_retries_exceeded` | Max retries exceeded returns error to workflow |
| `test_task_cancellation` | Workflow cancelled, pending tasks cancelled |

### Phase 3: State Management Tests

**File: `tests/e2e/state_tests.rs`**

| Test | Description |
|------|-------------|
| `test_state_set_get` | `ctx.set()` and `ctx.get()` roundtrip |
| `test_state_clear` | `ctx.clear()` removes state |
| `test_state_keys` | `ctx.state_keys()` lists all keys |
| `test_state_persistence_across_tasks` | State persists across workflow tasks |

### Phase 4: Timer and Promise Tests

**File: `tests/e2e/timer_tests.rs`**

| Test | Description |
|------|-------------|
| `test_durable_timer_sleep` | `ctx.sleep()` with 1-2 second duration |
| `test_timer_replay` | Timer already fired on replay |

**File: `tests/e2e/promise_tests.rs`**

| Test | Description |
|------|-------------|
| `test_promise_resolve` | `ctx.promise()` suspends, external resolve resumes |
| `test_promise_reject` | External reject returns error to workflow |

### Phase 5: Child Workflow Tests

**File: `tests/e2e/child_workflow_tests.rs`**

| Test | Description |
|------|-------------|
| `test_child_workflow_success` | Parent schedules child, receives result |
| `test_child_workflow_failure` | Child fails, parent receives error |
| `test_nested_child_workflows` | Parent → child → grandchild |

### Phase 6: Replay and Determinism Tests

**File: `tests/e2e/replay_tests.rs`**

| Test | Description |
|------|-------------|
| `test_worker_restart_replay` | Worker restart mid-workflow, replay continues |
| `test_determinism_violation_added_operation` | Added operation causes DeterminismViolation |
| `test_determinism_violation_removed_operation` | Removed operation causes DeterminismViolation |

### Phase 7: Error Handling Tests

**File: `tests/e2e/error_tests.rs`**

| Test | Description |
|------|-------------|
| `test_workflow_failure` | Workflow throws error, marked as failed |
| `test_error_message_preserved` | Error message preserved in failure |

### Phase 8: Concurrency Tests

**File: `tests/e2e/concurrency_tests.rs`**

| Test | Description |
|------|-------------|
| `test_concurrent_workflow_execution` | Multiple workflows executing simultaneously |
| `test_multiple_workers` | Two workers polling same queue |

## Test Fixtures

### Reusable Test Workflows

```rust
// tests/e2e/fixtures/workflows.rs

use flovyn_sdk::prelude::*;

#[derive(Clone)]
pub struct SimpleWorkflow;

impl WorkflowDefinition for SimpleWorkflow {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn kind(&self) -> &'static str {
        "simple-workflow"
    }

    async fn execute(
        &self,
        ctx: &mut impl WorkflowContext,
        input: Self::Input,
    ) -> Result<Self::Output> {
        let value = input["value"].as_i64().unwrap_or(0);
        Ok(json!({"result": value * 2}))
    }
}

#[derive(Clone)]
pub struct StatefulWorkflow;
// ctx.set(), ctx.get(), ctx.clear() operations

#[derive(Clone)]
pub struct TaskSchedulingWorkflow;
// ctx.schedule() task and await result

#[derive(Clone)]
pub struct TimerWorkflow;
// ctx.sleep() operation

#[derive(Clone)]
pub struct ChildWorkflow;
// Simple child for parent workflow tests

#[derive(Clone)]
pub struct ParentWorkflow;
// Schedules ChildWorkflow

#[derive(Clone)]
pub struct FailingWorkflow;
// Returns error for failure tests
```

### Reusable Test Tasks

```rust
// tests/e2e/fixtures/tasks.rs

use flovyn_sdk::prelude::*;

#[derive(Clone)]
pub struct EchoTask;

impl TaskDefinition for EchoTask {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn kind(&self) -> &'static str {
        "echo-task"
    }

    async fn execute(
        &self,
        _ctx: &impl TaskContext,
        input: Self::Input,
    ) -> Result<Self::Output> {
        Ok(input)
    }
}

#[derive(Clone)]
pub struct SlowTask;
// Sleeps for configurable duration

#[derive(Clone)]
pub struct FailingTask;
// Fails N times then succeeds (for retry tests)

#[derive(Clone)]
pub struct ProgressTask;
// Reports progress via ctx.report_progress()
```

## File Structure

```
tests/
└── e2e/
    ├── mod.rs              # Test module entry point
    ├── harness.rs          # TestHarness container setup
    ├── fixtures/
    │   ├── mod.rs
    │   ├── workflows.rs    # Test workflow definitions
    │   └── tasks.rs        # Test task definitions
    ├── workflow_tests.rs   # Phase 1 tests
    ├── task_tests.rs       # Phase 2 tests
    ├── state_tests.rs      # Phase 3 tests
    ├── timer_tests.rs      # Phase 4 tests
    ├── promise_tests.rs    # Phase 4 tests
    ├── child_workflow_tests.rs  # Phase 5 tests
    ├── replay_tests.rs     # Phase 6 tests
    ├── error_tests.rs      # Phase 7 tests
    └── concurrency_tests.rs # Phase 8 tests
```

## Success Criteria

- All E2E tests pass consistently (no flaky tests)
- Tests complete in < 5 minutes total
- Tests clean up resources properly (no leaked containers)
- Tests work in CI environment (GitHub Actions with Docker)
- Coverage of all major SDK features

## Notes

- Use `#[tokio::test]` for async tests
- Set reasonable timeouts (30s per test max)
- Use `tracing-subscriber` for debugging test failures
- Run tests serially by default (shared Docker resources)
- Mark slow tests with `#[ignore]` for quick iteration
- Docker networking: use `host.docker.internal` for container-to-host communication

---

## TODO

- [x] Add testcontainers dependencies to `sdk/Cargo.toml` (include `reqwest`, `jsonwebtoken`, `chrono`)
- [x] Create `tests/e2e/mod.rs` entry point
- [x] Implement `tests/e2e/harness.rs` with container setup
- [x] Implement self-signed JWT generation for REST API auth
- [x] Implement tenant/worker-token creation via REST API
- [x] Create `tests/e2e/fixtures/mod.rs`
- [x] Implement `tests/e2e/fixtures/workflows.rs` (DoublerWorkflow, EchoWorkflow, FailingWorkflow, RunOperationWorkflow, StatefulWorkflow, TaskSchedulingWorkflow)
- [x] Implement `tests/e2e/fixtures/tasks.rs` (EchoTask, SlowTask, FailingTask, ProgressTask)
- [x] Implement `tests/e2e/workflow_tests.rs` (harness test, more tests pending SDK client integration)
- [ ] Implement `tests/e2e/task_tests.rs` (5 tests)
- [ ] Implement `tests/e2e/state_tests.rs` (4 tests)
- [ ] Implement `tests/e2e/timer_tests.rs` (2 tests)
- [ ] Implement `tests/e2e/promise_tests.rs` (2 tests)
- [ ] Implement `tests/e2e/child_workflow_tests.rs` (3 tests)
- [ ] Implement `tests/e2e/replay_tests.rs` (3 tests)
- [ ] Implement `tests/e2e/error_tests.rs` (2 tests)
- [ ] Implement `tests/e2e/concurrency_tests.rs` (2 tests)
- [ ] Add GitHub Actions workflow for E2E tests
- [ ] Ensure Flovyn server Docker image is built in CI
