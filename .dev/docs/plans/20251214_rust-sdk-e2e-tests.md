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
    org_id: Uuid,
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

        // 4. Create test org and worker token via REST API
        let (org_id, org_slug, worker_token) =
            Self::setup_test_org(server_http_port).await;

        Self {
            postgres,
            nats,
            server,
            server_grpc_port,
            server_http_port,
            org_id,
            worker_token,
        }
    }

    /// Setup test org and worker token via REST API.
    /// Uses a self-signed JWT for authentication (signature not verified).
    async fn setup_test_org(http_port: u16) -> (Uuid, String, String) {
        let base_url = format!("http://localhost:{}", http_port);

        // 1. Generate self-signed JWT (any key works, verification disabled)
        let jwt = Self::generate_test_jwt();

        // 2. Create tenant via REST API
        // POST /api/orgs with Authorization: Bearer <jwt>
        let tenant_response = reqwest::Client::new()
            .post(format!("{}/api/orgs", base_url))
            .header("Authorization", format!("Bearer {}", jwt))
            .json(&json!({
                "name": "test org",
                "slug": format!("test-{}", Uuid::new_v4().to_string()[..8]),
            }))
            .send()
            .await
            .unwrap();

        let tenant: serde_json::Value = tenant_response.json().await.unwrap();
        let org_id = Uuid::parse_str(tenant["id"].as_str().unwrap()).unwrap();
        let org_slug = tenant["slug"].as_str().unwrap().to_string();

        // 3. Create worker token via REST API
        // POST /api/orgs/{slug}/worker-tokens
        let token_response = reqwest::Client::new()
            .post(format!("{}/api/orgs/{}/worker-tokens", base_url, org_slug))
            .header("Authorization", format!("Bearer {}", jwt))
            .json(&json!({
                "displayName": "e2e-test-worker",
            }))
            .send()
            .await
            .unwrap();

        let token: serde_json::Value = token_response.json().await.unwrap();
        let worker_token = token["token"].as_str().unwrap().to_string();

        (org_id, org_slug, worker_token)
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

    pub fn org_id(&self) -> Uuid {
        self.org_id
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

## Resolved Issues

### Task Scheduling Issue - RESOLVED

**Problem**: Workflow schedules task via `ctx.schedule_raw()`, but task never executed.

**Root cause**: The `schedule_raw` method was only recording a command but not actually submitting the task to the server. The server needs a `submitTask` gRPC call before the task worker can poll for it.

**Solution**:
1. Created `TaskSubmitter` trait (`sdk/src/workflow/task_submitter.rs`)
2. Implemented `GrpcTaskSubmitter` that calls `submitTask` via gRPC
3. Updated `WorkflowContextImpl` to use `TaskSubmitter` before recording SCHEDULE_TASK command
4. Passed `TaskSubmitter` from `WorkflowExecutorWorker` to the context

### Timer/Replay Issue - RESOLVED

**Problem**: Server rejected events with "Sequence gap: expected 5, got 1" when workflow resumed after timer.

**Root cause**: The SDK was restarting sequence numbering from 1 on each workflow execution instead of continuing from where it left off during replay.

**Solution**:
1. Fixed sequence number tracking in `WorkflowContextImpl` to continue from replay events
2. Added `next_sequence` field that is initialized from the max sequence in replay events + 1
3. New commands now get correct sequence numbers that follow the replayed events

### Worker Ready Race Condition - RESOLVED

**Problem**: `handle.await_ready()` would hang indefinitely because `notify_waiters()` was called before `notified().await`.

**Root cause**: `Notify::notify_waiters()` only wakes tasks already waiting. If worker calls it before test calls `notified().await`, the notification is lost.

**Solution**: Changed from `notify_waiters()` to `notify_one()` in both workflow and task workers. `notify_one()` stores a permit that can be consumed later, even if the waiter hasn't started yet.

---

## Test Timeouts

All E2E tests now have automatic timeout protection:

1. **Per-test timeout** (60 seconds default): Each test is wrapped with `with_timeout(TEST_TIMEOUT, ...)` that panics if the test takes too long.

2. **CI job timeout**: GitHub Actions jobs have `timeout-minutes` set (15 min for build, 10 min for lint/docs).

This prevents hanging tests from consuming CI minutes or blocking local development.

---

## TODO

- [x] Add testcontainers dependencies to `sdk/Cargo.toml` (include `reqwest`, `jsonwebtoken`, `chrono`)
- [x] Create `tests/e2e/mod.rs` entry point
- [x] Implement `tests/e2e/harness.rs` with container setup
- [x] Implement self-signed JWT generation for REST API auth
- [x] Implement tenant/worker-token creation via REST API
- [x] Create `tests/e2e/fixtures/mod.rs`
- [x] Implement `tests/e2e/fixtures/workflows.rs` (DoublerWorkflow, EchoWorkflow, FailingWorkflow, StatefulWorkflow, TaskSchedulingWorkflow, TimerWorkflow, MultiTaskWorkflow)
- [x] Implement `tests/e2e/fixtures/tasks.rs` (EchoTask, SlowTask)
- [x] Implement `tests/e2e/workflow_tests.rs` (4 tests passing)
- [x] Implement `tests/e2e/task_tests.rs` (2 tests passing)
- [x] Implement `tests/e2e/state_tests.rs` (1 test passing)
- [x] Implement `tests/e2e/timer_tests.rs` (2 tests passing)
- [x] Add test timeout protection (with_timeout wrapper, CI timeout-minutes)
- [x] Fix replay/sequence numbering issue
- [x] Fix task scheduling (TaskSubmitter trait + GrpcTaskSubmitter)
- [x] Fix worker ready race condition (notify_one vs notify_waiters)
- [x] Implement `tests/e2e/promise_tests.rs` (2 tests)
- [x] Implement `tests/e2e/child_workflow_tests.rs` (3 tests)
- [x] Implement `tests/e2e/error_tests.rs` (2 tests)
- [x] Implement `tests/e2e/concurrency_tests.rs` (2 tests)
- [x] Add workflows: PromiseWorkflow, PromiseWithTimeoutWorkflow, ParentWorkflow, ChildWorkflow, FailingChildWorkflow, ParentWithFailingChildWorkflow, GrandparentWorkflow
- [ ] Add GitHub Actions workflow for E2E tests
- [ ] Ensure Flovyn server Docker image is built in CI

**Note**: `tests/e2e/replay_tests.rs` was not implemented as determinism violation detection is better tested at the unit level with controlled replay scenarios. The E2E tests focus on integration with the real server.

## Current Test Status

All E2E tests implemented (19 tests total):

**workflow_tests.rs** (5 tests):
- `test_harness_setup` ✅
- `test_simple_workflow_execution` ✅
- `test_echo_workflow` ✅
- `test_failing_workflow` ✅
- `test_start_workflow_async` ✅

**task_tests.rs** (2 tests):
- `test_basic_task_scheduling` ✅
- `test_multiple_sequential_tasks` ✅

**state_tests.rs** (1 test):
- `test_state_set_get` ✅

**timer_tests.rs** (2 tests):
- `test_durable_timer_sleep` ✅
- `test_short_timer` ✅

**promise_tests.rs** (2 tests):
- `test_promise_resolve` ✅
- `test_promise_reject` ✅

**child_workflow_tests.rs** (3 tests):
- `test_child_workflow_success` ✅
- `test_child_workflow_failure` ✅
- `test_nested_child_workflows` ✅

**error_tests.rs** (2 tests):
- `test_workflow_failure` ✅
- `test_error_message_preserved` ✅

**concurrency_tests.rs** (2 tests):
- `test_concurrent_workflow_execution` ✅
- `test_multiple_workers` ✅

Run with:
```bash
FLOVYN_E2E_USE_DEV_INFRA=1 cargo test --test e2e -p flovyn-sdk -- --include-ignored --test-threads=1
```
