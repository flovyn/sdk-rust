# Implementation Plan: Worker Lifecycle Enhancement

**Design Document**: [worker-lifecycle-enhancement.md](../design/worker-lifecycle-enhancement.md)
**Created**: 2025-12-21
**Estimated Phases**: 5

## Overview

This plan implements the Worker Lifecycle Enhancement as specified in the design document. The implementation follows a test-first approach, building from foundational types to the full feature set.

## Prerequisites

- Rust SDK compiles and existing tests pass
- Flovyn server running for E2E tests
- Understanding of current `WorkerHandle`, `WorkflowExecutorWorker`, and `TaskExecutorWorker` internals

## Phase 1: Core Types and Data Structures

**Goal**: Define all new types without changing existing behavior.

### TODO

- [x] **1.1** Create `sdk/src/worker/lifecycle/mod.rs` module structure
  - Add `mod lifecycle;` to `sdk/src/worker/mod.rs`
  - Create submodules: `types.rs`, `events.rs`, `hooks.rs`, `metrics.rs`, `reconnection.rs`

- [x] **1.2** Implement `WorkerStatus` enum in `types.rs`
  ```rust
  pub enum WorkerStatus {
      Initializing,
      Registering,
      Running { server_worker_id: Option<Uuid>, started_at: SystemTime },
      Paused { reason: String },
      Reconnecting { attempts: u32, disconnected_at: SystemTime, last_error: Option<String> },
      ShuttingDown { requested_at: SystemTime, in_flight_count: usize },
      Stopped { stopped_at: SystemTime, reason: StopReason },
  }
  ```
  - Derive `Debug, Clone, PartialEq, Eq`
  - Add unit tests for equality and clone

- [x] **1.3** Implement `StopReason` enum in `types.rs`
  ```rust
  pub enum StopReason { Graceful, Immediate, Aborted, Error(String) }
  ```

- [x] **1.4** Implement `RegistrationInfo` struct in `types.rs`
  - Include `worker_id`, `success`, `registered_at`, `workflow_kinds`, `task_kinds`
  - Include `workflow_conflicts: Vec<WorkflowConflict>`, `task_conflicts: Vec<TaskConflict>`
  - Add conversion from existing `RegistrationResult` in `worker_lifecycle.rs`

- [x] **1.5** Implement `WorkflowConflict` and `TaskConflict` structs in `types.rs`
  - Fields: `kind`, `reason`, `existing_worker_id`

- [x] **1.6** Implement `ConnectionInfo` struct in `types.rs`
  - Fields: `connected`, `last_heartbeat`, `last_poll`, `heartbeat_failures`, `poll_failures`, `reconnect_attempt`
  - Implement `Default` trait

- [x] **1.7** Implement `WorkerMetrics` struct in `metrics.rs`
  - Fields as per design: execution counts, in-progress counts, error counts, avg durations, uptime
  - Implement `Default` trait
  - Add helper methods: `record_workflow_completed()`, `record_task_completed()`, `record_error()`

- [x] **1.8** Implement `WorkType` enum in `events.rs`
  ```rust
  pub enum WorkType { Workflow, Task }
  ```

- [x] **1.9** Implement `WorkerLifecycleEvent` enum in `events.rs`
  - All variants as per design document
  - Derive `Debug, Clone`

- [x] **1.10** Implement `WorkerControlError` in `types.rs`
  - Use `thiserror` for error derivation

- [x] **1.11** Write unit tests for all new types
  - Test serialization/deserialization where applicable
  - Test Default implementations
  - Test helper methods on `WorkerMetrics`

### Acceptance Criteria
- All types compile
- Unit tests pass
- No changes to existing behavior
- Types are exported from `sdk/src/lib.rs` or `prelude.rs`

---

## Phase 2: Lifecycle Hook System

**Goal**: Implement the hook trait and chain, integrate into builder.

### TODO

- [x] **2.1** Implement `WorkerLifecycleHook` trait in `hooks.rs`
  ```rust
  #[async_trait]
  pub trait WorkerLifecycleHook: Send + Sync {
      async fn on_event(&self, event: WorkerLifecycleEvent) { let _ = event; }
      async fn on_status_change(&self, old: WorkerStatus, new: WorkerStatus) { let _ = (old, new); }
      async fn on_before_start(&self) -> bool { true }
      async fn on_before_stop(&self, graceful: bool) { let _ = graceful; }
      async fn on_pause_requested(&self, reason: &str) -> bool { let _ = reason; true }
      async fn on_resume_requested(&self) -> bool { true }
  }
  ```

- [x] **2.2** Implement `HookChain` struct in `hooks.rs`
  - Store `Vec<Arc<dyn WorkerLifecycleHook>>`
  - Methods: `add()`, `emit()`, `on_status_change()`, `before_start()`, `before_stop()`
  - Execute hooks sequentially in registration order

- [x] **2.3** Write unit tests for `HookChain`
  - Test that hooks are called in order
  - Test that `before_start()` returns false if any hook returns false
  - Test that events are delivered to all hooks

- [x] **2.4** Add `lifecycle_hooks: Vec<Arc<dyn WorkerLifecycleHook>>` field to `FlovynClientBuilder`

- [x] **2.5** Add `add_lifecycle_hook<H: WorkerLifecycleHook + 'static>(self, hook: H) -> Self` to builder
  - Wrap hook in `Arc` and store
  - Also added `reconnection_strategy()` and `max_reconnect_attempts()` methods

- [x] **2.6** Pass hooks to worker config during `build()`
  - Extend `WorkflowWorkerConfig` and `TaskWorkerConfig` with `lifecycle_hooks: HookChain`
  - Pass hooks from `FlovynClient` to worker configs in `start()`

- [x] **2.7** Write integration test: hook receives events
  - *Note: Basic hook tests in `internals.rs::test_hooks_are_called`. Full E2E test requires server.*
  - Hooks are called during emit() - verified in unit tests
  - Workers now emit Starting, Registered, Ready, HeartbeatSent/Failed, Stopped events

### Acceptance Criteria
- Hooks can be registered via builder
- Hooks receive events during worker lifecycle
- Hook chain respects registration order
- Existing tests still pass

---

## Phase 3: WorkerInternals and Event Broadcasting

**Goal**: Create shared state structure for status, metrics, and event broadcasting.

### TODO

- [x] **3.1** Create `WorkerInternals` struct in `lifecycle/internals.rs`
  ```rust
  pub struct WorkerInternals {
      status: RwLock<WorkerStatus>,
      registration_info: RwLock<Option<RegistrationInfo>>,
      connection_info: RwLock<ConnectionInfo>,
      metrics: RwLock<WorkerMetrics>,
      event_tx: broadcast::Sender<WorkerLifecycleEvent>,
      hooks: HookChain,
      created_at: Instant,
      started_at: SystemTime,
      worker_id: String,
      worker_name: Option<String>,
  }
  ```

- [x] **3.2** Implement `WorkerInternals::new()` constructor
  - Initialize with `Initializing` status
  - Create broadcast channel with capacity 256

- [x] **3.3** Implement `WorkerInternals::emit()` method
  - Update metrics based on event type
  - Call hooks
  - Broadcast to channel (ignore send errors)

- [x] **3.4** Implement `WorkerInternals::update_status()` method
  - Store old status
  - Update to new status
  - Call `hooks.on_status_change()`

- [x] **3.5** Implement helper methods on `WorkerInternals`
  - `record_heartbeat_success()` / `record_heartbeat_failure()`
  - `record_poll_success()` / `record_poll_failure()`
  - `record_work_received()` / `record_work_completed()` / `record_work_failed()`
  - `record_starting()` / `record_ready()` / `record_stopped()`
  - `record_disconnected()` / `record_reconnecting()` / `record_reconnected()`
  - `record_paused()` / `record_resumed()` / `record_shutting_down()`
  - `set_registration_info()`
  - `hooks_allow_start()` / `hooks_allow_pause()` / `hooks_allow_resume()`

- [x] **3.6** Write unit tests for `WorkerInternals`
  - Test status transitions
  - Test metrics recording
  - Test event broadcasting
  - Test heartbeat/poll tracking
  - Test work metrics
  - Test reconnection flow
  - Test shutdown flow
  - Test hooks are called

- [x] **3.7** Integrate `WorkerInternals` into `WorkflowExecutorWorker`
  - Add `internals: Arc<WorkerInternals>` field
  - Create in `new()` with lifecycle hooks from config
  - Add `internals()` getter method

- [x] **3.8** Integrate `WorkerInternals` into `TaskExecutorWorker`
  - Same pattern as workflow worker

- [x] **3.9** Update `register_with_server()` to populate `registration_info`
  - Convert `RegistrationResult` to `RegistrationInfo`
  - Store in internals via `set_registration_info()`
  - Emits `Registered` or `RegistrationFailed` event

- [x] **3.10** Update heartbeat loop to use internals
  - Call `record_heartbeat_success()` / `record_heartbeat_failure()`
  - Emits `HeartbeatSent` / `HeartbeatFailed` events

- [x] **3.11** Update worker start/stop to emit lifecycle events
  - Emit `Starting` event before registration
  - Emit `Ready` event when polling begins
  - Emit `Stopped` event when worker stops
  - *Note: Work events (WorkReceived/Completed/Failed) in polling loop deferred to Phase 4*

### Acceptance Criteria
- `WorkerInternals` tracks all state
- Events are broadcast during lifecycle
- Metrics are updated correctly
- Heartbeat and poll loops emit events

---

## Phase 4: Enhanced WorkerHandle

**Goal**: Expose status, registration, connection, metrics, and event subscription via WorkerHandle.

### TODO

- [x] **4.1** Add new fields to `WorkerHandle`
  ```rust
  pub struct WorkerHandle {
      // Existing fields...
      internals: Arc<WorkerInternals>,
  }
  ```

- [x] **4.2** Implement status methods
  - `status(&self) -> WorkerStatus`
  - `is_running(&self) -> bool`
  - `is_connected(&self) -> bool`
  - `is_shutting_down(&self) -> bool`
  - `is_paused(&self) -> bool`
  - `is_reconnecting(&self) -> bool`

- [x] **4.3** Write E2E test: `test_worker_status_transitions`
  - Start worker, verify `Running` after `await_ready()`
  - Stop worker, verify `Stopped`
  - Implemented in `tests/e2e/lifecycle_tests.rs`

- [x] **4.4** Implement registration methods
  - `registration(&self) -> Option<RegistrationInfo>`
  - `server_worker_id(&self) -> Option<Uuid>`
  - `has_conflicts(&self) -> bool`
  - `workflow_conflicts(&self) -> Vec<WorkflowConflict>`
  - `task_conflicts(&self) -> Vec<TaskConflict>`

- [x] **4.5** Write E2E test: `test_registration_info`
  - Start worker with workflow and task
  - Verify registration info contains correct kinds
  - Verify server_worker_id is Some
  - Implemented in `tests/e2e/lifecycle_tests.rs`

- [x] **4.6** Implement connection methods
  - `connection(&self) -> ConnectionInfo`
  - `time_since_heartbeat(&self) -> Option<Duration>`
  - `time_since_poll(&self) -> Option<Duration>`

- [x] **4.7** Write E2E test: `test_connection_info`
  - Start worker, wait for heartbeat
  - Verify `last_heartbeat` is set
  - Verify `time_since_heartbeat()` returns reasonable value
  - Implemented in `tests/e2e/lifecycle_tests.rs`

- [x] **4.8** Implement metrics methods
  - `metrics(&self) -> WorkerMetrics`
  - `uptime(&self) -> Duration`
  - `workflows_in_progress(&self) -> usize`
  - `tasks_in_progress(&self) -> usize`
  - `workflows_executed(&self) -> u64`
  - `tasks_executed(&self) -> u64`

- [x] **4.9** Write E2E test: `test_worker_metrics`
  - Start worker, execute 5 workflows
  - Verify `workflows_executed == 5`
  - Verify `uptime > 0`
  - Implemented in `tests/e2e/lifecycle_tests.rs`

- [x] **4.10** Implement event subscription
  - `subscribe(&self) -> broadcast::Receiver<WorkerLifecycleEvent>`

- [x] **4.11** Write E2E test: `test_lifecycle_events`
  - Subscribe to events
  - Start worker, execute workflow, stop
  - Verify received: `Ready`, `WorkReceived`, `WorkCompleted`, `Stopped`
  - Implemented in `tests/e2e/lifecycle_tests.rs`
  - Also added `test_filtered_event_subscription` for filtered subscription

- [x] **4.12** Update `FlovynClient::start()` to create `WorkerHandle` with internals
  - Create shared `WorkerInternals` in start()
  - Pass to both workers via `with_internals()`
  - Include in returned WorkerHandle

- [x] **4.13** ~~Implement `Clone` for `WorkerHandle`~~
  - *Skipped: JoinHandle does not implement Clone*
  - Users can wrap in Arc if cloning is needed
  - `internals()` method provides access to shared state

### Acceptance Criteria
- All new methods on `WorkerHandle` work correctly
- E2E tests pass
- Existing functionality unchanged

---

## Phase 5: Reconnection Strategy

**Goal**: Implement configurable reconnection with exponential backoff.

### TODO

- [x] **5.1** Implement `ReconnectionStrategy` enum in `reconnection.rs`
  ```rust
  pub enum ReconnectionStrategy {
      None,
      Fixed { delay: Duration, max_attempts: Option<u32> },
      ExponentialBackoff { initial_delay: Duration, max_delay: Duration, multiplier: f64, max_attempts: Option<u32> },
      Custom(Arc<dyn ReconnectionPolicy>),
  }
  ```

- [x] **5.2** Implement `Default` for `ReconnectionStrategy`
  - Default to `ExponentialBackoff` with sensible defaults (1s initial, 60s max, 2.0 multiplier, None max_attempts)

- [x] **5.3** Implement `ReconnectionPolicy` trait
  ```rust
  #[async_trait]
  pub trait ReconnectionPolicy: Send + Sync {
      async fn next_delay(&self, attempt: u32, last_error: &str) -> Option<Duration>;
      async fn on_reconnected(&self);
      async fn reset(&self);
  }
  ```

- [x] **5.4** Add builder methods for reconnection
  - `reconnection_strategy(self, strategy: ReconnectionStrategy) -> Self`
  - `max_reconnect_attempts(self, max: u32) -> Self` (convenience)
  - *Note: `reconnect_initial_delay` and `reconnect_max_delay` deferred - users can configure full strategy*

- [x] **5.5** Store reconnection strategy in worker config
  - Added `reconnection_strategy: ReconnectionStrategy` to `WorkflowWorkerConfig` and `TaskWorkerConfig`
  - Passed from `FlovynClient::start()` to worker configs

- [x] **5.6** Implement `calculate_next_delay()` helper function
  - Implemented as `ReconnectionStrategy::calculate_delay()` method
  - Handle all strategy variants
  - Return `None` when max attempts reached

- [x] **5.7** Write unit tests for delay calculation
  - Fixed delay returns same value
  - Exponential grows correctly
  - Max delay is respected
  - Max attempts stops reconnection

- [x] **5.8-5.10** Implement reconnection in polling loop
  - *Simplified approach: Reconnection logic integrated directly into polling loop instead of separate heartbeat trigger*
  - On connection error: track attempts, calculate delay using strategy, sleep, retry
  - Update status to `Reconnecting` with attempt count and last error
  - When max attempts reached, stop worker
  - On successful poll after reconnection: emit `Reconnected` event, reset attempt counter
  - Emits `Disconnected`, `Reconnecting`, and `Reconnected` events as appropriate

- [ ] **5.11** Write E2E test: `test_max_reconnect_attempts` (with unreachable server)
  - Configure max 3 attempts
  - Verify worker stops with error after attempts exhausted

- [ ] **5.12** Write E2E test: `test_reconnection_on_failure` (if server restart is possible)
  - Start worker
  - Stop server (or simulate disconnect)
  - Verify `Reconnecting` status
  - Restart server
  - Verify `Running` status restored

### Acceptance Criteria
- Reconnection strategies configurable via builder
- Worker automatically reconnects on connection loss
- Max attempts respected
- Events emitted during reconnection

---

## Phase 6: Pause/Resume and Control Methods

**Goal**: Implement worker pause, resume, and manual control operations.

### TODO

- [x] **6.1** Add pause control channel to workers
  - Added `pause_tx: watch::Sender<bool>`, `pause_rx: watch::Receiver<bool>` to `WorkerInternals`
  - Added `is_paused()` and `pause_receiver()` methods

- [x] **6.2** Implement `WorkerHandle::pause(&self, reason: &str) -> Result<(), WorkerControlError>`
  - Validate current status is `Running`
  - Call hooks `on_pause_requested()`
  - If any hook returns false, return `RejectedByHook` error
  - Update status to `Paused`
  - Send pause signal via watch channel
  - Emit `Paused` event

- [x] **6.3** Implement `WorkerHandle::resume(&self) -> Result<(), WorkerControlError>`
  - Validate current status is `Paused`
  - Call hooks `on_resume_requested()`
  - Update status to `Running`
  - Send resume signal via watch channel
  - Emit `Resumed` event

- [x] **6.4** Update polling loop to respect pause
  - Get pause receiver at start of polling loop
  - Check if paused at each iteration
  - When paused, wait on pause_rx.changed() instead of polling
  - Also listen for shutdown signals while paused
  - On resume (pause_rx changes to false), continue polling

- [x] **6.5** Write E2E test: `test_pause_resume` (requires server)
  - Start worker
  - Pause worker
  - Verify status is `Paused`
  - Resume worker
  - Verify status is `Running`
  - Implemented in `tests/e2e/lifecycle_tests.rs`
  - Also added `test_pause_invalid_state` and `test_resume_invalid_state`

- [ ] **6.6** ~~Implement `WorkerHandle::re_register()`~~ *Deferred*
  - Requires access to gRPC channel from WorkerHandle
  - Would need architectural changes to pass channel to internals
  - Can be implemented later if needed

- [ ] **6.7** ~~Implement `WorkerHandle::heartbeat_now()`~~ *Deferred*
  - Requires access to gRPC channel from WorkerHandle
  - Would need to add signal mechanism for immediate heartbeat
  - Can be implemented later if needed

- [ ] **6.8** ~~Write E2E test: `test_manual_heartbeat`~~ *Deferred*
  - Depends on 6.7

- [x] **6.9** Implement `subscribe_filtered<F>()` method
  - Added to `WorkerHandle`
  - Uses `BroadcastStream` and `filter` from tokio-stream
  - Added `tokio-stream` sync feature and `futures` dependency
  - Returns `impl futures::Stream<Item = WorkerLifecycleEvent>`

- [x] **6.10** Write unit tests for pause/resume
  - `test_pause_resume` - full pause/resume cycle
  - `test_pause_invalid_state` - cannot pause when not running
  - `test_resume_invalid_state` - cannot resume when not paused
  - `test_pause_receiver` - verify initial state is not paused

### Acceptance Criteria
- Pause/resume works correctly
- Manual control methods work
- E2E tests pass
- No deadlocks or race conditions

---

## Phase 7: FlovynClient Updates and Builder Enhancements

**Goal**: Add remaining client methods and builder configuration.

### TODO

- [x] **7.1** Add `has_running_worker(&self) -> bool` to `FlovynClient`
  - Checks if stored worker internals exist and status is Running

- [x] **7.2** Add `worker_internals(&self) -> Option<Arc<WorkerInternals>>` to `FlovynClient`
  - *Note: Changed from `worker_handle()` since WorkerHandle is consumed on stop*
  - Returns the stored worker internals for monitoring/access

- [x] **7.3** Add configuration getters to `FlovynClient`
  - `heartbeat_interval(&self) -> Duration`
  - `poll_timeout(&self) -> Duration`
  - `max_concurrent_workflows(&self) -> usize`
  - `max_concurrent_tasks(&self) -> usize`

- [ ] **7.4** ~~Add `auto_reregister_on_reconnect(self, enabled: bool) -> Self` to builder~~ *Deferred*
  - Reconnection already handles re-registration implicitly via polling
  - Can be added later if explicit control is needed

- [x] **7.5** Store worker internals in `FlovynClient` after start
  - Added `worker_internals: RwLock<Option<Arc<WorkerInternals>>>` field
  - Populated in `start()` before returning WorkerHandle

- [x] **7.6** Update documentation for all new APIs
  - Added rustdoc comments to all new methods
  - Added example in doc comment for `worker_internals()`

- [x] **7.7** Export all new types in `prelude.rs`
  - All lifecycle types already exported in both lib.rs and prelude

### Acceptance Criteria
- All planned APIs implemented
- Documentation complete
- Types exported in prelude

---

## Phase 8: Examples and Documentation

**Goal**: Create example applications demonstrating new features.

### TODO

- [ ] **8.1** Create `examples/lifecycle-monitoring/` example
  - Demonstrate logging hook
  - Show metrics collection
  - Print status periodically

- [ ] **8.2** Create `examples/health-check-worker/` example
  - Add axum HTTP server
  - Expose `/health` endpoint
  - Return worker status and metrics as JSON

- [ ] **8.3** Create `examples/maintenance-worker/` example
  - CLI for pause/resume/status
  - Demonstrate maintenance workflow

- [ ] **8.4** Update `examples/hello-world/` to show basic lifecycle
  - Add simple hook for demonstration

- [ ] **8.5** Add integration test for each example
  - Verify examples compile and run

- [ ] **8.6** Update CLAUDE.md with new commands if needed

### Acceptance Criteria
- All examples compile and run
- Examples demonstrate key features
- Documentation updated

---

## Testing Strategy

### Unit Tests (Phase 1-2)
- All type constructors and methods
- Hook chain execution order
- Metrics calculations

### Integration Tests (Phase 3-4)
- WorkerInternals state management
- Event broadcasting
- WorkerHandle methods

### E2E Tests (Phase 4-6)
Each test should use a unique task queue to prevent interference:

| Test | Queue | Description |
|------|-------|-------------|
| `test_worker_status_transitions` | `lifecycle-status-{uuid}` | Status changes during lifecycle |
| `test_registration_info` | `lifecycle-reg-{uuid}` | Registration info exposed |
| `test_registration_conflicts` | `lifecycle-conflict-{uuid}` | Conflict detection |
| `test_lifecycle_events` | `lifecycle-events-{uuid}` | Event subscription works |
| `test_pause_resume` | `lifecycle-pause-{uuid}` | Pause/resume functionality |
| `test_worker_metrics` | `lifecycle-metrics-{uuid}` | Metrics collection |
| `test_connection_info` | `lifecycle-conn-{uuid}` | Connection tracking |
| `test_lifecycle_hook` | `lifecycle-hook-{uuid}` | Hook integration |
| `test_graceful_shutdown_waits` | `lifecycle-shutdown-{uuid}` | Graceful shutdown |
| `test_uptime` | `lifecycle-uptime-{uuid}` | Uptime tracking |
| `test_max_reconnect_attempts` | N/A (no server) | Reconnection limits |

### Test Helpers

Create test utilities in `sdk/src/testing/lifecycle.rs`:
```rust
pub struct TestLifecycleHook {
    pub events: Arc<Mutex<Vec<WorkerLifecycleEvent>>>,
}

impl TestLifecycleHook {
    pub fn new() -> Self { ... }
    pub async fn events(&self) -> Vec<WorkerLifecycleEvent> { ... }
    pub async fn wait_for_event<F>(&self, predicate: F, timeout: Duration) -> Option<WorkerLifecycleEvent> { ... }
}
```

---

## File Changes Summary

### New Files
- `sdk/src/worker/lifecycle/mod.rs`
- `sdk/src/worker/lifecycle/types.rs`
- `sdk/src/worker/lifecycle/events.rs`
- `sdk/src/worker/lifecycle/hooks.rs`
- `sdk/src/worker/lifecycle/metrics.rs`
- `sdk/src/worker/lifecycle/reconnection.rs`
- `sdk/src/worker/lifecycle/internals.rs`
- `sdk/src/testing/lifecycle.rs`
- `tests/e2e/test_worker_lifecycle.rs`
- `tests/e2e/test_worker_reconnection.rs`
- `examples/lifecycle-monitoring/`
- `examples/health-check-worker/`
- `examples/maintenance-worker/`

### Modified Files
- `sdk/src/worker/mod.rs` - Add lifecycle module
- `sdk/src/worker/workflow_worker.rs` - Integrate WorkerInternals
- `sdk/src/worker/task_worker.rs` - Integrate WorkerInternals
- `sdk/src/client/flovyn_client.rs` - New methods, store handle
- `sdk/src/client/builder.rs` - Hook registration, reconnection config
- `sdk/src/lib.rs` or `sdk/src/prelude.rs` - Export new types
- `CLAUDE.md` - Document new features

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Breaking existing API | All changes additive; existing methods unchanged |
| Race conditions in shared state | Use `RwLock` for reads, `Arc` for sharing, careful lock ordering |
| Deadlocks | Never hold locks across await points; use channels for signaling |
| Event flooding | Bounded broadcast channel; document that slow receivers may miss events |
| Test flakiness | Unique queue per test; adequate timeouts; proper cleanup |

---

## Definition of Done

Each phase is complete when:
1. All TODOs in the phase are checked off
2. Unit tests pass locally
3. E2E tests pass with server running
4. `cargo clippy` passes with no warnings
5. `cargo fmt --check` passes
6. Code is reviewed

Full implementation is complete when:
1. All phases complete
2. All E2E tests in test plan pass
3. Examples compile and run
4. Documentation updated
5. Migration guide verified with existing code
