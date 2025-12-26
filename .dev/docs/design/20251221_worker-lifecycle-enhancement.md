# Design: Worker Lifecycle Enhancement

**Status**: Draft
**Created**: 2025-12-21
**Author**: Claude Code
**Related**: [grpc-client-apis-enhancement.md](./grpc-client-apis-enhancement.md)

## Table of Contents

1. [Overview](#overview)
2. [Current State Analysis](#current-state-analysis)
3. [Gap Analysis](#gap-analysis)
4. [Proposed Design](#proposed-design)
5. [Detailed API Specifications](#detailed-api-specifications)
6. [Implementation Details](#implementation-details)
7. [E2E Test Plan](#e2e-test-plan)
8. [Example Applications](#example-applications)
9. [Migration Guide](#migration-guide)
10. [Open Questions](#open-questions)

---

## Overview

This document provides a detailed design for enhancing the worker lifecycle management in the Rust SDK. The goal is to expose worker lifecycle functionality that is currently internal-only, align with the Kotlin SDK capabilities, and provide a robust API for production deployments.

### Goals

1. **Observability**: Expose worker registration status, health, and metrics
2. **Control**: Provide graceful shutdown, restart, and lifecycle hooks
3. **Resilience**: Automatic reconnection, heartbeat monitoring, conflict detection
4. **Testability**: Easy-to-test lifecycle events and state transitions

### Non-Goals

1. Worker clustering/coordination (server-side concern)
2. Auto-scaling (external orchestrator concern)
3. Worker-to-worker communication

---

## Current State Analysis

### Rust SDK Worker Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         FlovynClient                            │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │ WorkflowRegistry│  │  TaskRegistry   │  │    Channel      │ │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘ │
└───────────┼────────────────────┼────────────────────┼──────────┘
            │                    │                    │
            ▼                    ▼                    │
┌───────────────────────┐ ┌───────────────────────┐  │
│ WorkflowExecutorWorker│ │  TaskExecutorWorker   │  │
│  ├─ register_with_    │ │  ├─ register_with_    │  │
│  │   server()         │ │  │   server()         │  │
│  ├─ heartbeat_loop()  │ │  ├─ heartbeat_loop()  │  │
│  ├─ notification_     │ │  ├─ polling_loop()    │  │
│  │   subscription_    │ │  └─ execute_task()    │  │
│  │   loop()           │ │                       │  │
│  ├─ polling_loop()    │ └───────────────────────┘  │
│  └─ execute_workflow()│                            │
└───────────────────────┘                            │
            │                                        │
            ▼                                        ▼
┌─────────────────────────────────────────────────────────────────┐
│                    WorkerLifecycleClient (gRPC)                 │
│  ├─ register_worker(name, version, type, workflows, tasks)     │
│  ├─ send_heartbeat(worker_id)                                  │
│  └─ send_heartbeat_with_kinds(worker_id, workflows, tasks)     │
└─────────────────────────────────────────────────────────────────┘
```

### Current Internal Implementation

**Location**: `sdk/src/worker/workflow_worker.rs`, `sdk/src/worker/task_worker.rs`

#### WorkflowWorkerConfig

```rust
pub struct WorkflowWorkerConfig {
    pub worker_id: String,
    pub tenant_id: Uuid,
    pub task_queue: String,
    pub poll_timeout: Duration,           // Default: 60s
    pub max_concurrent: usize,            // Default: 10
    pub heartbeat_interval: Duration,     // Default: 30s
    pub worker_name: Option<String>,
    pub worker_version: String,           // Default: "1.0.0"
    pub space_id: Option<Uuid>,
    pub enable_auto_registration: bool,   // Default: true
    pub enable_notifications: bool,       // Default: true
    pub worker_token: String,
    pub enable_telemetry: bool,
}
```

#### Current Startup Flow

```rust
// In WorkflowExecutorWorker::start()
pub async fn start(&mut self) -> Result<()> {
    // 1. Register with server (best-effort)
    if let Err(e) = self.register_with_server().await {
        warn!("Failed to register worker: {}", e);
    }

    // 2. Start heartbeat loop
    tokio::spawn(Self::heartbeat_loop(...));

    // 3. Start notification subscription (if enabled)
    if self.config.enable_notifications {
        tokio::spawn(Self::notification_subscription_loop(...));
    }

    // 4. Signal ready
    self.ready_notify.notify_one();

    // 5. Enter polling loop
    while running.load(Ordering::SeqCst) {
        // Poll and execute...
    }
}
```

#### Current WorkerHandle

```rust
pub struct WorkerHandle {
    shutdown_tx: watch::Sender<bool>,
    workflow_running: Option<Arc<AtomicBool>>,
    task_running: Option<Arc<AtomicBool>>,
    workflow_handle: Option<JoinHandle<()>>,
    task_handle: Option<JoinHandle<()>>,
    workflow_ready: Option<Arc<Notify>>,
    task_ready: Option<Arc<Notify>>,
}

impl WorkerHandle {
    pub async fn await_ready(&self);
    pub async fn stop(self);
    pub async fn stop_graceful(self);
    pub fn abort(self);
}
```

### What's Exposed vs Internal

| Component | Current State | Should Be |
|-----------|---------------|-----------|
| `WorkerHandle` | Public | Public (enhanced) |
| `WorkerHandle::await_ready()` | Public | Public |
| `WorkerHandle::stop()` | Public | Public |
| `WorkerHandle::stop_graceful()` | Public | Public |
| `WorkerHandle::abort()` | Public | Public |
| Registration status | Internal | **Public** |
| Server-assigned worker ID | Internal | **Public** |
| Heartbeat status | Internal | **Public** |
| Connection status | Internal | **Public** |
| Conflict information | Internal | **Public** |
| Lifecycle events/hooks | None | **New** |
| Metrics | None | **New** |

---

## Gap Analysis

### Compared to Kotlin SDK

| Feature | Kotlin SDK | Rust SDK | Gap |
|---------|------------|----------|-----|
| Registration status | Exposed via logs | Internal only | Need public API |
| Server worker ID | `serverWorkerId` field | Internal field | Need getter |
| Conflict detection | `workflowConflictsList` logged | Logged only | Need structured access |
| `awaitReady()` | `suspend fun awaitReady()` | `async fn await_ready()` | ✓ Parity |
| `stop()` | `fun stop()` | `async fn stop()` | ✓ Parity |
| Heartbeat interval config | `heartbeatIntervalSeconds` | `heartbeat_interval` | ✓ Parity |
| Worker name/version | Configurable | Configurable | ✓ Parity |
| Space ID | Configurable | Configurable | ✓ Parity |
| Auto-registration toggle | `enableAutoRegistration` | `enable_auto_registration` | ✓ Parity |
| Lifecycle hooks | None | None | Both need this |
| Reconnection logic | Implicit via polling | Implicit | Need explicit API |
| Health check endpoint | None | None | Nice to have |
| Metrics export | None | None | Nice to have |

### Missing Public APIs

1. **Registration State**
   - Is the worker registered?
   - When was it registered?
   - What workflows/tasks were registered?
   - Were there conflicts?

2. **Connection State**
   - Is the worker connected to the server?
   - When was the last successful heartbeat?
   - Is reconnection in progress?

3. **Lifecycle Events**
   - Worker started
   - Worker registered
   - Worker disconnected
   - Worker reconnecting
   - Worker stopped

4. **Health Information**
   - Current poll status
   - In-flight workflow/task count
   - Error rates

---

## Proposed Design

### 1. WorkerStatus Enum

```rust
/// Current operational status of the worker
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerStatus {
    /// Worker is initializing, not yet polling
    Initializing,

    /// Worker is registering with the server
    Registering,

    /// Worker is active and polling for work
    Running {
        /// Server-assigned worker ID
        server_worker_id: Option<Uuid>,
        /// When the worker started running
        started_at: SystemTime,
    },

    /// Worker is connected but temporarily paused
    Paused {
        reason: String,
    },

    /// Worker is attempting to reconnect after connection loss
    Reconnecting {
        /// Number of reconnection attempts
        attempts: u32,
        /// When connection was lost
        disconnected_at: SystemTime,
        /// Last error message
        last_error: Option<String>,
    },

    /// Worker is shutting down gracefully
    ShuttingDown {
        /// When shutdown was requested
        requested_at: SystemTime,
        /// Number of in-flight tasks
        in_flight_count: usize,
    },

    /// Worker has stopped
    Stopped {
        /// When the worker stopped
        stopped_at: SystemTime,
        /// Reason for stopping
        reason: StopReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Normal graceful shutdown
    Graceful,
    /// Immediate stop requested
    Immediate,
    /// Aborted
    Aborted,
    /// Unrecoverable error
    Error(String),
}
```

### 2. RegistrationInfo Struct

```rust
/// Information about worker registration with the server
#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    /// Server-assigned worker ID
    pub worker_id: Uuid,

    /// Whether registration was successful
    pub success: bool,

    /// When the worker was registered
    pub registered_at: SystemTime,

    /// Registered workflow kinds
    pub workflow_kinds: Vec<String>,

    /// Registered task kinds
    pub task_kinds: Vec<String>,

    /// Any workflow registration conflicts
    pub workflow_conflicts: Vec<WorkflowConflict>,

    /// Any task registration conflicts
    pub task_conflicts: Vec<TaskConflict>,
}

#[derive(Debug, Clone)]
pub struct WorkflowConflict {
    /// The workflow kind that has a conflict
    pub kind: String,

    /// Reason for the conflict
    pub reason: String,

    /// ID of the existing worker that has this workflow
    pub existing_worker_id: String,
}

#[derive(Debug, Clone)]
pub struct TaskConflict {
    /// The task kind that has a conflict
    pub kind: String,

    /// Reason for the conflict
    pub reason: String,

    /// ID of the existing worker that has this task
    pub existing_worker_id: String,
}
```

### 3. ConnectionInfo Struct

```rust
/// Information about the worker's connection to the server
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    /// Whether currently connected
    pub connected: bool,

    /// Time of last successful heartbeat
    pub last_heartbeat: Option<SystemTime>,

    /// Time of last successful poll
    pub last_poll: Option<SystemTime>,

    /// Number of consecutive heartbeat failures
    pub heartbeat_failures: u32,

    /// Number of consecutive poll failures
    pub poll_failures: u32,

    /// Current reconnection attempt (if reconnecting)
    pub reconnect_attempt: Option<u32>,
}
```

### 4. WorkerMetrics Struct

```rust
/// Runtime metrics for the worker
#[derive(Debug, Clone, Default)]
pub struct WorkerMetrics {
    /// Total workflows executed since start
    pub workflows_executed: u64,

    /// Total workflows currently in progress
    pub workflows_in_progress: usize,

    /// Total tasks executed since start
    pub tasks_executed: u64,

    /// Total tasks currently in progress
    pub tasks_in_progress: usize,

    /// Total workflow execution errors
    pub workflow_errors: u64,

    /// Total task execution errors
    pub task_errors: u64,

    /// Average workflow execution time (milliseconds)
    pub avg_workflow_duration_ms: f64,

    /// Average task execution time (milliseconds)
    pub avg_task_duration_ms: f64,

    /// Time since worker started
    pub uptime: Duration,
}
```

### 5. WorkerLifecycleEvent Enum

```rust
/// Events emitted during worker lifecycle
#[derive(Debug, Clone)]
pub enum WorkerLifecycleEvent {
    /// Worker has started initializing
    Starting {
        worker_id: String,
        worker_name: Option<String>,
    },

    /// Worker has registered with server
    Registered {
        info: RegistrationInfo,
    },

    /// Registration failed
    RegistrationFailed {
        error: String,
        will_retry: bool,
    },

    /// Worker is now polling for work
    Ready {
        server_worker_id: Option<Uuid>,
    },

    /// Worker received work
    WorkReceived {
        work_type: WorkType,
        execution_id: Uuid,
    },

    /// Work completed successfully
    WorkCompleted {
        work_type: WorkType,
        execution_id: Uuid,
        duration: Duration,
    },

    /// Work failed
    WorkFailed {
        work_type: WorkType,
        execution_id: Uuid,
        error: String,
    },

    /// Connection to server lost
    Disconnected {
        error: String,
    },

    /// Attempting to reconnect
    Reconnecting {
        attempt: u32,
    },

    /// Successfully reconnected
    Reconnected,

    /// Heartbeat sent successfully
    HeartbeatSent,

    /// Heartbeat failed
    HeartbeatFailed {
        error: String,
        consecutive_failures: u32,
    },

    /// Shutdown requested
    ShuttingDown {
        graceful: bool,
    },

    /// Worker has stopped
    Stopped {
        reason: StopReason,
        uptime: Duration,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkType {
    Workflow,
    Task,
}
```

### 6. WorkerLifecycleHook Trait

```rust
/// Hook for observing and reacting to worker lifecycle events
#[async_trait]
pub trait WorkerLifecycleHook: Send + Sync {
    /// Called when a lifecycle event occurs
    async fn on_event(&self, event: WorkerLifecycleEvent) {
        // Default: no-op
        let _ = event;
    }

    /// Called when the worker status changes
    async fn on_status_change(&self, old: WorkerStatus, new: WorkerStatus) {
        // Default: no-op
        let _ = (old, new);
    }

    /// Called before the worker starts polling
    /// Return false to prevent the worker from starting
    async fn on_before_start(&self) -> bool {
        true
    }

    /// Called before the worker stops
    /// Can perform cleanup operations
    async fn on_before_stop(&self, graceful: bool) {
        let _ = graceful;
    }

    /// Called when the worker should pause (e.g., backpressure)
    async fn on_pause_requested(&self, reason: &str) -> bool {
        let _ = reason;
        true // Allow pause by default
    }

    /// Called when the worker should resume
    async fn on_resume_requested(&self) -> bool {
        true // Allow resume by default
    }
}
```

### 7. Enhanced WorkerHandle

```rust
/// Handle for controlling and monitoring a running worker
pub struct WorkerHandle {
    // Existing fields
    shutdown_tx: watch::Sender<bool>,
    workflow_running: Option<Arc<AtomicBool>>,
    task_running: Option<Arc<AtomicBool>>,
    workflow_handle: Option<JoinHandle<()>>,
    task_handle: Option<JoinHandle<()>>,
    workflow_ready: Option<Arc<Notify>>,
    task_ready: Option<Arc<Notify>>,

    // New fields
    status: Arc<RwLock<WorkerStatus>>,
    registration_info: Arc<RwLock<Option<RegistrationInfo>>>,
    connection_info: Arc<RwLock<ConnectionInfo>>,
    metrics: Arc<RwLock<WorkerMetrics>>,
    event_tx: broadcast::Sender<WorkerLifecycleEvent>,
    started_at: SystemTime,
}

impl WorkerHandle {
    // === Existing Methods ===

    /// Wait for the worker to be ready and polling
    pub async fn await_ready(&self);

    /// Stop the worker immediately (interrupts current poll)
    pub async fn stop(self);

    /// Stop the worker gracefully (finishes current poll)
    pub async fn stop_graceful(self);

    /// Abort the worker immediately without waiting
    pub fn abort(self);

    // === New Status Methods ===

    /// Get the current worker status
    pub fn status(&self) -> WorkerStatus;

    /// Check if the worker is currently running
    pub fn is_running(&self) -> bool;

    /// Check if the worker is connected to the server
    pub fn is_connected(&self) -> bool;

    /// Check if the worker is shutting down
    pub fn is_shutting_down(&self) -> bool;

    // === New Registration Methods ===

    /// Get registration information (None if not yet registered)
    pub fn registration(&self) -> Option<RegistrationInfo>;

    /// Get the server-assigned worker ID (None if not registered)
    pub fn server_worker_id(&self) -> Option<Uuid>;

    /// Check if there were any registration conflicts
    pub fn has_conflicts(&self) -> bool;

    /// Get workflow registration conflicts
    pub fn workflow_conflicts(&self) -> Vec<WorkflowConflict>;

    /// Get task registration conflicts
    pub fn task_conflicts(&self) -> Vec<TaskConflict>;

    // === New Connection Methods ===

    /// Get current connection information
    pub fn connection(&self) -> ConnectionInfo;

    /// Get time since last successful heartbeat
    pub fn time_since_heartbeat(&self) -> Option<Duration>;

    /// Get time since last successful poll
    pub fn time_since_poll(&self) -> Option<Duration>;

    // === New Metrics Methods ===

    /// Get current worker metrics
    pub fn metrics(&self) -> WorkerMetrics;

    /// Get the worker uptime
    pub fn uptime(&self) -> Duration;

    /// Get the number of in-flight workflows
    pub fn workflows_in_progress(&self) -> usize;

    /// Get the number of in-flight tasks
    pub fn tasks_in_progress(&self) -> usize;

    // === New Event Subscription ===

    /// Subscribe to lifecycle events
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerLifecycleEvent>;

    /// Subscribe to lifecycle events with a filter
    pub fn subscribe_filtered<F>(&self, filter: F) -> impl Stream<Item = WorkerLifecycleEvent>
    where
        F: Fn(&WorkerLifecycleEvent) -> bool + Send + 'static;

    // === New Control Methods ===

    /// Request the worker to pause (stop accepting new work)
    pub async fn pause(&self, reason: &str) -> Result<(), WorkerControlError>;

    /// Request the worker to resume accepting work
    pub async fn resume(&self) -> Result<(), WorkerControlError>;

    /// Force re-registration with the server
    pub async fn re_register(&self) -> Result<RegistrationInfo, WorkerControlError>;

    /// Trigger an immediate heartbeat
    pub async fn heartbeat_now(&self) -> Result<(), WorkerControlError>;
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerControlError {
    #[error("Worker is not running")]
    NotRunning,

    #[error("Worker is shutting down")]
    ShuttingDown,

    #[error("Operation not supported in current state: {0}")]
    InvalidState(String),

    #[error("Operation timed out")]
    Timeout,

    #[error("Server error: {0}")]
    ServerError(String),
}
```

### 8. FlovynClient Updates

```rust
impl FlovynClient {
    // === Existing Methods ===

    /// Start the worker and return a handle
    pub async fn start(&self) -> Result<WorkerHandle, ClientError>;

    // === New Methods ===

    /// Check if the client has a running worker
    pub fn has_running_worker(&self) -> bool;

    /// Get a reference to the current worker handle (if running)
    pub fn worker_handle(&self) -> Option<&WorkerHandle>;

    /// Register a lifecycle hook (must be called before start)
    pub fn add_lifecycle_hook<H: WorkerLifecycleHook + 'static>(
        &mut self,
        hook: H,
    ) -> &mut Self;

    /// Get the configured heartbeat interval
    pub fn heartbeat_interval(&self) -> Duration;

    /// Get the configured poll timeout
    pub fn poll_timeout(&self) -> Duration;

    /// Get the configured max concurrent workflows
    pub fn max_concurrent_workflows(&self) -> usize;

    /// Get the configured max concurrent tasks
    pub fn max_concurrent_tasks(&self) -> usize;
}
```

### 9. Builder Updates

```rust
impl FlovynClientBuilder {
    // === Existing Methods ===
    // ...

    // === New Methods ===

    /// Add a lifecycle hook
    pub fn add_lifecycle_hook<H: WorkerLifecycleHook + 'static>(
        self,
        hook: H,
    ) -> Self;

    /// Set the reconnection strategy
    pub fn reconnection_strategy(self, strategy: ReconnectionStrategy) -> Self;

    /// Enable or disable automatic re-registration on reconnect
    pub fn auto_reregister_on_reconnect(self, enabled: bool) -> Self;

    /// Set the maximum number of reconnection attempts
    pub fn max_reconnect_attempts(self, max: u32) -> Self;

    /// Set the initial reconnection delay
    pub fn reconnect_initial_delay(self, delay: Duration) -> Self;

    /// Set the maximum reconnection delay
    pub fn reconnect_max_delay(self, delay: Duration) -> Self;
}

/// Strategy for handling reconnection
#[derive(Debug, Clone)]
pub enum ReconnectionStrategy {
    /// No automatic reconnection
    None,

    /// Fixed delay between attempts
    Fixed {
        delay: Duration,
        max_attempts: Option<u32>,
    },

    /// Exponential backoff
    ExponentialBackoff {
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: f64,
        max_attempts: Option<u32>,
    },

    /// Custom strategy
    Custom(Arc<dyn ReconnectionPolicy>),
}

#[async_trait]
pub trait ReconnectionPolicy: Send + Sync {
    /// Calculate the delay before the next reconnection attempt
    /// Returns None to stop reconnecting
    async fn next_delay(&self, attempt: u32, last_error: &str) -> Option<Duration>;

    /// Called when reconnection succeeds
    async fn on_reconnected(&self);

    /// Reset the policy state
    async fn reset(&self);
}
```

---

## Detailed API Specifications

### WorkerHandle::status()

```rust
impl WorkerHandle {
    /// Get the current worker status
    ///
    /// # Example
    ///
    /// ```rust
    /// let handle = client.start().await?;
    /// handle.await_ready().await;
    ///
    /// match handle.status() {
    ///     WorkerStatus::Running { server_worker_id, started_at } => {
    ///         println!("Worker running since {:?}", started_at);
    ///         if let Some(id) = server_worker_id {
    ///             println!("Server ID: {}", id);
    ///         }
    ///     }
    ///     WorkerStatus::Reconnecting { attempts, .. } => {
    ///         println!("Reconnecting, attempt {}", attempts);
    ///     }
    ///     _ => {}
    /// }
    /// ```
    pub fn status(&self) -> WorkerStatus {
        self.status.read().unwrap().clone()
    }
}
```

### WorkerHandle::subscribe()

```rust
impl WorkerHandle {
    /// Subscribe to lifecycle events
    ///
    /// Returns a broadcast receiver that will receive all lifecycle events.
    /// The receiver is bounded; if the receiver falls behind, older events
    /// will be dropped.
    ///
    /// # Example
    ///
    /// ```rust
    /// let handle = client.start().await?;
    /// let mut events = handle.subscribe();
    ///
    /// tokio::spawn(async move {
    ///     while let Ok(event) = events.recv().await {
    ///         match event {
    ///             WorkerLifecycleEvent::WorkCompleted { work_type, duration, .. } => {
    ///                 println!("{:?} completed in {:?}", work_type, duration);
    ///             }
    ///             WorkerLifecycleEvent::HeartbeatFailed { consecutive_failures, .. } => {
    ///                 if consecutive_failures > 3 {
    ///                     eprintln!("WARNING: {} consecutive heartbeat failures", consecutive_failures);
    ///                 }
    ///             }
    ///             _ => {}
    ///         }
    ///     }
    /// });
    /// ```
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerLifecycleEvent> {
        self.event_tx.subscribe()
    }
}
```

### WorkerHandle::pause() and resume()

```rust
impl WorkerHandle {
    /// Request the worker to pause (stop accepting new work)
    ///
    /// When paused, the worker will:
    /// - Complete any in-flight work
    /// - Continue sending heartbeats
    /// - Stop polling for new work
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The worker is not running
    /// - The worker is already shutting down
    /// - A lifecycle hook rejected the pause
    ///
    /// # Example
    ///
    /// ```rust
    /// // Pause during maintenance
    /// handle.pause("scheduled maintenance").await?;
    ///
    /// // Do maintenance...
    ///
    /// // Resume normal operation
    /// handle.resume().await?;
    /// ```
    pub async fn pause(&self, reason: &str) -> Result<(), WorkerControlError> {
        let current = self.status();
        match current {
            WorkerStatus::Running { .. } => {
                // Implementation details...
            }
            WorkerStatus::ShuttingDown { .. } => {
                Err(WorkerControlError::ShuttingDown)
            }
            _ => {
                Err(WorkerControlError::InvalidState(format!("{:?}", current)))
            }
        }
    }

    /// Request the worker to resume accepting work
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The worker is not paused
    /// - The worker is shutting down
    /// - A lifecycle hook rejected the resume
    pub async fn resume(&self) -> Result<(), WorkerControlError> {
        // Implementation details...
    }
}
```

### Reconnection with Exponential Backoff

```rust
impl Default for ReconnectionStrategy {
    fn default() -> Self {
        ReconnectionStrategy::ExponentialBackoff {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: None, // Infinite retries
        }
    }
}

// Usage
let client = FlovynClient::builder()
    .server_address("localhost", 9090)
    .tenant_id(tenant_id)
    .worker_token(token)
    .reconnection_strategy(ReconnectionStrategy::ExponentialBackoff {
        initial_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(30),
        multiplier: 1.5,
        max_attempts: Some(10),
    })
    .build()
    .await?;
```

---

## Implementation Details

### 1. Status Tracking State Machine

```
                    ┌─────────────┐
                    │ Initializing│
                    └──────┬──────┘
                           │ start()
                           ▼
                    ┌─────────────┐
                    │ Registering │◄────────────────┐
                    └──────┬──────┘                 │
                           │ success/failure        │ re_register()
                           ▼                        │
    ┌───────────────┬─────────────┬────────────────┤
    │               │   Running   │                │
    │               └──────┬──────┘                │
    │                      │                       │
    │    ┌─────────────────┼─────────────────┐     │
    │    │ pause()         │ connection lost │     │
    │    ▼                 ▼                 │     │
    │ ┌──────┐      ┌─────────────┐          │     │
    │ │Paused│      │Reconnecting │──────────┘     │
    │ └──┬───┘      └──────┬──────┘ reconnected    │
    │    │ resume()        │                       │
    │    └─────────────────┤ max attempts          │
    │                      ▼                       │
    │               ┌─────────────┐                │
    │               │ShuttingDown │◄───────────────┘
    │               └──────┬──────┘ stop()
    │                      │
    │                      ▼
    │               ┌─────────────┐
    └──────────────►│   Stopped   │
      abort()       └─────────────┘
```

### 2. Event Broadcasting Architecture

```rust
// In WorkflowExecutorWorker
struct WorkerInternals {
    event_tx: broadcast::Sender<WorkerLifecycleEvent>,
    status: Arc<RwLock<WorkerStatus>>,
    connection_info: Arc<RwLock<ConnectionInfo>>,
    metrics: Arc<RwLock<WorkerMetrics>>,
}

impl WorkerInternals {
    fn emit(&self, event: WorkerLifecycleEvent) {
        // Update metrics based on event
        match &event {
            WorkerLifecycleEvent::WorkCompleted { work_type, duration, .. } => {
                let mut metrics = self.metrics.write().unwrap();
                match work_type {
                    WorkType::Workflow => {
                        metrics.workflows_executed += 1;
                        metrics.workflows_in_progress -= 1;
                        // Update average duration...
                    }
                    WorkType::Task => {
                        metrics.tasks_executed += 1;
                        metrics.tasks_in_progress -= 1;
                    }
                }
            }
            WorkerLifecycleEvent::HeartbeatSent => {
                let mut conn = self.connection_info.write().unwrap();
                conn.last_heartbeat = Some(SystemTime::now());
                conn.heartbeat_failures = 0;
            }
            // ... handle other events
            _ => {}
        }

        // Broadcast event (ignore if no receivers)
        let _ = self.event_tx.send(event);
    }

    fn update_status(&self, new_status: WorkerStatus) {
        let old_status = {
            let mut status = self.status.write().unwrap();
            let old = status.clone();
            *status = new_status.clone();
            old
        };

        // Emit status change event
        self.emit(WorkerLifecycleEvent::StatusChanged {
            old: old_status,
            new: new_status,
        });
    }
}
```

### 3. Heartbeat Monitoring Enhancement

```rust
async fn heartbeat_loop(
    internals: Arc<WorkerInternals>,
    channel: Channel,
    worker_token: String,
    server_worker_id: Uuid,
    heartbeat_interval: Duration,
    running: Arc<AtomicBool>,
) {
    let mut client = WorkerLifecycleClient::new(channel, &worker_token);
    let mut consecutive_failures = 0u32;

    while running.load(Ordering::SeqCst) {
        tokio::time::sleep(heartbeat_interval).await;

        if !running.load(Ordering::SeqCst) {
            break;
        }

        match client.send_heartbeat(server_worker_id).await {
            Ok(_) => {
                if consecutive_failures > 0 {
                    debug!("Heartbeat recovered after {} failures", consecutive_failures);
                }
                consecutive_failures = 0;
                internals.emit(WorkerLifecycleEvent::HeartbeatSent);
            }
            Err(e) => {
                consecutive_failures += 1;
                warn!(
                    "Heartbeat failed (attempt {}): {}",
                    consecutive_failures, e
                );
                internals.emit(WorkerLifecycleEvent::HeartbeatFailed {
                    error: e.to_string(),
                    consecutive_failures,
                });

                // Update connection info
                {
                    let mut conn = internals.connection_info.write().unwrap();
                    conn.heartbeat_failures = consecutive_failures;
                }

                // Trigger reconnection if too many failures
                if consecutive_failures >= 3 {
                    internals.update_status(WorkerStatus::Reconnecting {
                        attempts: 0,
                        disconnected_at: SystemTime::now(),
                        last_error: Some(e.to_string()),
                    });
                }
            }
        }
    }
}
```

### 4. Reconnection Implementation

```rust
async fn reconnection_loop(
    internals: Arc<WorkerInternals>,
    strategy: ReconnectionStrategy,
    channel: Channel,
    config: WorkerConfig,
    running: Arc<AtomicBool>,
) {
    let mut attempt = 0u32;

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        // Calculate delay
        let delay = match &strategy {
            ReconnectionStrategy::None => {
                internals.update_status(WorkerStatus::Stopped {
                    stopped_at: SystemTime::now(),
                    reason: StopReason::Error("Connection lost, no reconnection configured".into()),
                });
                return;
            }
            ReconnectionStrategy::Fixed { delay, max_attempts } => {
                if let Some(max) = max_attempts {
                    if attempt >= *max {
                        internals.update_status(WorkerStatus::Stopped {
                            stopped_at: SystemTime::now(),
                            reason: StopReason::Error(format!(
                                "Max reconnection attempts ({}) reached",
                                max
                            )),
                        });
                        return;
                    }
                }
                *delay
            }
            ReconnectionStrategy::ExponentialBackoff {
                initial_delay,
                max_delay,
                multiplier,
                max_attempts,
            } => {
                if let Some(max) = max_attempts {
                    if attempt >= *max {
                        internals.update_status(WorkerStatus::Stopped {
                            stopped_at: SystemTime::now(),
                            reason: StopReason::Error(format!(
                                "Max reconnection attempts ({}) reached",
                                max
                            )),
                        });
                        return;
                    }
                }
                let delay = initial_delay.mul_f64(multiplier.powi(attempt as i32));
                delay.min(*max_delay)
            }
            ReconnectionStrategy::Custom(policy) => {
                match policy.next_delay(attempt, "").await {
                    Some(delay) => delay,
                    None => {
                        internals.update_status(WorkerStatus::Stopped {
                            stopped_at: SystemTime::now(),
                            reason: StopReason::Error("Reconnection policy stopped".into()),
                        });
                        return;
                    }
                }
            }
        };

        attempt += 1;
        internals.emit(WorkerLifecycleEvent::Reconnecting { attempt });
        internals.update_status(WorkerStatus::Reconnecting {
            attempts: attempt,
            disconnected_at: SystemTime::now(),
            last_error: None,
        });

        // Wait before attempting
        tokio::time::sleep(delay).await;

        // Try to reconnect
        match try_reconnect(&channel, &config).await {
            Ok(info) => {
                internals.emit(WorkerLifecycleEvent::Reconnected);
                internals.update_status(WorkerStatus::Running {
                    server_worker_id: Some(info.worker_id),
                    started_at: SystemTime::now(),
                });

                // Update registration info
                {
                    let mut reg = internals.registration_info.write().unwrap();
                    *reg = Some(info);
                }

                // Reset connection info
                {
                    let mut conn = internals.connection_info.write().unwrap();
                    conn.connected = true;
                    conn.heartbeat_failures = 0;
                    conn.poll_failures = 0;
                    conn.reconnect_attempt = None;
                }

                return;
            }
            Err(e) => {
                warn!("Reconnection attempt {} failed: {}", attempt, e);
                internals.update_status(WorkerStatus::Reconnecting {
                    attempts: attempt,
                    disconnected_at: SystemTime::now(),
                    last_error: Some(e.to_string()),
                });
            }
        }
    }
}
```

### 5. Lifecycle Hook Integration

```rust
struct HookChain {
    hooks: Vec<Arc<dyn WorkerLifecycleHook>>,
}

impl HookChain {
    async fn emit(&self, event: WorkerLifecycleEvent) {
        for hook in &self.hooks {
            hook.on_event(event.clone()).await;
        }
    }

    async fn on_status_change(&self, old: WorkerStatus, new: WorkerStatus) {
        for hook in &self.hooks {
            hook.on_status_change(old.clone(), new.clone()).await;
        }
    }

    async fn before_start(&self) -> bool {
        for hook in &self.hooks {
            if !hook.on_before_start().await {
                return false;
            }
        }
        true
    }

    async fn before_stop(&self, graceful: bool) {
        for hook in &self.hooks {
            hook.on_before_stop(graceful).await;
        }
    }
}

// Usage in worker
impl WorkflowExecutorWorker {
    pub async fn start(&mut self) -> Result<()> {
        // Check hooks before starting
        if !self.hook_chain.before_start().await {
            return Err(FlovynError::Other("Startup rejected by hook".into()));
        }

        // ... normal startup
    }
}
```

---

## E2E Test Plan

### Test File: `tests/e2e/test_worker_lifecycle.rs`

```rust
use flovyn_sdk::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Test that worker status transitions correctly
#[tokio::test]
async fn test_worker_status_transitions() {
    let client = test_client().await;
    let handle = client.start().await.unwrap();

    // Initially should be running after await_ready
    handle.await_ready().await;
    assert!(matches!(handle.status(), WorkerStatus::Running { .. }));
    assert!(handle.is_running());

    // Request graceful stop
    handle.stop_graceful().await;
    assert!(matches!(handle.status(), WorkerStatus::Stopped { reason: StopReason::Graceful, .. }));
}

/// Test registration info is available
#[tokio::test]
async fn test_registration_info() {
    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .tenant_id(test_tenant_id())
        .worker_token(test_token())
        .worker_name("test-worker")
        .worker_version("1.2.3")
        .register_workflow(EchoWorkflow)
        .register_task(EchoTask)
        .build()
        .await
        .unwrap();

    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Should have registration info
    let info = handle.registration().expect("Should be registered");
    assert!(info.success);
    assert!(info.workflow_kinds.contains(&"echo".to_string()));
    assert!(info.task_kinds.contains(&"echo-task".to_string()));

    // Server worker ID should be available
    assert!(handle.server_worker_id().is_some());

    handle.stop().await;
}

/// Test conflict detection when registering duplicate workflow
#[tokio::test]
async fn test_registration_conflicts() {
    // Start first worker
    let client1 = test_client_with_workflow(ConflictWorkflow).await;
    let handle1 = client1.start().await.unwrap();
    handle1.await_ready().await;

    // Start second worker with same workflow (should detect conflict)
    let client2 = test_client_with_workflow(ConflictWorkflow).await;
    let handle2 = client2.start().await.unwrap();
    handle2.await_ready().await;

    // Second worker should have conflicts
    let conflicts = handle2.workflow_conflicts();
    assert!(!conflicts.is_empty());
    assert_eq!(conflicts[0].kind, "conflict-workflow");

    handle1.stop().await;
    handle2.stop().await;
}

/// Test lifecycle event subscription
#[tokio::test]
async fn test_lifecycle_events() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let client = test_client().await;
    let handle = client.start().await.unwrap();

    // Subscribe to events
    let mut receiver = handle.subscribe();
    let events_task = tokio::spawn(async move {
        while let Ok(event) = receiver.recv().await {
            events_clone.lock().await.push(event);
        }
    });

    handle.await_ready().await;

    // Start a workflow to generate events
    let workflow_id = client
        .start_workflow("echo", json!({"message": "test"}))
        .await
        .unwrap();

    // Wait for completion
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Stop worker
    handle.stop().await;
    let _ = events_task.await;

    // Check events
    let collected_events = events.lock().await;
    assert!(collected_events.iter().any(|e| matches!(e, WorkerLifecycleEvent::Ready { .. })));
    assert!(collected_events.iter().any(|e| matches!(e, WorkerLifecycleEvent::WorkReceived { .. })));
    assert!(collected_events.iter().any(|e| matches!(e, WorkerLifecycleEvent::WorkCompleted { .. })));
    assert!(collected_events.iter().any(|e| matches!(e, WorkerLifecycleEvent::Stopped { .. })));
}

/// Test worker pause and resume
#[tokio::test]
async fn test_pause_resume() {
    let client = test_client().await;
    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Pause the worker
    handle.pause("test pause").await.unwrap();
    assert!(matches!(handle.status(), WorkerStatus::Paused { .. }));

    // Start a workflow - it should queue but not be processed
    client.start_workflow("slow-workflow", json!({})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Worker should still be paused, workflow not started
    assert!(matches!(handle.status(), WorkerStatus::Paused { .. }));
    assert_eq!(handle.workflows_in_progress(), 0);

    // Resume
    handle.resume().await.unwrap();
    assert!(matches!(handle.status(), WorkerStatus::Running { .. }));

    // Workflow should now be picked up
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(handle.metrics().workflows_executed >= 1);

    handle.stop().await;
}

/// Test metrics collection
#[tokio::test]
async fn test_worker_metrics() {
    let client = test_client().await;
    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Execute some workflows
    for i in 0..5 {
        client
            .start_workflow("echo", json!({"message": format!("test-{}", i)}))
            .await
            .unwrap();
    }

    // Wait for completion
    tokio::time::sleep(Duration::from_secs(3)).await;

    let metrics = handle.metrics();
    assert_eq!(metrics.workflows_executed, 5);
    assert!(metrics.avg_workflow_duration_ms > 0.0);
    assert!(metrics.uptime > Duration::from_secs(3));

    handle.stop().await;
}

/// Test connection info tracking
#[tokio::test]
async fn test_connection_info() {
    let client = test_client().await;
    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Wait for at least one heartbeat
    tokio::time::sleep(Duration::from_secs(35)).await;

    let conn = handle.connection();
    assert!(conn.connected);
    assert!(conn.last_heartbeat.is_some());
    assert_eq!(conn.heartbeat_failures, 0);

    // Time since heartbeat should be less than heartbeat interval
    let since_heartbeat = handle.time_since_heartbeat().unwrap();
    assert!(since_heartbeat < Duration::from_secs(35));

    handle.stop().await;
}

/// Test lifecycle hook
#[tokio::test]
async fn test_lifecycle_hook() {
    #[derive(Default)]
    struct TestHook {
        started: AtomicU32,
        registered: AtomicU32,
        stopped: AtomicU32,
    }

    #[async_trait]
    impl WorkerLifecycleHook for TestHook {
        async fn on_event(&self, event: WorkerLifecycleEvent) {
            match event {
                WorkerLifecycleEvent::Starting { .. } => {
                    self.started.fetch_add(1, Ordering::SeqCst);
                }
                WorkerLifecycleEvent::Registered { .. } => {
                    self.registered.fetch_add(1, Ordering::SeqCst);
                }
                WorkerLifecycleEvent::Stopped { .. } => {
                    self.stopped.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    }

    let hook = Arc::new(TestHook::default());

    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .tenant_id(test_tenant_id())
        .worker_token(test_token())
        .register_workflow(EchoWorkflow)
        .add_lifecycle_hook(hook.clone())
        .build()
        .await
        .unwrap();

    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    assert_eq!(hook.started.load(Ordering::SeqCst), 1);
    assert_eq!(hook.registered.load(Ordering::SeqCst), 1);

    handle.stop().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(hook.stopped.load(Ordering::SeqCst), 1);
}

/// Test graceful shutdown waits for in-flight work
#[tokio::test]
async fn test_graceful_shutdown_waits() {
    let client = test_client().await;
    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Start a slow workflow
    client
        .start_workflow("slow-workflow", json!({"delay_ms": 3000}))
        .await
        .unwrap();

    // Wait for it to start processing
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(handle.workflows_in_progress(), 1);

    // Request graceful shutdown
    let start = std::time::Instant::now();
    handle.stop_graceful().await;
    let elapsed = start.elapsed();

    // Should have waited for workflow to complete
    assert!(elapsed >= Duration::from_secs(2));
}

/// Test reconnection on heartbeat failure
#[tokio::test]
async fn test_reconnection_on_failure() {
    // This test requires ability to simulate server disconnection
    // Skip if not available
    if !can_simulate_disconnect() {
        return;
    }

    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .tenant_id(test_tenant_id())
        .worker_token(test_token())
        .register_workflow(EchoWorkflow)
        .reconnection_strategy(ReconnectionStrategy::ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
            multiplier: 2.0,
            max_attempts: Some(5),
        })
        .build()
        .await
        .unwrap();

    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Simulate disconnect
    simulate_disconnect();

    // Should enter reconnecting state
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(matches!(handle.status(), WorkerStatus::Reconnecting { .. }));

    // Restore connection
    restore_connection();

    // Should reconnect
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(matches!(handle.status(), WorkerStatus::Running { .. }));

    handle.stop().await;
}

/// Test uptime tracking
#[tokio::test]
async fn test_uptime() {
    let client = test_client().await;
    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    let uptime1 = handle.uptime();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let uptime2 = handle.uptime();

    assert!(uptime2 > uptime1);
    assert!(uptime2 - uptime1 >= Duration::from_secs(2));

    handle.stop().await;
}
```

### Test File: `tests/e2e/test_worker_reconnection.rs`

```rust
/// Test that worker continues after transient errors
#[tokio::test]
async fn test_continues_after_transient_poll_error() {
    let client = test_client().await;
    let handle = client.start().await.unwrap();
    handle.await_ready().await;

    // Track events
    let mut events = handle.subscribe();
    let reconnect_events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = reconnect_events.clone();

    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if matches!(event, WorkerLifecycleEvent::Reconnecting { .. } | WorkerLifecycleEvent::Reconnected) {
                events_clone.lock().await.push(event);
            }
        }
    });

    // Even with poll errors, worker should recover
    // This relies on the server returning errors occasionally for testing
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Worker should still be running
    assert!(handle.is_running());

    handle.stop().await;
}

/// Test max reconnection attempts
#[tokio::test]
async fn test_max_reconnect_attempts() {
    // Use unreachable server
    let client = FlovynClient::builder()
        .server_address("localhost", 19999) // Wrong port
        .tenant_id(test_tenant_id())
        .worker_token(test_token())
        .register_workflow(EchoWorkflow)
        .reconnection_strategy(ReconnectionStrategy::Fixed {
            delay: Duration::from_millis(50),
            max_attempts: Some(3),
        })
        .build()
        .await
        .unwrap();

    let handle = client.start().await.unwrap();

    // Should stop after max attempts
    tokio::time::sleep(Duration::from_secs(2)).await;

    match handle.status() {
        WorkerStatus::Stopped { reason: StopReason::Error(msg), .. } => {
            assert!(msg.contains("Max reconnection attempts"));
        }
        _ => panic!("Expected stopped with error"),
    }
}
```

---

## Example Applications

### Example 1: Production Worker with Monitoring

```rust
// examples/production-worker/src/main.rs

use flovyn_sdk::prelude::*;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Logging hook for production monitoring
struct LoggingHook;

#[async_trait]
impl WorkerLifecycleHook for LoggingHook {
    async fn on_event(&self, event: WorkerLifecycleEvent) {
        match event {
            WorkerLifecycleEvent::Starting { worker_id, worker_name } => {
                info!(worker_id, ?worker_name, "Worker starting");
            }
            WorkerLifecycleEvent::Registered { info } => {
                info!(
                    worker_id = %info.worker_id,
                    workflows = ?info.workflow_kinds,
                    tasks = ?info.task_kinds,
                    "Worker registered"
                );
                if !info.workflow_conflicts.is_empty() {
                    warn!(conflicts = ?info.workflow_conflicts, "Workflow conflicts detected");
                }
            }
            WorkerLifecycleEvent::Ready { server_worker_id } => {
                info!(?server_worker_id, "Worker ready and polling");
            }
            WorkerLifecycleEvent::WorkCompleted { work_type, execution_id, duration } => {
                info!(
                    ?work_type,
                    %execution_id,
                    duration_ms = duration.as_millis(),
                    "Work completed"
                );
            }
            WorkerLifecycleEvent::WorkFailed { work_type, execution_id, error } => {
                error!(
                    ?work_type,
                    %execution_id,
                    error,
                    "Work failed"
                );
            }
            WorkerLifecycleEvent::Disconnected { error } => {
                error!(error, "Disconnected from server");
            }
            WorkerLifecycleEvent::Reconnecting { attempt } => {
                warn!(attempt, "Attempting to reconnect");
            }
            WorkerLifecycleEvent::Reconnected => {
                info!("Reconnected to server");
            }
            WorkerLifecycleEvent::HeartbeatFailed { consecutive_failures, error } => {
                if consecutive_failures >= 3 {
                    error!(consecutive_failures, error, "Multiple heartbeat failures");
                }
            }
            WorkerLifecycleEvent::Stopped { reason, uptime } => {
                info!(?reason, uptime_secs = uptime.as_secs(), "Worker stopped");
            }
            _ => {}
        }
    }

    async fn on_status_change(&self, old: WorkerStatus, new: WorkerStatus) {
        info!(?old, ?new, "Worker status changed");
    }
}

/// Metrics exporter hook
struct MetricsHook {
    // In production, this would be a metrics client (Prometheus, StatsD, etc.)
}

#[async_trait]
impl WorkerLifecycleHook for MetricsHook {
    async fn on_event(&self, event: WorkerLifecycleEvent) {
        match event {
            WorkerLifecycleEvent::WorkCompleted { work_type, duration, .. } => {
                // metrics.histogram("work.duration", duration.as_millis());
                // metrics.counter("work.completed", 1, [("type", work_type)]);
            }
            WorkerLifecycleEvent::WorkFailed { work_type, .. } => {
                // metrics.counter("work.failed", 1, [("type", work_type)]);
            }
            WorkerLifecycleEvent::HeartbeatFailed { consecutive_failures, .. } => {
                // metrics.gauge("heartbeat.failures", consecutive_failures);
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::init();

    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .tenant_id(std::env::var("FLOVYN_TENANT_ID")?.parse()?)
        .worker_token(std::env::var("FLOVYN_WORKER_TOKEN")?)
        .worker_name("production-worker")
        .worker_version(env!("CARGO_PKG_VERSION"))
        .heartbeat_interval(Duration::from_secs(15))
        .max_concurrent_workflows(10)
        .max_concurrent_tasks(20)
        .reconnection_strategy(ReconnectionStrategy::ExponentialBackoff {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            max_attempts: None, // Keep trying forever
        })
        .add_lifecycle_hook(LoggingHook)
        .add_lifecycle_hook(MetricsHook {})
        .register_workflow(OrderWorkflow)
        .register_task(PaymentTask)
        .register_task(InventoryTask)
        .build()
        .await?;

    let handle = client.start().await?;
    info!("Worker started");

    // Wait for ready
    handle.await_ready().await;
    info!("Worker ready");

    // Print initial status
    if let Some(info) = handle.registration() {
        info!(
            "Registered with server ID: {}, workflows: {:?}, tasks: {:?}",
            info.worker_id, info.workflow_kinds, info.task_kinds
        );
    }

    // Handle shutdown signals
    let shutdown_signal = async {
        tokio::signal::ctrl_c().await.ok();
    };

    // Periodic status logging
    let status_handle = handle.clone();
    let status_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let metrics = status_handle.metrics();
            info!(
                workflows_executed = metrics.workflows_executed,
                tasks_executed = metrics.tasks_executed,
                workflows_in_progress = metrics.workflows_in_progress,
                tasks_in_progress = metrics.tasks_in_progress,
                uptime_secs = metrics.uptime.as_secs(),
                "Worker metrics"
            );
        }
    });

    shutdown_signal.await;
    info!("Shutdown signal received");

    status_task.abort();
    handle.stop_graceful().await;
    info!("Worker stopped gracefully");

    Ok(())
}
```

### Example 2: Worker with Health Check Endpoint

```rust
// examples/health-check-worker/src/main.rs

use axum::{routing::get, Json, Router};
use flovyn_sdk::prelude::*;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    worker_status: String,
    connected: bool,
    uptime_secs: u64,
    workflows_executed: u64,
    tasks_executed: u64,
    last_heartbeat_secs_ago: Option<u64>,
}

struct AppState {
    worker_handle: Option<WorkerHandle>,
}

async fn health_check(state: axum::extract::State<Arc<RwLock<AppState>>>) -> Json<HealthResponse> {
    let state = state.read().await;

    if let Some(ref handle) = state.worker_handle {
        let status = handle.status();
        let metrics = handle.metrics();
        let conn = handle.connection();

        let worker_status = match &status {
            WorkerStatus::Running { .. } => "running",
            WorkerStatus::Paused { .. } => "paused",
            WorkerStatus::Reconnecting { .. } => "reconnecting",
            WorkerStatus::ShuttingDown { .. } => "shutting_down",
            WorkerStatus::Stopped { .. } => "stopped",
            _ => "initializing",
        };

        let last_heartbeat_secs_ago = handle
            .time_since_heartbeat()
            .map(|d| d.as_secs());

        Json(HealthResponse {
            status: if handle.is_running() && conn.connected {
                "healthy".to_string()
            } else {
                "unhealthy".to_string()
            },
            worker_status: worker_status.to_string(),
            connected: conn.connected,
            uptime_secs: metrics.uptime.as_secs(),
            workflows_executed: metrics.workflows_executed,
            tasks_executed: metrics.tasks_executed,
            last_heartbeat_secs_ago,
        })
    } else {
        Json(HealthResponse {
            status: "not_started".to_string(),
            worker_status: "not_started".to_string(),
            connected: false,
            uptime_secs: 0,
            workflows_executed: 0,
            tasks_executed: 0,
            last_heartbeat_secs_ago: None,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app_state = Arc::new(RwLock::new(AppState { worker_handle: None }));

    // Start HTTP server for health checks
    let app = Router::new()
        .route("/health", get(health_check))
        .with_state(app_state.clone());

    let http_server = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // Start Flovyn worker
    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .tenant_id(/* ... */)
        .worker_token(/* ... */)
        .register_workflow(MyWorkflow)
        .build()
        .await?;

    let handle = client.start().await?;
    handle.await_ready().await;

    // Store handle for health checks
    {
        let mut state = app_state.write().await;
        state.worker_handle = Some(handle.clone());
    }

    // Wait for shutdown
    tokio::signal::ctrl_c().await?;

    handle.stop_graceful().await;
    http_server.abort();

    Ok(())
}
```

### Example 3: Worker with Pause/Resume for Maintenance

```rust
// examples/maintenance-worker/src/main.rs

use flovyn_sdk::prelude::*;
use tokio::sync::mpsc;

enum MaintenanceCommand {
    Pause(String),
    Resume,
    Status,
    Shutdown,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = FlovynClient::builder()
        .server_address("localhost", 9090)
        .tenant_id(/* ... */)
        .worker_token(/* ... */)
        .register_workflow(MyWorkflow)
        .build()
        .await?;

    let handle = client.start().await?;
    handle.await_ready().await;

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<MaintenanceCommand>(10);

    // Command handler
    let handle_clone = handle.clone();
    let cmd_handler = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                MaintenanceCommand::Pause(reason) => {
                    match handle_clone.pause(&reason).await {
                        Ok(_) => println!("Worker paused: {}", reason),
                        Err(e) => eprintln!("Failed to pause: {}", e),
                    }
                }
                MaintenanceCommand::Resume => {
                    match handle_clone.resume().await {
                        Ok(_) => println!("Worker resumed"),
                        Err(e) => eprintln!("Failed to resume: {}", e),
                    }
                }
                MaintenanceCommand::Status => {
                    let status = handle_clone.status();
                    let metrics = handle_clone.metrics();
                    println!("Status: {:?}", status);
                    println!("Workflows executed: {}", metrics.workflows_executed);
                    println!("In progress: {}", metrics.workflows_in_progress);
                }
                MaintenanceCommand::Shutdown => {
                    break;
                }
            }
        }
    });

    // Simple CLI for demonstration
    let cmd_tx_clone = cmd_tx.clone();
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut reader = tokio::io::BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            if reader.read_line(&mut line).await.is_err() {
                break;
            }

            let parts: Vec<&str> = line.trim().split_whitespace().collect();
            match parts.as_slice() {
                ["pause", reason @ ..] => {
                    let reason = reason.join(" ");
                    cmd_tx_clone.send(MaintenanceCommand::Pause(reason)).await.ok();
                }
                ["resume"] => {
                    cmd_tx_clone.send(MaintenanceCommand::Resume).await.ok();
                }
                ["status"] => {
                    cmd_tx_clone.send(MaintenanceCommand::Status).await.ok();
                }
                ["quit"] | ["exit"] => {
                    cmd_tx_clone.send(MaintenanceCommand::Shutdown).await.ok();
                    break;
                }
                _ => {
                    println!("Commands: pause <reason>, resume, status, quit");
                }
            }
        }
    });

    cmd_handler.await?;
    handle.stop_graceful().await;

    Ok(())
}
```

---

## Migration Guide

### From Current API

The existing API remains fully compatible. New features are additive:

```rust
// Before (still works)
let client = FlovynClient::builder()
    .server_address("localhost", 9090)
    .tenant_id(tenant_id)
    .worker_token(token)
    .register_workflow(MyWorkflow)
    .build()
    .await?;

let handle = client.start().await?;
handle.await_ready().await;
// ... use client ...
handle.stop().await;

// After (with new features)
let client = FlovynClient::builder()
    .server_address("localhost", 9090)
    .tenant_id(tenant_id)
    .worker_token(token)
    .register_workflow(MyWorkflow)
    // New: Add lifecycle hooks
    .add_lifecycle_hook(MyLoggingHook)
    // New: Configure reconnection
    .reconnection_strategy(ReconnectionStrategy::ExponentialBackoff {
        initial_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(60),
        multiplier: 2.0,
        max_attempts: None,
    })
    .build()
    .await?;

let handle = client.start().await?;
handle.await_ready().await;

// New: Access status
println!("Status: {:?}", handle.status());

// New: Access registration info
if let Some(info) = handle.registration() {
    println!("Server worker ID: {}", info.worker_id);
}

// New: Subscribe to events
let mut events = handle.subscribe();
tokio::spawn(async move {
    while let Ok(event) = events.recv().await {
        println!("Event: {:?}", event);
    }
});

// New: Access metrics
println!("Uptime: {:?}", handle.uptime());

handle.stop_graceful().await;
```

---

## Open Questions

1. **Event Buffer Size**: What should be the default buffer size for the event broadcast channel? Too small risks dropping events; too large wastes memory.
   - Proposed: 256 events

2. **Heartbeat Failure Threshold**: After how many consecutive heartbeat failures should we trigger reconnection?
   - Proposed: 3 failures (configurable)

3. **Pause Behavior**: When paused, should we complete in-flight work or pause immediately?
   - Proposed: Complete in-flight work (more graceful)

4. **Metrics Persistence**: Should metrics survive reconnection/restart?
   - Proposed: No, reset on restart (simpler, matches uptime semantics)

5. **Hook Execution Order**: Should hooks be executed in registration order or concurrently?
   - Proposed: Sequential in registration order (predictable)

6. **Clone Semantics for WorkerHandle**: Should `WorkerHandle` be `Clone`? If so, what happens when one clone calls `stop()`?
   - Proposed: Yes, cloneable. All clones share the same underlying worker; `stop()` affects all.

---

## References

- [Kotlin SDK WorkerLifecycleClient](file:///Users/manhha/Developer/manhha/leanapp/flovyn/sdk/kotlin/src/main/kotlin/ai/flovyn/sdk/kotlin/client/WorkerLifecycleClient.kt)
- [Kotlin SDK WorkflowExecutorWorker](file:///Users/manhha/Developer/manhha/leanapp/flovyn/sdk/kotlin/src/main/kotlin/ai/flovyn/sdk/kotlin/worker/WorkflowExecutorWorker.kt)
- [Rust SDK WorkflowExecutorWorker](file:///Users/manhha/Developer/manhha/flovyn/sdk-rust/sdk/src/worker/workflow_worker.rs)
- [Rust SDK WorkerLifecycleClient](file:///Users/manhha/Developer/manhha/flovyn/sdk-rust/sdk/src/client/worker_lifecycle.rs)
- [gRPC Service Definitions](file:///Users/manhha/Developer/manhha/flovyn/sdk-rust/sdk/proto/flovyn/v1/)
