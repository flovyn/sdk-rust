# E2E Tests Implementation Plan

## Goal
Implement E2E tests for the Rust SDK that test against a real Flovyn server with security enabled.

## Status: COMPLETED ✓

All E2E tests are now passing:
- `test_harness_setup` - Verifies container and tenant setup
- `test_simple_workflow_execution` - Tests DoublerWorkflow execution
- `test_echo_workflow` - Tests EchoWorkflow with input echo
- `test_failing_workflow` - Tests workflow failure event handling
- `test_start_workflow_async` - Tests async workflow start and event polling

## Implementation Summary

### 1. Worker Token Authentication
- Created `WorkerTokenInterceptor` in `sdk/src/client/auth.rs`
- Adds `Authorization: Bearer <token>` header to all gRPC requests
- Token must have `fwt_` prefix (enforced at construction)

### 2. gRPC Client Updates
All gRPC clients now use authenticated channels:
- `WorkerLifecycleClient` - worker registration and heartbeat
- `WorkflowDispatch` - workflow start, poll, and completion
- `TaskExecutionClient` - task polling and completion

### 3. E2E Test Harness
- Uses Testcontainers to start Flovyn server
- Connects to existing dev PostgreSQL (port 5435) and NATS (port 4222)
- Creates test tenant and worker token via REST API with self-signed JWT
- Server runs with `FLOVYN_SECURITY_JWT_SKIP_SIGNATURE_VERIFICATION=true`

### 4. Workflow Version Registration
- SDK sends `version` and `contentHash` with workflow capabilities
- Content hash computed as SHA-256 of `{kind}:{version}`
- Server creates `workflow_definition_version` record with default flag

## Key Learnings

1. **Tests must run sequentially** (`--test-threads=1`) to avoid conflicts when multiple workers register the same workflow kind
2. **Failing workflows are retried** by the server - tests should check for failure events rather than waiting for permanent failure
3. **Explicit workflow version** should be passed in `StartWorkflowOptions` for reliable version resolution

## Running Tests

```bash
# With dev infrastructure (PostgreSQL on 5435, NATS on 4222)
FLOVYN_E2E_USE_DEV_INFRA=1 cargo test --test e2e -p flovyn-sdk -- --include-ignored --test-threads=1
```

## Test Flow

1. Test harness starts Flovyn server container (with security enabled)
2. Harness creates tenant via REST API with JWT auth
3. Harness creates worker token via REST API
4. Rust SDK builds FlovynClient with worker_token
5. SDK registers worker (workflows + tasks) via gRPC with auth
6. Server creates workflow_definition AND workflow_definition_version
7. SDK starts workflow via gRPC startWorkflow
8. Server resolves version (finds the registered version)
9. Workflow executes and completes
