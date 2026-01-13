# Distributed Locks and Entity Patterns

This document researches how workflow orchestration platforms handle distributed locking and entity state management, comparing Azure Durable Functions, Temporal, and potential Flovyn implementations.

---

## Problem Statement

In distributed systems, we often need to coordinate access to shared resources. A classic example is a bank transfer:

```
1. Check source account has sufficient balance
2. Withdraw from source account
3. Deposit to destination account
```

Without coordination, concurrent transfers could cause race conditions (double-spending, overdrafts).

---

## Azure Durable Functions Approach

Azure Durable Functions provides built-in **Durable Entities** with **Critical Sections**:

```python
def transfer_safe_orchestration(context: df.DurableOrchestrationContext):
    [source, dest, amount] = context.get_input()

    # Critical Section: Acquire a lock for each entity
    with (yield context.lock([source, dest])):
        # Make sure that the source account has adequate balance
        source_balance = yield context.call_entity(source, "get")
        if source_balance < amount:
            return False
        else:
            # Wait for transfer to complete
            yield context.task_all([
                context.call_entity(source, "withdraw", amount),
                context.call_entity(dest, "deposit", amount)
            ])
            return True
```

### Key Features

| Feature | Description |
|---------|-------------|
| **Durable Entities** | Actor-like stateful objects with named operations |
| **`context.lock()`** | Built-in distributed lock primitive for multiple resources |
| **`call_entity()`** | Invoke operations on entities (get, withdraw, deposit) |
| **Automatic deadlock prevention** | Locks acquired in deterministic order |

### Durable Entities vs Orchestrations (Important!)

**Key insight from DurableTask source code analysis:** Entities ARE internally event-sourced like orchestrations, but they **automatically use continue-as-new after each batch** of operations. This is the secret to avoiding history accumulation.

#### Internal Architecture (from DurableTask source)

```csharp
// SchedulerState - the entity's persisted state (src/DurableTask.Core/Entities/SchedulerState.cs)
[DataContract]
internal class SchedulerState
{
    // The serialized entity state (JSON string)
    [DataMember(Name = "state", EmitDefaultValue = false)]
    public string? EntityState { get; set; }

    // Queue of waiting operations
    [DataMember(Name = "queue", EmitDefaultValue = false)]
    public Queue<RequestMessage>? Queue { get; private set; }

    // Lock information
    [DataMember(Name = "lockedBy", EmitDefaultValue = false)]
    public string? LockedBy { get; set; }

    // Message ordering/deduplication
    [DataMember(Name = "sorter", EmitDefaultValue = false)]
    public MessageSorter MessageSorter { get; set; }
}
```

**The trick: Automatic history reset after each batch**

```csharp
// From TaskEntityDispatcher.OnProcessWorkItemAsync
// After processing operations, entity ALWAYS continues-as-new:
var serializedSchedulerState = this.SerializeSchedulerStateForNextExecution(schedulerState);
var nextExecutionStartedEvent = new ExecutionStartedEvent(-1, serializedSchedulerState)
{
    OrchestrationInstance = workItem.OrchestrationRuntimeState.OrchestrationInstance,
};

// This creates an immediate continue-as-new, resetting history
runtimeState.AddEvent(nextExecutionStartedEvent);
```

| Aspect | Durable Entities | Orchestrations |
|--------|------------------|----------------|
| **Internal Model** | Event-sourced (same as orchestrations) | Event-sourced |
| **State Storage** | Serialized in SchedulerState JSON | Derived from replay |
| **History Management** | **Auto continue-as-new after each batch** | Manual continue-as-new |
| **History Size** | Always minimal (1-2 events) | Can grow unbounded |
| **Execution Model** | Batch of operations → reset | Long-running with checkpoints |

#### Why This Matters

The entity execution flow:
```
1. Receive batch of operations (1 to N messages)
2. Deserialize SchedulerState from ExecutionStartedEvent input
3. Execute all operations in batch
4. Serialize updated SchedulerState
5. Complete with ContinueAsNew → new execution with fresh history
```

This means:
- **History never grows** - reset after each batch
- **No manual continue-as-new needed** - framework handles it
- **State is checkpoint-based** - not replay-based
- **Still durable** - SchedulerState survives failures

```csharp
// User-facing entity code is simple:
public class Counter
{
    [JsonProperty("value")]
    public int CurrentValue { get; set; }

    public void Add(int amount) => this.CurrentValue += amount;
    public void Reset() => this.CurrentValue = 0;
    public int Get() => this.CurrentValue;
}
// Framework handles: serialization, history reset, locking, message ordering
```

#### Lock Chain Implementation

Entities support distributed locking via lock chains (for critical sections):

```csharp
// Lock request message contains:
[DataMember(Name = "lockset")]
public EntityId[]? LockSet { get; set; }    // Sorted set of entities to lock

[DataMember(Name = "pos")]
public int Position { get; set; }           // Position in lock chain

// Lock chain mechanism:
// Orchestration → Entity1 (pos=0) → Entity2 (pos=1) → ... → EntityN (pos=N)
// Each entity increments position and forwards to next
// Last entity sends response back to orchestration
```

This is a clever distributed locking pattern without a central lock manager.

### Pros
- Clean, intuitive API
- Built-in deadlock prevention (lock chains with sorted acquisition)
- Server-managed lock state
- **Automatic history reset** - framework handles continue-as-new
- Message ordering and deduplication built-in (MessageSorter)
- Operation batching for efficiency

### Cons
- Tight coupling to Azure infrastructure
- Entity programming model requires learning curve
- No audit trail of operations (only current state in SchedulerState)
- Lock chains add latency (sequential entity traversal)

---

## Temporal Approach

Temporal does NOT have built-in locks or entities. Instead, it provides patterns:

### Pattern 1: Mutex Workflow

A dedicated "mutex workflow" per resource manages lock acquisition/release via signals.

**Source:** [github.com/temporalio/samples-go/mutex](https://pkg.go.dev/github.com/temporalio/samples-go/mutex)

```
┌─────────────────┐         ┌─────────────────┐
│ Transfer        │         │ Mutex Workflow  │
│ Workflow        │         │ (per resource)  │
├─────────────────┤         ├─────────────────┤
│ 1. Signal       │────────>│ Queue request   │
│    "acquire"    │         │                 │
│                 │<────────│ Grant lock      │
│ 2. Do work      │         │ (signal back)   │
│                 │         │                 │
│ 3. Signal       │────────>│ Release lock    │
│    "release"    │         │ Process next    │
└─────────────────┘         └─────────────────┘
```

**How it works:**
1. Workflow needing lock signals mutex workflow with "acquire"
2. Mutex workflow queues requests and grants locks sequentially
3. After critical section, workflow signals "release"
4. Mutex workflow processes next queued request

### Pattern 2: Entity Workflow

Model each entity as a long-running workflow. Operations are sent as signals and processed sequentially.

**Source:** [docs.temporal.io/develop/typescript/entity-pattern](https://docs.temporal.io/develop/typescript/entity-pattern)

```typescript
export async function entityWorkflow(input: Input, isNew = true): Promise<void> {
  const pendingUpdates = Array<Update>();

  // Register signal handler
  setHandler(updateSignal, (updateCommand) => {
    pendingUpdates.push(updateCommand);
  });

  if (isNew) {
    await setup(input);
  }

  for (let iteration = 1; iteration <= MAX_ITERATIONS; ++iteration) {
    // Wait for updates (with timeout to ensure continue-as-new)
    await condition(() => pendingUpdates.length > 0, '1 day');

    // Process updates sequentially
    while (pendingUpdates.length) {
      const update = pendingUpdates.shift();
      await runAnActivityOrChildWorkflow(update);
    }
  }

  // Continue-as-new to avoid event history limits
  await continueAsNew<typeof entityWorkflow>(input, false);
}
```

**Key insight:** A workflow execution is naturally single-threaded, so all operations are serialized without explicit locks.

### Event History Limitation (Critical!)

Unlike Azure Durable Entities, Temporal's Entity Workflow pattern **does suffer from event history accumulation**:

```
Entity Workflow processes 10,000 operations over time:
├── Event 1: WorkflowStarted
├── Event 2: SignalReceived (operation 1)
├── Event 3: ActivityScheduled
├── Event 4: ActivityCompleted
├── ...
├── Event 40,000: SignalReceived (operation 10,000)
└── Event 40,001: ... → Approaching 50K limit!
```

**Temporal's solution: Continue-As-New**

```typescript
// Must periodically restart to clear history
if (operationCount > 1000) {
  await continueAsNew<typeof entityWorkflow>(currentState, false);
}
```

This resets the event history but requires:
- Careful state serialization
- Handling in-flight operations during transition
- Additional complexity in entity logic

### Pros
- No special server support needed
- Flexible, composable patterns
- Works with standard workflow primitives
- Full audit trail (all operations in history)

### Cons
- More complex to implement
- Requires understanding of patterns
- No built-in deadlock prevention
- **Event history accumulation** - requires continue-as-new for long-lived entities

---

## Flovyn Current Capabilities

Flovyn SDK provides these relevant primitives:

| Primitive | Description | Use for Locking |
|-----------|-------------|-----------------|
| **Promises** | External signals that workflows can await | Coordinate lock acquisition |
| **Workflow State** | `get/set` key-value storage per workflow | Store entity state |
| **Child Workflows** | Spawn and await sub-workflows | Entity workflow pattern |
| **`join_all`** | Wait for multiple futures | Parallel operations |
| **Timers** | Durable sleep | Timeout on lock acquisition |

### Missing Features
- No built-in `context.lock()` primitive
- No durable entities abstraction
- No `SignalWithStart` equivalent (start workflow if not exists, signal if exists)

---

## SignalWithStart vs Idempotency Key

These are related but serve different purposes. Understanding the distinction is critical for implementing the Entity Pattern.

### SignalWithStart (Temporal)

**SignalWithStart** is an atomic operation that:
1. Starts a workflow if it doesn't already exist (by workflow ID)
2. Sends a signal to the workflow (whether newly started or already running)

```go
// Temporal Go SDK
client.SignalWithStartWorkflow(
    ctx,
    workflowID,           // e.g., "account-123"
    signalName,           // e.g., "operation"
    signalArg,            // e.g., WithdrawOperation{amount: 100}
    workflowOptions,
    workflowFunc,
    workflowArgs,
)
```

**Why it's needed for Entity Pattern:**

```
Account "account-123" entity workflow:
- First operation:  SignalWithStart creates workflow + sends signal
- Second operation: SignalWithStart just sends signal (workflow exists)
- Third operation:  SignalWithStart just sends signal (workflow exists)
```

**Without SignalWithStart, there's a race condition:**

```
Thread A: Check if workflow exists? No → Start workflow
Thread B: Check if workflow exists? No → Start workflow  ← CONFLICT!
```

Or worse:

```
Thread A: Check if workflow exists? No
Thread B: Check if workflow exists? No → Start workflow
Thread A: Start workflow ← CONFLICT! (workflow already exists)
```

### Idempotency Key (Flovyn Current)

**Idempotency key** ensures the same request doesn't create duplicate resources:

```rust
// Flovyn - for promises (webhook correlation)
ctx.promise_with_options_raw("payment", PromiseOptions::with_key("stripe:ch_abc123"))

// Flovyn - for tasks (external job correlation)
ctx.schedule_with_options_raw("task", input, ScheduleTaskOptions::with_key("job:batch_123"))
```

**Use case:** Correlating with external systems, deduplicating webhook retries.

### Key Differences

| Aspect | SignalWithStart | Idempotency Key |
|--------|-----------------|-----------------|
| **Purpose** | Atomically ensure workflow exists + receive signal | Deduplicate requests by external ID |
| **Scope** | Workflow lifecycle management | Single operation deduplication |
| **Atomicity** | Start + Signal in one atomic operation | Single operation is idempotent |
| **Use case** | Route operations to entity workflows | Webhook deduplication, external correlation |
| **Race condition** | Prevents duplicate workflow starts | Prevents duplicate task/promise creation |

### How They Work Together

In a complete entity system, you'd use both:

```
1. External request arrives: "Withdraw $100 from account-123"
2. SignalWithStart: Ensure account-123 workflow exists + send withdraw signal
3. Idempotency Key: The withdraw operation itself has key "withdraw:txn-456"
   - If webhook retries, same key = same operation (no double withdrawal)
```

### What Flovyn Needs for Entity Pattern

**Option A: Full SignalWithStart API (Recommended)**

```rust
// Atomic: start workflow if not exists + send signal
client.signal_with_start_workflow(
    SignalWithStartOptions {
        workflow_id: "account-123",
        workflow_kind: "account-entity",
        workflow_input: json!({ "account_id": "123" }),
        signal_name: "operation",
        signal_data: json!({ "type": "withdraw", "amount": 100 }),
    }
).await?;
```

**Option B: Idempotent Start (Simpler but has race window)**

```rust
// Server returns existing workflow if ID matches (instead of error)
let execution = client.get_or_start_workflow(
    GetOrStartOptions {
        workflow_id: "account-123",
        workflow_kind: "account-entity",
        input: json!({ "account_id": "123" }),
    }
).await?;

// Then signal separately (small race window here)
client.signal_workflow("account-123", "operation", operation).await?;
```

**Option B** is simpler but has a small race window between start and signal where:
- Another caller might signal first
- Workflow might complete before signal arrives

**Option A** is truly atomic and preferred for production entity patterns.

### Server Implementation for SignalWithStart

```sql
-- Pseudocode for atomic SignalWithStart
BEGIN TRANSACTION;

-- Try to get existing workflow
SELECT * FROM workflow_executions
WHERE workflow_id = 'account-123' AND status = 'RUNNING'
FOR UPDATE;

IF NOT FOUND THEN
    -- Create new workflow
    INSERT INTO workflow_executions (workflow_id, kind, input, status)
    VALUES ('account-123', 'account-entity', '{"account_id": "123"}', 'RUNNING');
END IF;

-- Always append signal event
INSERT INTO workflow_events (workflow_id, event_type, data)
VALUES ('account-123', 'SIGNAL_RECEIVED', '{"name": "operation", "data": {...}}');

COMMIT;
```

---

## Proposed Flovyn Implementations

### Approach 1: Entity Workflow Pattern (Recommended)

Model each entity (e.g., bank account) as a long-running workflow:

```rust
use flovyn_sdk::prelude::*;
use serde::{Deserialize, Serialize};

// === Entity Operations ===

#[derive(Serialize, Deserialize, Clone)]
pub enum AccountOperation {
    GetBalance,
    Withdraw { amount: i64 },
    Deposit { amount: i64 },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AccountResponse {
    pub balance: i64,
    pub success: bool,
    pub error: Option<String>,
}

// === Account Entity Workflow ===

#[derive(Default)]
pub struct AccountEntityWorkflow;

#[async_trait]
impl WorkflowDefinition for AccountEntityWorkflow {
    type Input = String;  // account_id
    type Output = ();

    fn kind(&self) -> &str { "account-entity" }

    async fn execute(&self, ctx: &dyn WorkflowContext, account_id: Self::Input) -> Result<()> {
        // Initialize or restore balance from state
        let mut balance: i64 = ctx.get_typed("balance").await?.unwrap_or(0);
        let mut operation_count: u64 = ctx.get_typed("op_count").await?.unwrap_or(0);

        loop {
            // Wait for next operation (with timeout for continue-as-new)
            let op_result = select(
                ctx.promise::<AccountOperation>("operation"),
                ctx.sleep(Duration::from_secs(86400)), // 1 day
            ).await;

            let op = match op_result {
                Either::Left(op) => op?,
                Either::Right(_) => {
                    // Timeout - could implement continue-as-new here
                    continue;
                }
            };

            // Process operation (single-threaded = automatic serialization)
            let response = match op {
                AccountOperation::GetBalance => {
                    AccountResponse {
                        balance,
                        success: true,
                        error: None,
                    }
                }
                AccountOperation::Withdraw { amount } => {
                    if balance >= amount {
                        balance -= amount;
                        ctx.set_typed("balance", balance).await?;
                        AccountResponse {
                            balance,
                            success: true,
                            error: None,
                        }
                    } else {
                        AccountResponse {
                            balance,
                            success: false,
                            error: Some("Insufficient funds".to_string()),
                        }
                    }
                }
                AccountOperation::Deposit { amount } => {
                    balance += amount;
                    ctx.set_typed("balance", balance).await?;
                    AccountResponse {
                        balance,
                        success: true,
                        error: None,
                    }
                }
            };

            // Track operation count
            operation_count += 1;
            ctx.set_typed("op_count", operation_count).await?;

            // Store response for caller
            ctx.set_typed("last_response", &response).await?;
        }
    }
}
```

**Transfer workflow using entity pattern:**

```rust
#[derive(Serialize, Deserialize)]
pub struct TransferRequest {
    pub source_account: String,
    pub dest_account: String,
    pub amount: i64,
}

#[derive(Default)]
pub struct TransferWorkflow;

#[async_trait]
impl WorkflowDefinition for TransferWorkflow {
    type Input = TransferRequest;
    type Output = bool;

    fn kind(&self) -> &str { "transfer" }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<bool> {
        // Helper to call entity operation
        async fn call_account(
            ctx: &dyn WorkflowContext,
            account_id: &str,
            op: AccountOperation,
        ) -> Result<AccountResponse> {
            // In practice, this would signal the account entity workflow
            // and wait for response via a correlation mechanism
            //
            // For now, this is pseudocode showing the concept
            todo!("Implement entity call mechanism")
        }

        // 1. Check source balance
        let balance_response = call_account(
            ctx,
            &input.source_account,
            AccountOperation::GetBalance
        ).await?;

        if balance_response.balance < input.amount {
            return Ok(false);
        }

        // 2. Withdraw from source (serialized within source entity)
        let withdraw_response = call_account(
            ctx,
            &input.source_account,
            AccountOperation::Withdraw { amount: input.amount },
        ).await?;

        if !withdraw_response.success {
            return Ok(false);
        }

        // 3. Deposit to destination (serialized within dest entity)
        let deposit_response = call_account(
            ctx,
            &input.dest_account,
            AccountOperation::Deposit { amount: input.amount },
        ).await?;

        // Note: If deposit fails, we should compensate (saga pattern)
        if !deposit_response.success {
            // Rollback: re-deposit to source
            call_account(
                ctx,
                &input.source_account,
                AccountOperation::Deposit { amount: input.amount },
            ).await?;
            return Ok(false);
        }

        Ok(true)
    }
}
```

### Approach 2: Mutex Workflow Pattern

For cases requiring explicit multi-resource locking:

```rust
#[derive(Serialize, Deserialize)]
pub struct LockRequest {
    pub requester_workflow_id: Uuid,
    pub correlation_id: String,
}

#[derive(Default)]
pub struct MutexWorkflow;

#[async_trait]
impl WorkflowDefinition for MutexWorkflow {
    type Input = String;  // resource_id
    type Output = ();

    fn kind(&self) -> &str { "mutex" }

    async fn execute(&self, ctx: &dyn WorkflowContext, resource_id: Self::Input) -> Result<()> {
        loop {
            // Wait for lock acquisition request
            let lock_request: LockRequest = ctx.promise("acquire").await?;

            // Lock is granted (only one workflow reaches here at a time)
            // The requester is notified via their correlation promise

            // Wait for release signal from the lock holder
            let _: () = ctx.promise("release").await?;

            // Lock released, loop to grant next request
        }
    }
}

/// Helper to acquire multiple locks with deadlock prevention
pub async fn with_locks<F, T>(
    ctx: &dyn WorkflowContext,
    resources: &[&str],
    f: F,
) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    // Sort resources to prevent deadlocks (consistent ordering)
    let mut sorted: Vec<&str> = resources.to_vec();
    sorted.sort();

    // Acquire locks in order
    for resource in &sorted {
        acquire_lock(ctx, resource).await?;
    }

    // Execute critical section
    let result = f.await;

    // Release locks in reverse order
    for resource in sorted.iter().rev() {
        release_lock(ctx, resource).await?;
    }

    result
}
```

### Approach 3: Future Enhancement - Native Lock Primitive

Add server-side lock support for cleaner API:

```rust
// Hypothetical future API
#[async_trait]
pub trait WorkflowContext {
    // ... existing methods ...

    /// Acquire distributed locks on multiple resources.
    /// Returns a guard that releases locks on drop.
    /// Resources are locked in sorted order to prevent deadlocks.
    async fn lock(&self, resources: &[&str]) -> Result<LockGuard>;
}

// Usage would be clean like Azure:
async fn execute(&self, ctx: &dyn WorkflowContext, input: TransferRequest) -> Result<bool> {
    let _guard = ctx.lock(&[&input.source, &input.dest]).await?;

    let source_balance = get_balance(ctx, &input.source).await?;
    if source_balance < input.amount {
        return Ok(false);
    }

    join_all([
        withdraw(ctx, &input.source, input.amount),
        deposit(ctx, &input.dest, input.amount),
    ]).await?;

    Ok(true)
    // Locks automatically released when _guard drops
}
```

**Server implementation requirements:**
- Lock state stored durably (survives restarts)
- Lock acquisition recorded in event log
- Automatic deadlock detection or timeout
- Lock release on workflow completion/failure

---

## Comparison Summary

| Feature | Azure Durable Functions | Temporal | Flovyn (Current) | Flovyn (Target) |
|---------|------------------------|----------|------------------|-----------------|
| Built-in Entities | Yes (auto-reset) | No (workflow pattern) | No (workflow pattern) | Yes (auto-reset) |
| Entity Internal Model | Event-sourced + auto CAN* | Event-sourced | Event-sourced | Event-sourced + auto CAN |
| Entity History Size | Always minimal (auto-reset) | Can grow (manual CAN) | Can grow | Minimal (auto-reset) |
| Entity State Storage | SchedulerState JSON | Workflow state | Workflow state | EntityState JSON |
| Built-in Lock | Yes (lock chains) | No (pattern) | No (pattern) | Yes (lock chains) |
| Signals/Messages | Via entities | Signals | Promises | Promises + Signals |
| SignalWithStart | Yes (via entities) | Yes | No | Yes |
| Idempotency Key | Yes | Yes | Yes | Yes |
| Message Ordering | MessageSorter | Manual | Manual | MessageSorter |
| Workflow State | Yes | Yes | Yes (`get/set`) | Yes |
| Parallel Execution | `task_all` | Yes | `join_all` | `join_all` |
| Deadlock Prevention | Automatic (sorted locks) | Manual (sort) | Manual (sort) | Automatic |
| Continue-as-New | Auto (entities) / Manual (orch) | Manual | No | Auto (entities) / Manual (orch) |
| Reliability | Event-sourced | Event-sourced | Event-sourced | Event-sourced |

*CAN = Continue-As-New

**Note:** Direct state storage (non-event-sourced) is not viable due to reliability concerns.

---

## Architectural Decision: Entity State Model

Based on analysis of the DurableTask source code, Flovyn has three options:

### Option A: Entity Workflow Pattern (like Temporal)

Model entities as long-running workflows with event-sourced state. User must manually handle continue-as-new.

```
Entity "account-123":
├── WorkflowStarted
├── SignalReceived: Deposit $100
├── StateUpdated: balance = 100
├── SignalReceived: Withdraw $50
├── StateUpdated: balance = 50
├── ... (history grows forever)
└── User code must call Continue-As-New after N operations
```

**Pros:**
- Uses existing workflow infrastructure
- Full audit trail of all operations
- No new server primitives needed

**Cons:**
- Event history accumulation (requires manual continue-as-new)
- Replay overhead grows with operation count
- User must manage history size

### Option B: Auto-Reset Entity Pattern (like Azure DurableTask)

Use existing workflow infrastructure but **automatically continue-as-new after each batch**.

```
Entity "account-123" execution flow:

Batch 1:
├── ExecutionStartedEvent { input: SchedulerState { balance: 0, queue: [] } }
├── EventRaisedEvent: Deposit $100
├── EventRaisedEvent: Withdraw $50
└── ContinueAsNewEvent { input: SchedulerState { balance: 50, queue: [] } }

Batch 2 (new execution, fresh history):
├── ExecutionStartedEvent { input: SchedulerState { balance: 50, queue: [] } }
├── EventRaisedEvent: Deposit $25
└── ContinueAsNewEvent { input: SchedulerState { balance: 75, queue: [] } }
```

**Key insight from Azure:** The framework handles continue-as-new automatically after each operation batch. User code never sees history management.

```rust
// Hypothetical Flovyn API
#[entity]  // Marker that enables auto-reset behavior
pub struct AccountEntity {
    balance: i64,
}

impl AccountEntity {
    pub fn deposit(&mut self, amount: i64) {
        self.balance += amount;
    }
    pub fn withdraw(&mut self, amount: i64) -> bool {
        if self.balance >= amount {
            self.balance -= amount;
            true
        } else {
            false
        }
    }
}
// Framework auto-serializes state and continues-as-new after each batch
```

**Pros:**
- Uses existing workflow infrastructure (no new storage primitives)
- **No manual continue-as-new** - framework handles it
- History always minimal (1 batch worth of events)
- Can add lock chain support incrementally

**Cons:**
- Requires server support for entity dispatching
- No audit trail (only current state preserved)
- Need to implement SchedulerState equivalent (state + queue + locks)

### ~~Option C: First-Class Entities (Separate Primitive)~~ - NOT VIABLE

~~Implement entities as a completely separate primitive with direct state storage (not event-sourced at all).~~

**Why this is NOT an option:**

Direct state storage without event sourcing is **not reliable**:
- No replay capability on failure
- State updates are not atomic with operation completion
- Partial failures can leave state inconsistent
- No built-in exactly-once guarantees

Event sourcing provides critical durability guarantees:
- Operations are logged before execution
- State can be reconstructed from events
- Failures during execution can be retried safely
- Exactly-once semantics via replay

**Conclusion:** Both Option A and Option B use event sourcing, which is required for reliability.

### Recommendation

**Short-term:** Use Option A (Entity Workflow Pattern)
- Leverage existing infrastructure
- Document the continue-as-new requirement
- Users manage history size manually

**Medium-term:** Implement Option B (Auto-Reset Pattern like Azure)
- This is the **target architecture**
- Reuses workflow infrastructure + automatic history management
- Server needs: entity dispatcher, SchedulerState serialization, auto continue-as-new
- SDK needs: entity trait/macro, state serialization
- Users get simple API, framework handles complexity

### Implementation Priority for Option B

Based on DurableTask analysis, the key components needed:

1. **SchedulerState equivalent:**
   ```rust
   struct EntityState {
       state: Value,           // Serialized entity state
       queue: VecDeque<Op>,    // Pending operations
       locked_by: Option<Uuid>, // Lock holder
       message_sorter: MessageSorter, // For ordering
   }
   ```

2. **Entity Dispatcher:**
   - Fetch entity work items
   - Deserialize EntityState from workflow input
   - Execute operation batch
   - Auto continue-as-new with updated EntityState

3. **Lock Chain Support:**
   - Lock request with sorted entity list
   - Position tracking for chain traversal
   - Response routing back to orchestration

4. **Message Ordering (MessageSorter):**
   - Timestamp + predecessor for causal ordering
   - Deduplication window
   - Out-of-order message buffering

---

## Proposed API Design (Option B)

### 1. Entity Definition

```rust
use flovyn_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// Unique identifier for an entity instance
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId {
    /// Entity type (e.g., "account", "counter", "inventory")
    pub entity_type: String,
    /// Entity key (e.g., "account-123", "product-456")
    pub key: String,
}

impl EntityId {
    pub fn new(entity_type: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            entity_type: entity_type.into(),
            key: key.into(),
        }
    }
}

// Convenience: EntityId::from(("account", "123"))
impl<T: Into<String>, K: Into<String>> From<(T, K)> for EntityId {
    fn from((entity_type, key): (T, K)) -> Self {
        Self::new(entity_type, key)
    }
}
```

### 2. Entity Trait (Similar to WorkflowDefinition)

```rust
/// Define an entity with typed state and operations.
///
/// Entities are actor-like stateful objects that process operations serially.
/// The framework handles: serialization, history management, locking, message ordering.
#[async_trait]
pub trait EntityDefinition: Send + Sync + Default {
    /// The state type for this entity (must be serializable)
    type State: Serialize + DeserializeOwned + Default + Send + Sync;

    /// Entity type identifier (e.g., "account", "counter")
    fn entity_type(&self) -> &str;

    /// Handle an operation on this entity.
    ///
    /// Called by the framework for each operation in the batch.
    /// Operations are processed serially - no concurrent access to state.
    async fn handle(
        &self,
        ctx: &dyn EntityContext,
        state: &mut Self::State,
        operation: &str,
        input: Value,
    ) -> Result<Value>;

    /// Called when entity is first created (optional)
    fn on_create(&self, _state: &mut Self::State) {}
}
```

### 3. Entity Context

```rust
/// Context available during entity operation execution.
#[async_trait]
pub trait EntityContext: Send + Sync {
    /// Get the entity ID
    fn entity_id(&self) -> &EntityId;

    /// Get the current operation name
    fn operation(&self) -> &str;

    /// Check if this is a signal (fire-and-forget) vs call (request-response)
    fn is_signal(&self) -> bool;

    /// Get the caller's workflow/entity ID (if any)
    fn caller_id(&self) -> Option<&str>;

    // === Outbound Operations ===

    /// Signal another entity (fire-and-forget, no response)
    fn signal_entity(&self, target: EntityId, operation: &str, input: Value);

    /// Schedule a task (for side effects like sending emails)
    fn schedule_task(&self, task_type: &str, input: Value) -> TaskFutureRaw;

    // === Deterministic APIs ===

    /// Current time (deterministic for replay)
    fn current_time_millis(&self) -> i64;

    /// Generate deterministic UUID
    fn random_uuid(&self) -> Uuid;
}
```

### 4. Example: Counter Entity

```rust
use flovyn_sdk::entity::*;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CounterState {
    pub value: i64,
}

#[derive(Default)]
pub struct CounterEntity;

#[async_trait]
impl EntityDefinition for CounterEntity {
    type State = CounterState;

    fn entity_type(&self) -> &str {
        "counter"
    }

    async fn handle(
        &self,
        _ctx: &dyn EntityContext,
        state: &mut Self::State,
        operation: &str,
        input: Value,
    ) -> Result<Value> {
        match operation {
            "get" => {
                Ok(json!({ "value": state.value }))
            }
            "add" => {
                let amount: i64 = serde_json::from_value(input)?;
                state.value += amount;
                Ok(json!({ "value": state.value }))
            }
            "reset" => {
                state.value = 0;
                Ok(json!({ "value": state.value }))
            }
            _ => Err(FlovynError::InvalidOperation(operation.to_string())),
        }
    }
}
```

### 5. Example: Bank Account Entity

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AccountState {
    pub balance: i64,
    pub currency: String,
    pub is_locked: bool,
}

#[derive(Default)]
pub struct AccountEntity;

#[async_trait]
impl EntityDefinition for AccountEntity {
    type State = AccountState;

    fn entity_type(&self) -> &str {
        "account"
    }

    async fn handle(
        &self,
        ctx: &dyn EntityContext,
        state: &mut Self::State,
        operation: &str,
        input: Value,
    ) -> Result<Value> {
        match operation {
            "get_balance" => {
                Ok(json!({
                    "balance": state.balance,
                    "currency": state.currency,
                }))
            }
            "deposit" => {
                let amount: i64 = input.get("amount")
                    .and_then(|v| v.as_i64())
                    .ok_or(FlovynError::InvalidInput("amount required"))?;

                state.balance += amount;
                Ok(json!({
                    "success": true,
                    "balance": state.balance,
                }))
            }
            "withdraw" => {
                let amount: i64 = input.get("amount")
                    .and_then(|v| v.as_i64())
                    .ok_or(FlovynError::InvalidInput("amount required"))?;

                if state.balance < amount {
                    return Ok(json!({
                        "success": false,
                        "error": "insufficient_funds",
                        "balance": state.balance,
                    }));
                }

                state.balance -= amount;
                Ok(json!({
                    "success": true,
                    "balance": state.balance,
                }))
            }
            _ => Err(FlovynError::InvalidOperation(operation.to_string())),
        }
    }

    fn on_create(&self, state: &mut Self::State) {
        state.currency = "USD".to_string();
    }
}
```

### 6. Calling Entities from Workflows

```rust
/// Extension trait for WorkflowContext to call entities
pub trait WorkflowContextEntityExt: WorkflowContext {
    /// Call an entity operation and wait for response.
    ///
    /// This is a request-response pattern - the workflow suspends until
    /// the entity processes the operation and returns a result.
    fn call_entity(
        &self,
        entity_id: impl Into<EntityId>,
        operation: &str,
        input: Value,
    ) -> EntityCallFuture;

    /// Signal an entity (fire-and-forget).
    ///
    /// The workflow does NOT wait for the entity to process.
    /// Use this for notifications or when you don't need a response.
    fn signal_entity(
        &self,
        entity_id: impl Into<EntityId>,
        operation: &str,
        input: Value,
    );

    /// Acquire locks on multiple entities for a critical section.
    ///
    /// Locks are acquired in sorted order to prevent deadlocks.
    /// Returns a guard that releases locks when dropped.
    fn lock_entities(
        &self,
        entities: &[EntityId],
    ) -> EntityLockFuture;
}

/// Future for entity call results
pub struct EntityCallFuture { /* ... */ }

/// Future for acquiring entity locks
pub struct EntityLockFuture { /* ... */ }

/// Guard that releases locks when dropped
pub struct EntityLockGuard { /* ... */ }
```

### 7. Transfer Workflow Using Entities

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TransferInput {
    pub source_account: String,
    pub dest_account: String,
    pub amount: i64,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TransferOutput {
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct TransferWorkflow;

#[async_trait]
impl WorkflowDefinition for TransferWorkflow {
    type Input = TransferInput;
    type Output = TransferOutput;

    fn kind(&self) -> &str { "transfer" }

    async fn execute(
        &self,
        ctx: &dyn WorkflowContext,
        input: Self::Input,
    ) -> Result<Self::Output> {
        let source = EntityId::new("account", &input.source_account);
        let dest = EntityId::new("account", &input.dest_account);

        // Acquire locks on both accounts (sorted order for deadlock prevention)
        let _lock = ctx.lock_entities(&[source.clone(), dest.clone()]).await?;

        // Check source balance
        let balance_result = ctx.call_entity(
            source.clone(),
            "get_balance",
            json!({}),
        ).await?;

        let balance = balance_result.get("balance")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        if balance < input.amount {
            return Ok(TransferOutput {
                success: false,
                error: Some("Insufficient funds".to_string()),
            });
        }

        // Withdraw from source
        let withdraw_result = ctx.call_entity(
            source.clone(),
            "withdraw",
            json!({ "amount": input.amount }),
        ).await?;

        if !withdraw_result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Ok(TransferOutput {
                success: false,
                error: withdraw_result.get("error")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            });
        }

        // Deposit to destination
        let deposit_result = ctx.call_entity(
            dest.clone(),
            "deposit",
            json!({ "amount": input.amount }),
        ).await?;

        if !deposit_result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
            // Rollback: re-deposit to source
            ctx.call_entity(
                source,
                "deposit",
                json!({ "amount": input.amount }),
            ).await?;

            return Ok(TransferOutput {
                success: false,
                error: Some("Deposit failed, rolled back".to_string()),
            });
        }

        Ok(TransferOutput {
            success: true,
            error: None,
        })
        // Lock released here when _lock goes out of scope
    }
}
```

### 8. Client API: SignalWithStart

```rust
impl FlovynClient {
    /// Atomically start an entity (if not exists) and send a signal.
    ///
    /// This is critical for the entity pattern - ensures no race condition
    /// between checking if entity exists and sending the first operation.
    pub async fn signal_with_start_entity(
        &self,
        entity_id: EntityId,
        operation: &str,
        input: Value,
    ) -> Result<()> {
        // Server handles atomically:
        // 1. Check if entity workflow exists
        // 2. If not, create it with empty state
        // 3. Send signal (operation) to the entity
        todo!()
    }

    /// Call an entity operation and wait for response.
    ///
    /// If entity doesn't exist, it's created first.
    pub async fn call_entity(
        &self,
        entity_id: EntityId,
        operation: &str,
        input: Value,
    ) -> Result<Value> {
        todo!()
    }

    /// Get current state of an entity (if exists).
    pub async fn get_entity_state(
        &self,
        entity_id: EntityId,
    ) -> Result<Option<Value>> {
        todo!()
    }
}
```

### 9. Worker Registration

```rust
// Register entities with the worker (similar to workflows/tasks)
let client = FlovynClient::builder()
    .server_url("http://localhost:9090")
    .register_entity::<CounterEntity>()
    .register_entity::<AccountEntity>()
    .register_workflow::<TransferWorkflow>()
    .build()
    .await?;

client.start_workers().await?;
```

### 10. Alternative: Derive Macro API (More Ergonomic)

For even cleaner code, we could provide a derive macro:

```rust
use flovyn_sdk::entity::*;

#[derive(Entity, Default, Serialize, Deserialize)]
#[entity(type = "counter")]
pub struct Counter {
    value: i64,
}

#[entity_operations]
impl Counter {
    pub fn get(&self) -> i64 {
        self.value
    }

    pub fn add(&mut self, amount: i64) -> i64 {
        self.value += amount;
        self.value
    }

    pub fn reset(&mut self) {
        self.value = 0;
    }
}

// Usage from workflow:
let result = ctx.call_entity::<Counter>("counter-123", Counter::add(10)).await?;

// Or with EntityId:
let counter_id = EntityId::new("counter", "counter-123");
let result: i64 = ctx.call::<Counter>(counter_id, |c| c.add(10)).await?;
```

This macro would generate:
- `EntityDefinition` implementation
- Operation name to method dispatch
- Type-safe input/output serialization

### API Summary

| Component | API |
|-----------|-----|
| **Define Entity** | `impl EntityDefinition for MyEntity` |
| **Entity State** | `type State = MyState` (Serialize + Default) |
| **Handle Operations** | `async fn handle(&self, ctx, state, op, input)` |
| **Entity ID** | `EntityId::new("type", "key")` |
| **Call from Workflow** | `ctx.call_entity(id, "op", input).await` |
| **Signal from Workflow** | `ctx.signal_entity(id, "op", input)` |
| **Lock Entities** | `ctx.lock_entities(&[id1, id2]).await` |
| **Client: Signal+Start** | `client.signal_with_start_entity(id, "op", input)` |
| **Register Worker** | `.register_entity::<MyEntity>()`|

---

## Architecture: How Entities Fit with Workflows and Tasks

### Component Responsibilities

```
┌─────────────────────────────────────────────────────────────────┐
│                         External World                          │
│  (HTTP APIs, Databases, Email Services, Payment Gateways, etc.) │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ I/O (non-deterministic)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                           TASKS                                 │
│  - Execute external I/O                                         │
│  - Non-deterministic (can fail, retry)                          │
│  - Stateless                                                    │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ schedule_task()
                              │
┌─────────────────────────────────────────────────────────────────┐
│                         WORKFLOWS                               │
│  - Orchestrate tasks, entities, child workflows                 │
│  - Deterministic (replay-safe)                                  │
│  - Can wait for task results                                    │
│  - Can call entities and wait for response                      │
│  - Can acquire locks on entities                                │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ call_entity() / signal_entity()
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                          ENTITIES                               │
│  - Manage state (the "nouns" - accounts, carts, users)          │
│  - Process operations serially                                  │
│  - Primarily for state mutations                                │
│  - Can signal other entities (fire-and-forget)                  │
│  - Can schedule tasks (fire-and-forget)                         │
│  - Should NOT do complex orchestration                          │
└─────────────────────────────────────────────────────────────────┘
```

### Design Principle

| Component | Purpose | Can Wait for I/O? |
|-----------|---------|-------------------|
| **Task** | Execute external I/O | Yes (it IS the I/O) |
| **Workflow** | Orchestrate everything | Yes (via tasks) |
| **Entity** | Manage state | **No** (fire-and-forget only) |

### What Entities CAN and CANNOT Do

| Action | Allowed? | Why |
|--------|----------|-----|
| Mutate own state | ✅ Yes | Primary purpose |
| Signal another entity | ✅ Yes | Fire-and-forget |
| Schedule a task (fire-and-forget) | ✅ Yes | For side effects |
| **Wait for task result** | ❌ No | Blocks batch processing |
| **Call another entity and wait** | ❌ No | Can cause deadlocks |
| **Direct HTTP/DB calls** | ❌ No | Non-deterministic |
| **Complex orchestration** | ❌ No | Use workflows instead |

### Rule of Thumb

- **Entities** = State (nouns: Account, Cart, User, Inventory)
- **Tasks** = I/O (verbs: SendEmail, ChargePayment, CallAPI)
- **Workflows** = Orchestration (processes: Checkout, Transfer, Onboarding)

### Example: E-commerce Checkout

```rust
// ============================================================
// ENTITIES: State management (the "nouns")
// ============================================================

#[derive(Default)]
pub struct CartEntity;

impl EntityDefinition for CartEntity {
    type State = CartState;
    fn entity_type(&self) -> &str { "cart" }

    async fn handle(&self, ctx: &dyn EntityContext, state: &mut Self::State,
                    operation: &str, input: Value) -> Result<Value> {
        match operation {
            "add_item" => {
                let item: CartItem = serde_json::from_value(input)?;
                state.items.push(item);
                Ok(json!({ "item_count": state.items.len() }))
            }
            "get" => Ok(serde_json::to_value(&state)?),
            "clear" => {
                state.items.clear();
                Ok(json!({ "cleared": true }))
            }
            _ => Err(FlovynError::InvalidOperation(operation.into()))
        }
    }
}

#[derive(Default)]
pub struct InventoryEntity;

impl EntityDefinition for InventoryEntity {
    type State = InventoryState;
    fn entity_type(&self) -> &str { "inventory" }

    async fn handle(&self, ctx: &dyn EntityContext, state: &mut Self::State,
                    operation: &str, input: Value) -> Result<Value> {
        match operation {
            "reserve" => {
                let qty: i32 = input.get("quantity")
                    .and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if state.available >= qty {
                    state.available -= qty;
                    state.reserved += qty;
                    Ok(json!({ "success": true, "available": state.available }))
                } else {
                    Ok(json!({ "success": false, "error": "insufficient_stock" }))
                }
            }
            "release" => {
                let qty: i32 = input.get("quantity")
                    .and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                state.reserved -= qty;
                state.available += qty;
                Ok(json!({ "success": true }))
            }
            "commit" => {
                let qty: i32 = input.get("quantity")
                    .and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                state.reserved -= qty;
                // Item is now sold, don't add back to available
                Ok(json!({ "success": true }))
            }
            _ => Err(FlovynError::InvalidOperation(operation.into()))
        }
    }
}

// ============================================================
// TASKS: External I/O (the "verbs" that touch outside world)
// ============================================================

#[derive(Default)]
pub struct ChargePaymentTask;

#[async_trait]
impl TaskDefinition for ChargePaymentTask {
    type Input = PaymentRequest;
    type Output = PaymentResult;

    fn kind(&self) -> &str { "charge-payment" }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        // Actually call Stripe API (non-deterministic I/O)
        let client = stripe::Client::new(&std::env::var("STRIPE_KEY")?);
        let charge = client.charges().create(&input).await?;
        Ok(PaymentResult {
            success: true,
            transaction_id: charge.id
        })
    }
}

#[derive(Default)]
pub struct SendEmailTask;

#[async_trait]
impl TaskDefinition for SendEmailTask {
    type Input = EmailRequest;
    type Output = EmailResult;

    fn kind(&self) -> &str { "send-email" }

    async fn execute(&self, input: Self::Input, ctx: &dyn TaskContext) -> Result<Self::Output> {
        // Actually send email via SendGrid/SES (non-deterministic I/O)
        let client = sendgrid::Client::new(&std::env::var("SENDGRID_KEY")?);
        client.send(&input).await?;
        Ok(EmailResult { sent: true })
    }
}

// ============================================================
// WORKFLOW: Orchestration (coordinates entities + tasks)
// ============================================================

#[derive(Default)]
pub struct CheckoutWorkflow;

#[async_trait]
impl WorkflowDefinition for CheckoutWorkflow {
    type Input = CheckoutInput;
    type Output = CheckoutResult;

    fn kind(&self) -> &str { "checkout" }

    async fn execute(&self, ctx: &dyn WorkflowContext, input: Self::Input) -> Result<Self::Output> {
        let cart_id = EntityId::new("cart", &input.cart_id);

        // 1. Get cart contents (ENTITY CALL)
        let cart = ctx.call_entity(&cart_id, "get", json!({})).await?;
        let items: Vec<CartItem> = serde_json::from_value(
            cart.get("items").cloned().unwrap_or_default()
        )?;

        if items.is_empty() {
            return Ok(CheckoutResult::empty_cart());
        }

        // 2. Reserve inventory for each item (ENTITY CALLS)
        let mut reserved_items = Vec::new();
        for item in &items {
            let inventory_id = EntityId::new("inventory", &item.product_id);
            let result = ctx.call_entity(
                &inventory_id,
                "reserve",
                json!({ "quantity": item.quantity }),
            ).await?;

            if !result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                // Rollback: release all reserved items
                for reserved in &reserved_items {
                    let inv_id = EntityId::new("inventory", &reserved.product_id);
                    ctx.call_entity(
                        &inv_id,
                        "release",
                        json!({ "quantity": reserved.quantity })
                    ).await?;
                }
                return Ok(CheckoutResult::out_of_stock(&item.product_id));
            }
            reserved_items.push(item.clone());
        }

        // 3. Charge payment (TASK - external I/O)
        let payment_result = ctx.schedule::<ChargePaymentTask>(PaymentRequest {
            amount: calculate_total(&items),
            currency: "USD".into(),
            customer_id: input.user_id.clone(),
        }).await?;

        if !payment_result.success {
            // Rollback: release all inventory
            for item in &items {
                let inv_id = EntityId::new("inventory", &item.product_id);
                ctx.call_entity(
                    &inv_id,
                    "release",
                    json!({ "quantity": item.quantity })
                ).await?;
            }
            return Ok(CheckoutResult::payment_failed());
        }

        // 4. Commit inventory (ENTITY CALLS)
        for item in &items {
            let inv_id = EntityId::new("inventory", &item.product_id);
            ctx.call_entity(
                &inv_id,
                "commit",
                json!({ "quantity": item.quantity })
            ).await?;
        }

        // 5. Clear cart (ENTITY CALL)
        ctx.call_entity(&cart_id, "clear", json!({})).await?;

        // 6. Send confirmation email (TASK - external I/O)
        ctx.schedule::<SendEmailTask>(EmailRequest {
            to: input.email.clone(),
            subject: "Order Confirmed".into(),
            body: format!("Your order {} has been placed!", payment_result.transaction_id),
        }).await?;

        Ok(CheckoutResult {
            success: true,
            order_id: payment_result.transaction_id,
        })
    }
}
```

### When Entity Needs to Trigger External Work

If an entity operation needs to trigger external work, use **fire-and-forget**:

```rust
async fn handle(&self, ctx: &dyn EntityContext, state: &mut Self::State,
                operation: &str, input: Value) -> Result<Value> {
    match operation {
        "place_order" => {
            // Update state
            state.status = OrderStatus::Placed;
            state.placed_at = ctx.current_time_millis();

            // Fire-and-forget: signal another entity
            // We don't wait for result - it happens asynchronously
            ctx.signal_entity(
                EntityId::new("notification-queue", "default"),
                "enqueue",
                json!({
                    "type": "order_placed",
                    "order_id": state.order_id
                }),
            );

            Ok(json!({ "status": "placed" }))
        }
        // ...
    }
}
```

**Better pattern:** Let the workflow handle side effects:

```rust
// In workflow - cleaner separation of concerns:
ctx.call_entity(&order_id, "place_order", json!({...})).await?;
ctx.schedule::<SendNotificationTask>(notification).await?;  // Workflow waits
```

### Component Interaction Summary

| Component | Can Be Called From | Can Call |
|-----------|-------------------|----------|
| **Entity** | Workflow, Client, Entity (signal) | Entity (signal), Task (fire-and-forget) |
| **Task** | Workflow, Entity (fire-and-forget) | External services (HTTP, DB, etc.) |
| **Workflow** | Client, Workflow (child) | Entity, Task, Workflow (child) |

### Tasks CANNOT Call Entities (Confirmed)

**Verified from DurableTask source code and Azure documentation:**

| Component | Can Call Entities? | Evidence |
|-----------|-------------------|----------|
| **Workflow (Orchestration)** | ✅ Yes | `IDurableOrchestrationContext.CallEntityAsync()` |
| **Client** | ✅ Yes | Durable client binding |
| **Task (Activity)** | ❌ **No** | `IDurableActivityContext` has no entity methods |
| **Entity** | ✅ Signal only | Fire-and-forget via `EntityContext` |

**From DurableTask source** (`TaskContext.cs`):
```csharp
public class TaskContext
{
    public OrchestrationInstance OrchestrationInstance { get; }
    public string Name { get; }
    public string? Version { get; }
    public int TaskId { get; }
    // NO entity methods - tasks cannot call entities!
}
```

**Why this design makes sense:**
1. **Tasks are stateless I/O workers** - They execute external calls and return results
2. **Entities need deterministic replay** - Task execution is NOT replayed, so entity calls from tasks would break consistency
3. **Workflows are the coordinator** - They manage flow and ensure determinism

**Correct pattern:**
```rust
// ❌ WRONG: Task trying to call entity
impl TaskDefinition for MyTask {
    async fn execute(&self, input, ctx: &dyn TaskContext) -> Result<Output> {
        // ctx.call_entity(...) - NOT AVAILABLE AND SHOULD NOT BE!
    }
}

// ✅ CORRECT: Workflow coordinates entity and task
impl WorkflowDefinition for MyWorkflow {
    async fn execute(&self, ctx: &dyn WorkflowContext, input) -> Result<Output> {
        // 1. Get data from entity
        let data = ctx.call_entity(&entity_id, "get", json!({})).await?;

        // 2. Pass to task for external I/O
        let result = ctx.schedule::<MyTask>(TaskInput { data }).await?;

        // 3. Update entity with result
        ctx.call_entity(&entity_id, "update", json!({ "result": result })).await?;

        Ok(output)
    }
}
```

**References:**
- [Function types in Azure Durable Functions](https://learn.microsoft.com/en-us/azure/azure-functions/durable/durable-functions-types-features-overview)
- [IDurableOrchestrationContext.CallEntityAsync](https://learn.microsoft.com/en-us/dotnet/api/microsoft.azure.webjobs.extensions.durabletask.idurableorchestrationcontext.callentityasync)

### Why This Separation Matters

1. **Entities stay simple** - Just state + mutations, no complex orchestration
2. **Tasks are isolated** - External I/O failures don't corrupt entity state
3. **Workflows are the brain** - Handle retries, compensation, complex logic
4. **Testability** - Each component can be tested in isolation
5. **Reliability** - Event sourcing works because entities are deterministic

---

## Recommendations

### Short-term (No SDK changes)
1. **Document Entity Workflow Pattern** - Add examples showing how to model entities as workflows
2. **Document Mutex Pattern** - Add examples for explicit locking scenarios
3. **Add helper functions** - Utility functions for common patterns

### Medium-term (Server + SDK enhancements)
1. **Add `SignalWithStart`** (HIGH PRIORITY) - Atomic start-or-signal operation
   - Critical for Entity Pattern - without it, there are race conditions
   - Server needs: atomic workflow lookup + creation + signal append
   - SDK needs: `client.signal_with_start_workflow()` API
2. **Add `select` combinator** - Wait for first of multiple futures (needed for timeouts in entity loops)
3. **Add entity helpers** - Higher-level abstractions for entity pattern

### Long-term (Server + SDK)
1. **Continue-as-new** - Reset event history for long-running entity workflows
   - Prevents hitting event history limits (Temporal has 50K event limit)
   - Entity workflows may run indefinitely, processing thousands of operations
2. **Native `context.lock()`** - Server-managed distributed locks
   - Cleaner API than Mutex Workflow pattern
   - Automatic deadlock prevention
3. **Durable Entities** - First-class entity support with automatic lifecycle management
   - Azure-like `call_entity()` API
   - Server-managed entity state

---

## Key Learnings from DurableTask Source Code

Analysis of the Azure DurableTask Framework source code revealed several important implementation details:

### 1. Entities ARE Event-Sourced (Internally)

Contrary to initial assumptions, entities use the same event-sourcing infrastructure as orchestrations. The key difference is **automatic history management**.

### 2. The Auto-Reset Trick

After each batch of operations, entities automatically:
1. Serialize their state to `SchedulerState` JSON
2. Complete with `ContinueAsNewEvent`
3. Start fresh execution with the serialized state as input

This keeps history minimal while reusing the workflow infrastructure.

### 3. Lock Chains (Not Central Lock Manager)

Distributed locking is implemented via lock chains:
- Orchestration sends lock request with sorted entity list
- Each entity forwards to next in chain (incrementing position)
- Last entity responds back to orchestration
- No central lock manager needed

### 4. Message Ordering via MessageSorter

Entities handle out-of-order message delivery with:
- Timestamp + predecessor tracking
- Configurable reorder window
- Automatic deduplication
- Buffering for causal ordering

### 5. Extended Sessions for Efficiency

Entities can process multiple batches without unloading:
- State cached in memory between batches
- Async wait for next messages
- Reduces serialization overhead

### 6. Implications for Flovyn

The DurableTask approach (Option B) is attractive because:
- Reuses existing workflow infrastructure
- No new storage primitives needed
- Proven at scale in Azure production
- Clean separation: user code vs framework concerns

---

## References

### Azure Durable Functions
- [Durable Entities Documentation](https://learn.microsoft.com/en-us/azure/azure-functions/durable/durable-functions-entities)
- [Azure Storage Provider Architecture](https://learn.microsoft.com/en-us/azure/azure-functions/durable/durable-functions-azure-storage-provider)
- [DurableTask Framework Source Code](https://github.com/Azure/durabletask) - analyzed locally at `~/Developer/manhha/tmp/durabletask`
  - `src/DurableTask.Core/Entities/` - Entity implementation
  - `src/DurableTask.Core/Entities/SchedulerState.cs` - Entity state model
  - `src/DurableTask.Core/Entities/MessageSorter.cs` - Message ordering
  - `src/DurableTask.Core/History/` - Event types

### Temporal
- [Mutex Sample (Go)](https://pkg.go.dev/github.com/temporalio/samples-go/mutex)
- [Entity Pattern (TypeScript)](https://docs.temporal.io/develop/typescript/entity-pattern)
- [Entity Lifecycle Demo (Go)](https://github.com/temporal-sa/temporal-entity-lifecycle-go)
- [Workflows as Actors Blog](https://temporal.io/blog/workflows-as-actors-is-it-really-possible)
- [Managing Long-Running Workflows](https://temporal.io/blog/very-long-running-workflows)
