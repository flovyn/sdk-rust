# Design: Server-Initiated Worker Drain

**Status**: Draft
**Created**: 2025-12-21
**Author**: Claude Code
**Related**: [worker-lifecycle-enhancement.md](./worker-lifecycle-enhancement.md)

## Overview

This document describes a mechanism for the server to initiate graceful drain of workers, primarily for server maintenance scenarios. The design uses a **stateless mode** approach where the heartbeat response contains the current server mode, allowing workers to automatically adjust their behavior.

### Goals

1. **Graceful maintenance**: Allow server to drain workers before going offline
2. **Automatic recovery**: Workers resume automatically when server returns
3. **Stateless protocol**: No command history to track; current state only
4. **Backward compatible**: Old workers ignore new fields; old servers return empty response

### Non-Goals

1. Individual worker targeting (drain specific worker) - future enhancement
2. Queue-level drain (drain specific queue) - future enhancement
3. Workflow cancellation during drain - separate mechanism

---

## Problem Statement

### Current Behavior

When a server needs maintenance:

1. Admin stops the server
2. Workers' heartbeats fail immediately
3. Workers enter disconnected state
4. In-flight workflows may be interrupted
5. Workers reconnect when server returns

**Issues**:
- No graceful drain period
- In-flight work may be lost or retried
- No warning to workers before shutdown

### Desired Behavior

1. Admin initiates drain mode on server
2. Server tells workers to stop accepting new work
3. Workers complete in-flight work
4. Admin shuts down server (workers now idle)
5. Server comes back online
6. Workers automatically resume

---

## Design

### Core Concept: Server Mode

Instead of sending imperative commands ("pause now", "resume now"), the server communicates its current **mode** in every heartbeat response. Workers adjust their behavior based on the current mode.

```
┌─────────────────────────────────────────────────────────────────┐
│                        SERVER MODES                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  NORMAL          Worker polls for work, executes normally       │
│                                                                 │
│  DRAINING        Worker stops polling, completes in-flight      │
│                  work, continues heartbeats                     │
│                                                                 │
│  (future)                                                       │
│  READ_ONLY       Worker can query but not start new work        │
│  SHUTTING_DOWN   Server will be offline imminently              │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### Why Stateless?

| Approach | Command-Based | Mode-Based (Stateless) |
|----------|---------------|------------------------|
| Message | "Pause now" | "Current mode is DRAINING" |
| Server restart | Must replay commands | Fresh start = NORMAL |
| Missed message | Worker in wrong state | Next heartbeat corrects |
| Consistency | Eventual | Immediate (per heartbeat) |
| Complexity | Track command history | No history needed |

---

## Protocol Changes

### Current Protocol

```protobuf
service WorkerLifecycle {
  rpc SendHeartbeat(WorkerHeartbeatRequest) returns (google.protobuf.Empty);
}
```

### Proposed Protocol

```protobuf
service WorkerLifecycle {
  // Existing
  rpc RegisterWorker(WorkerRegistrationRequest) returns (WorkerRegistrationResponse);

  // Modified: now returns WorkerHeartbeatResponse instead of Empty
  rpc SendHeartbeat(WorkerHeartbeatRequest) returns (WorkerHeartbeatResponse);
}

// New message
message WorkerHeartbeatResponse {
  // Current server mode
  ServerMode mode = 1;

  // Optional human-readable message (e.g., "Scheduled maintenance at 2am UTC")
  optional string message = 2;

  // Hint: expected time until mode changes (milliseconds)
  // Workers can use this for UI display or logging
  optional int64 estimated_duration_ms = 3;

  // Server timestamp for clock sync (milliseconds since epoch)
  int64 server_time_ms = 4;
}

enum ServerMode {
  // Default/unknown - treat as NORMAL for backward compatibility
  SERVER_MODE_UNSPECIFIED = 0;

  // Normal operation - accept and execute work
  SERVER_MODE_NORMAL = 1;

  // Draining - stop accepting new work, complete in-flight
  SERVER_MODE_DRAINING = 2;

  // Read-only - can query workflows but not start new ones (future)
  SERVER_MODE_READ_ONLY = 3;
}
```

### Backward Compatibility

| Client Version | Server Version | Behavior |
|----------------|----------------|----------|
| Old | Old | Empty response, works as before |
| Old | New | Response with mode, old client ignores extra fields |
| New | Old | Empty response, client treats as NORMAL |
| New | New | Full functionality |

The key is that `SERVER_MODE_UNSPECIFIED (0)` is treated as `NORMAL`, so old servers returning empty responses are interpreted correctly.

---

## Worker State Machine

### States

```rust
pub enum WorkerStatus {
    /// Worker is initializing
    Initializing,

    /// Worker is registering with server
    Registering,

    /// Worker is active and polling for work
    Running {
        server_worker_id: Option<Uuid>,
        started_at: SystemTime,
    },

    /// Server requested drain - completing in-flight work
    Draining {
        reason: Option<String>,
        started_at: SystemTime,
        in_flight_count: usize,
    },

    /// Worker is disconnected from server
    Disconnected {
        disconnected_at: SystemTime,
        last_error: Option<String>,
        reconnect_attempts: u32,
    },

    /// Worker is shutting down (client-initiated)
    ShuttingDown {
        requested_at: SystemTime,
        in_flight_count: usize,
    },

    /// Worker has stopped
    Stopped {
        stopped_at: SystemTime,
        reason: StopReason,
    },
}
```

### State Transitions

```
                              ┌──────────────────────────────────────┐
                              │                                      │
                              ▼                                      │
                       ┌─────────────┐                               │
                       │ Initializing│                               │
                       └──────┬──────┘                               │
                              │ start()                              │
                              ▼                                      │
                       ┌─────────────┐                               │
                       │ Registering │                               │
                       └──────┬──────┘                               │
                              │ registration success                 │
                              ▼                                      │
          ┌───────────────────────────────────────┐                  │
          │                                       │                  │
          │                RUNNING                │◄─────────────────┤
          │         (polling for work)            │                  │
          │                                       │  heartbeat OK +  │
          └───────────┬───────────────┬───────────┘  mode=NORMAL     │
                      │               │                              │
   heartbeat OK +     │               │  heartbeat fails             │
   mode=DRAINING      │               │  (3 consecutive)             │
                      │               │                              │
                      ▼               ▼                              │
              ┌─────────────┐  ┌──────────────┐                      │
              │  DRAINING   │  │ DISCONNECTED │──────────────────────┘
              │ (completing)│  │  (retrying)  │  heartbeat OK +
              └──────┬──────┘  └──────────────┘  re-register
                     │
                     │ heartbeat fails
                     │ (3 consecutive)
                     │
                     ▼
              ┌──────────────┐
              │ DISCONNECTED │
              │  (retrying)  │
              └──────────────┘


  From any state except Stopped:

      stop() ──────────────► ShuttingDown ──────► Stopped
      abort() ─────────────────────────────────► Stopped
```

### Key Transitions

| From | To | Trigger |
|------|----|---------|
| Running | Draining | Heartbeat returns `mode=DRAINING` |
| Draining | Running | Heartbeat returns `mode=NORMAL` |
| Running | Disconnected | 3 consecutive heartbeat failures |
| Draining | Disconnected | 3 consecutive heartbeat failures |
| Disconnected | Running | Heartbeat succeeds + `mode=NORMAL` + re-register |
| Disconnected | Draining | Heartbeat succeeds + `mode=DRAINING` + re-register |

---

## Worker Behavior by State

### Running State

```rust
// Heartbeat loop
loop {
    sleep(heartbeat_interval).await;

    match client.send_heartbeat(worker_id).await {
        Ok(response) => {
            reset_failure_count();

            match response.mode() {
                ServerMode::Normal | ServerMode::Unspecified => {
                    // Continue normally
                }
                ServerMode::Draining => {
                    emit_event(DrainRequested {
                        reason: response.message
                    });
                    transition_to(Draining);
                }
            }
        }
        Err(e) => {
            increment_failure_count();
            if failure_count >= 3 {
                transition_to(Disconnected);
            }
        }
    }
}

// Polling loop (concurrent with heartbeat)
loop {
    if status != Running { break; }

    match poll_for_work().await {
        Ok(Some(work)) => execute(work),
        Ok(None) => continue,
        Err(e) => handle_poll_error(e),
    }
}
```

### Draining State

```rust
// Heartbeat loop continues
loop {
    sleep(heartbeat_interval).await;

    match client.send_heartbeat(worker_id).await {
        Ok(response) => {
            reset_failure_count();

            match response.mode() {
                ServerMode::Normal | ServerMode::Unspecified => {
                    emit_event(DrainCancelled);
                    transition_to(Running);
                    // Resume polling
                }
                ServerMode::Draining => {
                    // Stay in draining, update message if changed
                }
            }
        }
        Err(e) => {
            increment_failure_count();
            if failure_count >= 3 {
                transition_to(Disconnected);
            }
        }
    }
}

// Polling loop STOPPED - do not poll for new work
// In-flight work continues to completion
// When in_flight_count reaches 0, worker is fully drained
```

### Disconnected State

```rust
// Reconnection loop with exponential backoff
let mut attempt = 0;
loop {
    let delay = calculate_backoff(attempt);
    sleep(delay).await;
    attempt += 1;

    emit_event(Reconnecting { attempt });

    // Try heartbeat first (lightweight connectivity check)
    match client.send_heartbeat(worker_id).await {
        Ok(response) => {
            // Server is back! Re-register to refresh state
            match client.register_worker(...).await {
                Ok(registration) => {
                    emit_event(Reconnected);
                    update_registration(registration);

                    // Check mode to determine next state
                    match response.mode() {
                        ServerMode::Normal | ServerMode::Unspecified => {
                            transition_to(Running);
                        }
                        ServerMode::Draining => {
                            transition_to(Draining);
                        }
                    }
                    return; // Exit reconnection loop
                }
                Err(e) => {
                    // Registration failed, will retry
                    emit_event(RegistrationFailed { error: e });
                }
            }
        }
        Err(e) => {
            // Still disconnected
            emit_event(ReconnectFailed { error: e, attempt });
        }
    }
}
```

---

## Lifecycle Events

New events for drain scenarios:

```rust
pub enum WorkerLifecycleEvent {
    // ... existing events ...

    /// Server requested drain mode
    DrainRequested {
        reason: Option<String>,
        estimated_duration: Option<Duration>,
    },

    /// Server cancelled drain mode (back to normal)
    DrainCancelled,

    /// Worker is fully drained (no in-flight work)
    FullyDrained,

    /// Reconnection attempt
    Reconnecting {
        attempt: u32,
    },

    /// Reconnection failed
    ReconnectFailed {
        error: String,
        attempt: u32,
    },

    /// Successfully reconnected
    Reconnected,
}
```

---

## Server-Side Design

### Server State

```kotlin
class ServerState {
    var mode: ServerMode = ServerMode.NORMAL
    var drainMessage: String? = null
    var drainStartedAt: Instant? = null
    var estimatedDuration: Duration? = null
}
```

### Admin API

```kotlin
// REST API for server management
@RestController
class AdminController(private val serverState: ServerState) {

    @PostMapping("/admin/drain")
    fun initiateDrain(
        @RequestParam message: String?,
        @RequestParam estimatedMinutes: Long?
    ): ResponseEntity<*> {
        serverState.mode = ServerMode.DRAINING
        serverState.drainMessage = message
        serverState.drainStartedAt = Instant.now()
        serverState.estimatedDuration = estimatedMinutes?.let { Duration.ofMinutes(it) }

        log.info("Server entering drain mode: $message")
        return ResponseEntity.ok(mapOf("status" to "draining"))
    }

    @PostMapping("/admin/resume")
    fun resume(): ResponseEntity<*> {
        serverState.mode = ServerMode.NORMAL
        serverState.drainMessage = null
        serverState.drainStartedAt = null
        serverState.estimatedDuration = null

        log.info("Server resuming normal operation")
        return ResponseEntity.ok(mapOf("status" to "normal"))
    }

    @GetMapping("/admin/status")
    fun status(): ResponseEntity<*> {
        return ResponseEntity.ok(mapOf(
            "mode" to serverState.mode,
            "message" to serverState.drainMessage,
            "drainStartedAt" to serverState.drainStartedAt,
            "estimatedDuration" to serverState.estimatedDuration
        ))
    }
}
```

### Heartbeat Handler

```kotlin
@GrpcService
class WorkerLifecycleService(
    private val serverState: ServerState,
    private val workerRegistry: WorkerRegistry
) : WorkerLifecycleGrpcKt.WorkerLifecycleCoroutineImplBase() {

    override suspend fun sendHeartbeat(
        request: WorkerHeartbeatRequest
    ): WorkerHeartbeatResponse {
        // Update worker last-seen timestamp
        workerRegistry.recordHeartbeat(request.workerId)

        // Return current server mode
        return WorkerHeartbeatResponse.newBuilder()
            .setMode(serverState.mode)
            .apply {
                serverState.drainMessage?.let { setMessage(it) }
                serverState.estimatedDuration?.let {
                    setEstimatedDurationMs(it.toMillis())
                }
            }
            .setServerTimeMs(System.currentTimeMillis())
            .build()
    }
}
```

### Monitoring Workers During Drain

```kotlin
@RestController
class AdminController(
    private val serverState: ServerState,
    private val workerRegistry: WorkerRegistry
) {
    @GetMapping("/admin/workers")
    fun listWorkers(): ResponseEntity<*> {
        val workers = workerRegistry.getAllWorkers()
        return ResponseEntity.ok(mapOf(
            "serverMode" to serverState.mode,
            "workers" to workers.map { worker ->
                mapOf(
                    "workerId" to worker.id,
                    "workerName" to worker.name,
                    "lastHeartbeat" to worker.lastHeartbeat,
                    "inFlightWorkflows" to worker.inFlightWorkflows,
                    "inFlightTasks" to worker.inFlightTasks,
                    "status" to if (worker.isStale()) "stale" else "active"
                )
            },
            "summary" to mapOf(
                "totalWorkers" to workers.size,
                "activeWorkers" to workers.count { !it.isStale() },
                "totalInFlight" to workers.sumOf { it.inFlightWorkflows + it.inFlightTasks }
            )
        ))
    }

    @GetMapping("/admin/drain/status")
    fun drainStatus(): ResponseEntity<*> {
        val workers = workerRegistry.getAllWorkers()
        val totalInFlight = workers.sumOf { it.inFlightWorkflows + it.inFlightTasks }

        return ResponseEntity.ok(mapOf(
            "mode" to serverState.mode,
            "fullyDrained" to (totalInFlight == 0),
            "inFlightCount" to totalInFlight,
            "workersWithInFlight" to workers.filter {
                it.inFlightWorkflows + it.inFlightTasks > 0
            }.map { it.id }
        ))
    }
}
```

---

## Complete Maintenance Flow

### Timeline Diagram

```
Time ──────────────────────────────────────────────────────────────────►

Admin                 Server                      Worker A        Worker B
  │                     │                            │               │
  │ POST /admin/drain   │                            │               │
  │ message="Maint 2am" │                            │               │
  ├────────────────────►│                            │               │
  │                     │ mode=DRAINING              │               │
  │                     │                            │               │
  │                     │◄───────── heartbeat ───────┤               │
  │                     ├───── mode=DRAINING ───────►│               │
  │                     │                            │ stop polling  │
  │                     │                            │ complete work │
  │                     │                            │               │
  │                     │◄─────────────── heartbeat ─┼───────────────┤
  │                     ├───────── mode=DRAINING ────┼──────────────►│
  │                     │                            │               │ stop polling
  │                     │                            │               │ complete work
  │                     │                            │               │
  │ GET /admin/drain/   │                            │               │
  │     status          │                            │               │
  ├────────────────────►│                            │               │
  │◄────────────────────┤                            │               │
  │ {inFlight: 5}       │                            │               │
  │                     │                            │               │
  │        ... workers completing in-flight work ... │               │
  │                     │                            │               │
  │ GET /admin/drain/   │                            │               │
  │     status          │                            │               │
  ├────────────────────►│                            │               │
  │◄────────────────────┤                            │               │
  │ {inFlight: 0,       │                            │               │
  │  fullyDrained:true} │                            │               │
  │                     │                            │               │
  │ (shutdown server)   │                            │               │
  │                     │ X                          │               │
  │                     │                            │ heartbeat fail│
  │                     │                            │ DISCONNECTED  │
  │                     │                            │               │ heartbeat fail
  │                     │                            │               │ DISCONNECTED
  │                     │                            │               │
  │    ... server maintenance ... │                  │ (backoff)     │ (backoff)
  │                     │                            │               │
  │ (start server)      │                            │               │
  │                     │ ◄                          │               │
  │                     │                            │               │
  │                     │◄───────── heartbeat ───────┤               │
  │                     │◄──────── re-register ──────┤               │
  │                     ├───── mode=NORMAL ─────────►│               │
  │                     │                            │ RUNNING       │
  │                     │                            │ resume poll   │
  │                     │                            │               │
  │                     │◄─────────────── heartbeat ─┼───────────────┤
  │                     │◄────────────── re-register ┼───────────────┤
  │                     ├───────── mode=NORMAL ──────┼──────────────►│
  │                     │                            │               │ RUNNING
  │                     │                            │               │ resume poll
  │                     │                            │               │
```

### Operator Runbook

```markdown
## Server Maintenance Procedure

### 1. Initiate Drain
```bash
curl -X POST "http://server:8080/admin/drain?message=Scheduled%20maintenance&estimatedMinutes=30"
```

### 2. Monitor Drain Progress
```bash
# Check every 30 seconds until fullyDrained=true
while true; do
  curl -s "http://server:8080/admin/drain/status" | jq
  sleep 30
done
```

Expected response when ready:
```json
{
  "mode": "DRAINING",
  "fullyDrained": true,
  "inFlightCount": 0,
  "workersWithInFlight": []
}
```

### 3. Shutdown Server
```bash
# Safe to shutdown now
systemctl stop flovyn-server
```

### 4. Perform Maintenance
- Database migrations
- Config updates
- etc.

### 5. Start Server
```bash
systemctl start flovyn-server
```

Server starts in NORMAL mode by default. Workers will automatically reconnect.

### 6. Verify Recovery
```bash
curl -s "http://server:8080/admin/workers" | jq
```

Expected: all workers showing "active" status.
```

---

## SDK Implementation

### Configuration

```rust
impl FlovynClientBuilder {
    /// Set behavior when server requests drain
    pub fn on_drain_behavior(self, behavior: DrainBehavior) -> Self;
}

pub enum DrainBehavior {
    /// Comply with drain request (default)
    Comply,

    /// Ignore drain requests (for critical workers)
    Ignore,

    /// Custom handler
    Custom(Arc<dyn DrainHandler>),
}

#[async_trait]
pub trait DrainHandler: Send + Sync {
    /// Called when drain is requested. Return true to comply.
    async fn on_drain_requested(&self, reason: Option<&str>) -> bool;

    /// Called when drain is cancelled
    async fn on_drain_cancelled(&self);
}
```

### WorkerHandle Extensions

```rust
impl WorkerHandle {
    /// Check if worker is currently draining
    pub fn is_draining(&self) -> bool;

    /// Get drain info if draining
    pub fn drain_info(&self) -> Option<DrainInfo>;

    /// Check if worker is fully drained (no in-flight work)
    pub fn is_fully_drained(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct DrainInfo {
    pub reason: Option<String>,
    pub started_at: SystemTime,
    pub estimated_duration: Option<Duration>,
    pub in_flight_count: usize,
}
```

---

## E2E Tests

### Test: Server Drain Request

```rust
#[tokio::test]
async fn test_server_drain_request() {
    let server = start_test_server().await;
    let client = create_test_client(&server).await;

    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Verify running
    assert!(matches!(handle.status(), WorkerStatus::Running { .. }));

    // Server initiates drain
    server.set_mode(ServerMode::Draining, Some("Test maintenance")).await;

    // Wait for next heartbeat
    tokio::time::sleep(Duration::from_secs(35)).await;

    // Verify draining
    assert!(handle.is_draining());
    let info = handle.drain_info().unwrap();
    assert_eq!(info.reason, Some("Test maintenance".to_string()));

    // Server cancels drain
    server.set_mode(ServerMode::Normal, None).await;

    // Wait for next heartbeat
    tokio::time::sleep(Duration::from_secs(35)).await;

    // Verify running again
    assert!(matches!(handle.status(), WorkerStatus::Running { .. }));
    assert!(!handle.is_draining());

    handle.stop().await;
}
```

### Test: Drain Completes In-Flight Work

```rust
#[tokio::test]
async fn test_drain_completes_in_flight() {
    let server = start_test_server().await;
    let client = create_test_client(&server).await;

    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Start slow workflow
    client.start_workflow("slow-workflow", json!({"delay_ms": 5000})).await.unwrap();

    // Wait for workflow to start
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(handle.workflows_in_progress(), 1);

    // Server initiates drain
    server.set_mode(ServerMode::Draining, None).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Verify draining but not fully drained
    assert!(handle.is_draining());
    assert!(!handle.is_fully_drained());
    assert_eq!(handle.workflows_in_progress(), 1);

    // Wait for workflow to complete
    tokio::time::sleep(Duration::from_secs(6)).await;

    // Now fully drained
    assert!(handle.is_fully_drained());
    assert_eq!(handle.workflows_in_progress(), 0);

    handle.stop().await;
}
```

### Test: Reconnection After Server Restart

```rust
#[tokio::test]
async fn test_reconnection_after_server_restart() {
    let server = start_test_server().await;
    let client = create_test_client(&server).await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Subscribe to events
    let mut receiver = handle.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = receiver.recv().await {
            events_clone.lock().await.push(event);
        }
    });

    // Stop server
    server.stop().await;

    // Wait for disconnection
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(matches!(handle.status(), WorkerStatus::Disconnected { .. }));

    // Restart server
    server.start().await;

    // Wait for reconnection
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(matches!(handle.status(), WorkerStatus::Running { .. }));

    // Verify events
    let events = events.lock().await;
    assert!(events.iter().any(|e| matches!(e, WorkerLifecycleEvent::Disconnected { .. })));
    assert!(events.iter().any(|e| matches!(e, WorkerLifecycleEvent::Reconnecting { .. })));
    assert!(events.iter().any(|e| matches!(e, WorkerLifecycleEvent::Reconnected)));

    handle.stop().await;
}
```

### Test: Drain During Disconnection

```rust
#[tokio::test]
async fn test_reconnect_to_draining_server() {
    let server = start_test_server().await;
    let client = create_test_client(&server).await;

    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Stop server
    server.stop().await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(matches!(handle.status(), WorkerStatus::Disconnected { .. }));

    // Restart server in drain mode
    server.set_mode(ServerMode::Draining, Some("Pre-drained")).await;
    server.start().await;

    // Wait for reconnection
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Should be draining, not running
    assert!(handle.is_draining());

    handle.stop().await;
}
```

---

## Alternatives Considered

### 1. Notification Stream for Commands

Use the existing `SubscribeToNotifications` stream to send drain commands.

**Pros**:
- Real-time delivery
- No polling delay

**Cons**:
- Stream may be disconnected
- More complex state management
- Doesn't solve reconnection problem

### 2. Dedicated Control Stream

New bidirectional stream for control commands.

**Pros**:
- Clean separation of concerns
- Real-time bidirectional communication

**Cons**:
- Additional connection to maintain
- More infrastructure complexity
- Overkill for infrequent operations

### 3. Command with Acknowledgment

Server sends command, waits for worker acknowledgment.

**Pros**:
- Guaranteed delivery
- Clear state tracking

**Cons**:
- Complex distributed state
- What if worker crashes before ack?
- Doesn't scale well

---

## Open Questions

1. **Drain timeout**: Should there be a maximum drain duration after which server proceeds anyway?
   - Proposed: Configurable per-drain with default of 5 minutes

2. **Force drain**: Should there be a "force" option that cancels in-flight work?
   - Proposed: Separate mechanism via workflow cancellation API

3. **Partial drain**: Drain specific queues or workflow types?
   - Proposed: Future enhancement with queue-level mode

4. **Worker priority during drain**: Should some workers drain before others?
   - Proposed: Out of scope for initial implementation

---

## Implementation Phases

### Phase 1: Protocol (Server)
- Add `WorkerHeartbeatResponse` message
- Modify heartbeat handler to return mode
- Add admin API for mode control

### Phase 2: SDK Core
- Parse heartbeat response
- Implement mode-based state transitions
- Add drain-related lifecycle events

### Phase 3: SDK Extensions
- Add `DrainBehavior` configuration
- Add `DrainHandler` trait
- Add `WorkerHandle` drain methods

### Phase 4: Reconnection
- Implement reconnection with mode checking
- Re-registration after reconnect
- Backoff strategy

### Phase 5: Testing & Documentation
- E2E tests for all scenarios
- Operator runbook
- SDK documentation

---

## References

- [Worker Lifecycle Enhancement](./worker-lifecycle-enhancement.md) - Base lifecycle design
- [gRPC Protocol Definition](../../../proto/flovyn.proto) - Protocol buffers
- Kubernetes Pod Lifecycle - Inspiration for drain semantics
- AWS ELB Connection Draining - Industry pattern
