# SDK and FFI Module Consolidation Analysis

## Overview

This document analyzes the current structure of `flovyn-sdk`, `flovyn-ffi`, and `flovyn-core` to identify opportunities for extracting common logic into the shared core, reducing duplication and maintenance burden.

**Context**: The 3-tier architecture was established in [Core SDK in Rust for Multiple Languages](../design/core-sdk-in-rust-for-multiple-language.md) to support multi-language SDKs (Kotlin, Python, Swift) via uniffi bindings.

## Current Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  User Code (Rust)          │   User Code (Kotlin/Swift/Python)     │
└──────────────┬──────────────┴──────────────────┬────────────────────┘
               │                                  │
               ▼                                  ▼
┌──────────────────────────┐       ┌───────────────────────────────────┐
│     flovyn-sdk           │       │          flovyn-ffi                │
│     (20,187 LOC)         │       │          (3,214 LOC)               │
│                          │       │                                    │
│  - WorkflowDefinition    │       │  - CoreWorker object               │
│  - WorkflowContext       │       │  - CoreClient object               │
│  - TaskDefinition        │       │  - FfiWorkflowContext              │
│  - TaskContext           │       │  - Activation protocol             │
│  - Worker executors      │       │  - FfiWorkflowCommand              │
│  - FlovynClient builder  │       │  - uniffi bindings                 │
│  - Testing utilities     │       │                                    │
└──────────────┬───────────┘       └──────────────┬────────────────────┘
               │                                   │
               └───────────────┬───────────────────┘
                               ▼
               ┌───────────────────────────────────┐
               │          flovyn-core              │
               │          (~5,000 LOC)             │
               │                                   │
               │  - gRPC clients                   │
               │  - Workflow commands, events      │
               │  - Determinism validation         │
               │  - Task metadata, streaming       │
               │  - Worker lifecycle types         │
               │  - Generated protobuf code        │
               └───────────────────────────────────┘
```

## Crate Sizes and Responsibilities

| Crate | Files | LOC | Primary Responsibility |
|-------|-------|-----|------------------------|
| flovyn-sdk | 46 | 20,187 | Rust-idiomatic SDK with traits, futures, worker executors |
| flovyn-ffi | 9 | 3,214 | uniffi bindings with activation-based protocol |
| flovyn-core | 26 | ~5,000 | Language-agnostic types and gRPC clients |

## Current Overlap Analysis

### 1. Workflow Context Implementation (HIGH DUPLICATION)

Both modules implement replay-aware workflow context logic:

**SDK: `sdk/src/workflow/context_impl.rs`** (3,237 LOC)
```rust
// WorkflowContextImpl handles:
// - Event lookup for replay (O(1) caches for tasks, timers, operations)
// - Deterministic random via SeededRandom
// - Command recording via CommandRecorder
// - State management via SetState/ClearState commands
// - Task scheduling with futures
// - Timer management
// - Promise handling
// - Child workflow coordination
```

**FFI: `ffi/src/context.rs`** (1,085 LOC)
```rust
// FfiWorkflowContext handles:
// - Event lookup for replay (same O(1) caches)
// - Deterministic random via SeededRandom
// - Command recording (returns Vec<FfiWorkflowCommand>)
// - State management via SetState/ClearState
// - Task scheduling (returns FfiTaskResult enum)
// - Timer management (returns FfiTimerResult enum)
// - Promise handling (returns FfiPromiseResult enum)
// - Child workflow coordination (returns FfiChildWorkflowResult enum)
```

**Key Differences:**
- SDK uses Rust futures and async/await; FFI returns status enums (Pending/Completed/Failed)
- SDK has richer type system with generics; FFI uses JSON bytes at boundary
- SDK integrates with worker executor; FFI is standalone context object

**Shared Logic:**
- Event caching and lookup (~200 LOC)
- Deterministic random generation (~50 LOC)
- Sequence number management (~30 LOC)
- State key normalization and management (~100 LOC)
- Operation idempotency key generation (~20 LOC)

### 2. Error Handling (MEDIUM DUPLICATION)

**SDK: `sdk/src/error.rs`**
- `FlovynError` enum with 18+ variants
- Implements `From<CoreError>` conversion

**FFI: `ffi/src/error.rs`** (151 LOC)
- `FfiError` enum with 8 variants (subset for FFI boundary)
- Implements `From<CoreError>` and `From<DeterminismViolationError>`

**Shared Logic:**
- CoreError conversion patterns are nearly identical

### 3. Command Generation (MEDIUM DUPLICATION)

**SDK:** Uses `WorkflowCommand` from core directly

**FFI:** Defines `FfiWorkflowCommand` (488 LOC) with:
- 18 command variants
- `to_proto_command()` conversion to gRPC types

**Observation:** FFI essentially re-implements command types for uniffi compatibility, then converts to proto. This is unavoidable due to uniffi's type restrictions, but the conversion logic could be shared.

### 4. Event Type Handling (LOW DUPLICATION)

**SDK:** Re-exports `EventType` from core

**FFI:** Defines `FfiEventType` (249 LOC) with bidirectional conversions

**Observation:** FFI needs uniffi-compatible enum, which requires re-definition. Conversion logic is straightforward and low maintenance.

## Consolidation Opportunities

### Opportunity 1: Extract Replay Engine to Core (HIGH VALUE)

**What:** Extract the core replay logic (event lookup, caching, sequence management) into `flovyn-core` as a reusable `ReplayEngine`.

**Current State:**
```
SDK WorkflowContextImpl ──┬── Event caching logic
                          ├── Sequence management
                          └── Deterministic state lookup

FFI FfiWorkflowContext ───┬── Event caching logic (duplicate)
                          ├── Sequence management (duplicate)
                          └── Deterministic state lookup (duplicate)
```

**Proposed State:**
```
                    flovyn-core
                         │
              ┌──────────┴──────────┐
              │    ReplayEngine     │
              │  - Event lookup     │
              │  - Caching          │
              │  - Sequence mgmt    │
              └──────────┬──────────┘
                         │
         ┌───────────────┴───────────────┐
         │                               │
         ▼                               ▼
SDK WorkflowContextImpl          FFI FfiWorkflowContext
  (uses ReplayEngine)              (uses ReplayEngine)
  - Async futures                  - Status enums
  - Rust traits                    - JSON boundary
```

**Benefits:**
- ~300-400 LOC deduplication
- Single source of truth for replay logic
- Easier to test and validate determinism
- Bug fixes apply to both SDK and FFI

**Complexity:** Medium - requires careful API design to support both async (SDK) and sync (FFI) patterns.

### Opportunity 2: Unified State Management (MEDIUM VALUE)

**What:** Extract state management logic (get/set/clear, key normalization) into core.

**Current State:**
- Both SDK and FFI implement state caching, key validation, and command generation
- Logic is similar but not identical

**Proposed:**
```rust
// In flovyn-core
pub struct StateManager {
    state_cache: HashMap<String, Value>,
}

impl StateManager {
    pub fn get(&self, key: &str) -> Option<&Value>;
    pub fn set(&mut self, key: &str, value: Value) -> WorkflowCommand;
    pub fn clear(&mut self, key: &str) -> Option<WorkflowCommand>;
    pub fn clear_all(&mut self) -> Vec<WorkflowCommand>;
    pub fn keys(&self) -> impl Iterator<Item = &str>;
    pub fn apply_initial_state(&mut self, events: &[ReplayEvent]);
}
```

**Benefits:**
- ~100 LOC deduplication
- Consistent state behavior across SDKs

### Opportunity 3: Command Helpers (LOW VALUE)

**What:** Add helper functions in core for building common command patterns.

**Current State:**
- FFI's `FfiWorkflowCommand::to_proto_command()` builds proto commands manually
- SDK uses core's `WorkflowCommand` which already has proto conversion

**Observation:** FFI needs its own enum for uniffi, so this duplication is structural. However, we could add shared validation logic.

## Recommendations

### Phase 1: Extract ReplayEngine (High Impact)

1. Create `core/src/workflow/replay_engine.rs`:
   ```rust
   pub struct ReplayEngine {
       events: Vec<ReplayEvent>,
       task_cache: HashMap<String, ReplayEvent>,
       timer_cache: HashMap<String, ReplayEvent>,
       operation_cache: HashMap<String, ReplayEvent>,
       state_cache: HashMap<String, Value>,
       sequence_number: u32,
   }

   impl ReplayEngine {
       pub fn new(events: Vec<ReplayEvent>) -> Self;
       pub fn next_sequence(&mut self) -> u32;
       pub fn lookup_task(&self, task_id: &str) -> Option<&ReplayEvent>;
       pub fn lookup_timer(&self, timer_id: &str) -> Option<&ReplayEvent>;
       pub fn lookup_operation(&self, op_name: &str) -> Option<&ReplayEvent>;
       pub fn get_state(&self, key: &str) -> Option<&Value>;
       pub fn set_state(&mut self, key: &str, value: Value);
       pub fn clear_state(&mut self, key: &str);
   }
   ```

2. Update SDK `WorkflowContextImpl` to use `ReplayEngine`
3. Update FFI `FfiWorkflowContext` to use `ReplayEngine`

**Estimated Impact:** ~400 LOC reduction, improved consistency

### Phase 2: Extract SeededRandom (Low Effort)

The `SeededRandom` / `DeterministicRandom` implementation should be in core (it may already be).

Verify:
- Core has `SeededRandom` trait and implementation
- Both SDK and FFI use the same implementation

### Phase 3: Document Intentional Differences

Some duplication is intentional due to architectural differences:
- FFI uses status enums (Pending/Completed) for language interop
- SDK uses Rust futures for ergonomic async
- FFI uses JSON bytes; SDK uses generic types

Document these as intentional design decisions, not technical debt.

## Non-Goals

The following are **not** consolidation candidates:

1. **Worker executors** - SDK has rich async worker with lifecycle hooks; FFI exposes minimal poll/complete interface. These serve fundamentally different purposes.

2. **Client builders** - SDK's `FlovynClientBuilder` is Rust-idiomatic with trait-based registration; FFI's config is simple records for uniffi.

3. **Error enums** - FFI's simplified error set is intentional for language interop. Core errors are more granular for Rust consumers.

4. **Type definitions** - FFI must re-define types for uniffi compatibility. This is structural, not technical debt.

## Summary

| Opportunity | LOC Saved | Effort | Priority |
|-------------|-----------|--------|----------|
| ReplayEngine extraction | ~400 | Medium | High |
| State management | ~100 | Low | Medium |
| Command helpers | ~50 | Low | Low |
| SeededRandom unification | ~50 | Low | Medium |

**Total potential:** ~600 LOC reduction with improved maintainability.

The current architecture is sound - the 3-tier design (core/ffi/sdk) was intentional. The main opportunity is extracting the replay engine logic, which is genuinely duplicated between SDK and FFI contexts.

## References

- [Core SDK in Rust for Multiple Languages](../design/core-sdk-in-rust-for-multiple-language.md)
- [Phase 3: Kotlin SDK](../plans/phase3-kotlin-sdk.md)
- [Multi-Language SDK Strategy](multi-language-sdk-strategy.md)
