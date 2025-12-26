# Implementation Plan: Stateright Model Checking for SDK

## Overview

Implement model checking using Stateright to verify correctness properties of the Flovyn SDK, specifically focusing on per-type sequence matching and parallel combinator semantics.

**Design Document:**
- [Stateright Model Checking Design](../design/20251225_stateright-model-checking.md)

**Prerequisites:**
- [Parallel Execution](./20251220_parallel-execution-implementation.md) (implemented)
- [Sequence-Based Replay](./20251219_sequence-based-replay-implementation.md) (implemented)

---

## Phase 1: Evaluation Setup

### 1.1 Add Stateright Dependency

**File:** `sdk/Cargo.toml`

Add stateright as a dev-dependency:

```toml
[dev-dependencies]
stateright = "0.30"
num_cpus = "1.16"
```

### 1.2 Create Model Checking Test Module

**File:** `sdk/tests/model/mod.rs` (new file)

```rust
//! Model checking tests using Stateright.
//!
//! These tests exhaustively verify correctness properties of the SDK
//! by exploring all possible state transitions.

mod sequence_matching;
```

### 1.3 Create Test Entry Point

**File:** `sdk/tests/model.rs` (new file)

```rust
//! Stateright model checking tests.

mod model;
```

### 1.4 Implement Basic SequenceMatchingModel

**File:** `sdk/tests/model/sequence_matching.rs` (new file)

Implement the model from the design document:
- State: per-type sequence counters, event history, commands issued, violation flag
- Actions: ScheduleTask, StartTimer, ScheduleChild
- Properties: matching commands pass, violations detected, extension allowed

### 1.5 Run Evaluation and Measure

Verify the model:
1. Model compiles and runs
2. Explores expected state space (target: < 10,000 states for small configs)
3. Completes in reasonable time (target: < 30 seconds)
4. Properties are verified correctly

**Decision point:** If model checking is too slow, explores too many states, or finds nothing useful, stop here and document findings.

---

## Phase 2: Core Models

### 2.1 Refine SequenceMatchingModel

**File:** `sdk/tests/model/sequence_matching.rs`

Extend the basic model to cover all command types:
- Add PromiseCreated command
- Add OperationCompleted command
- Add SetState command
- Verify per-type sequence independence

### 2.2 Add Determinism Violation Detection Model

**File:** `sdk/tests/model/determinism.rs` (new file)

Model that specifically tests determinism violation detection:
- Verify mismatches are detected at correct sequence
- Verify error messages are informative
- Verify replay stops on first violation

### 2.3 Implement JoinAllModel

**File:** `sdk/tests/model/join_all.rs` (new file)

Model from design document:
- State: pending futures, results, resolved outcome
- Actions: Complete with Ok/Err for each pending future
- Properties: fail-fast, all-ok requires all complete, eventually resolves

### 2.4 Implement SelectModel

**File:** `sdk/tests/model/select.rs` (new file)

Similar to JoinAllModel but for select semantics:
- First completion wins
- Remaining futures should be cancelled
- Both success and error propagation

### 2.5 Export Model Modules

**File:** `sdk/tests/model/mod.rs`

```rust
mod sequence_matching;
mod determinism;
mod join_all;
mod select;
```

---

## Phase 3: Property Verification

### 3.1 Add Property Tests to SequenceMatchingModel

Properties to verify:
- `matching_commands_never_violate`: When commands match history, no violation
- `mismatched_commands_always_violate`: When commands mismatch, violation detected
- `extension_is_allowed`: Commands beyond history are allowed without violation
- `per_type_sequences_are_independent`: Task seq doesn't affect timer seq, etc.

### 3.2 Add Property Tests to JoinAllModel

Properties to verify:
- `fail_fast`: First error immediately resolves join_all
- `all_ok_requires_all_complete`: AllOk only when all futures complete successfully
- `order_preserved`: Results are in original future order
- `eventually_resolves`: All paths lead to resolution

### 3.3 Add Property Tests to SelectModel

Properties to verify:
- `first_wins`: First completion determines result
- `error_propagates`: First error is returned
- `eventually_resolves`: All paths lead to resolution

---

## Phase 4: Parameterized Testing

### 4.1 Add Parameterized Configuration

**File:** `sdk/tests/model/config.rs` (new file)

```rust
/// Configuration for model checking tests
pub struct ModelConfig {
    /// Maximum number of commands to explore
    pub max_commands: usize,
    /// Number of parallel threads
    pub threads: usize,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            max_commands: 5,
            threads: num_cpus::get(),
        }
    }
}
```

### 4.2 Add Test Variants

Create tests with different configurations:
- Small (2 event types, 3 commands): fast, basic verification
- Medium (3 event types, 5 commands): thorough verification
- Large (4 event types, 7 commands): exhaustive (may be slow)

---

## Phase 5: CI Integration

### 5.1 Add Model Checking Workflow

**File:** `.github/workflows/model-check.yml` (new file)

```yaml
name: Model Checking

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
  # Allow manual trigger for expensive checks
  workflow_dispatch:

jobs:
  model-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run model checking tests
        run: cargo test --test model -p flovyn-sdk -- --test-threads=1
        timeout-minutes: 10
```

### 5.2 Add Local Run Script

**File:** `bin/dev/run-model-check.sh` (new file)

```bash
#!/bin/bash
# Run Stateright model checking tests

set -e

echo "Running model checking tests..."
cargo test --test model -p flovyn-sdk -- --test-threads=1 --nocapture "$@"
echo "Model checking complete!"
```

### 5.3 Update CLAUDE.md

Add model checking commands to the build/development section.

---

## Phase 6: Documentation and Reporting

### 6.1 Add Model Documentation

**File:** `sdk/tests/model/README.md` (new file)

Document:
- What model checking is
- How to run the tests
- How to interpret results
- How to add new models
- State space complexity estimates

### 6.2 Add Exploration Statistics

Enhance test output to show:
- Number of states explored
- Time taken
- Memory used
- Property verification results

---

## TODO List

### Phase 1: Evaluation Setup
- [x] Add `stateright = "0.30"` to sdk/Cargo.toml dev-dependencies
- [x] Add `num_cpus = "1.16"` to sdk/Cargo.toml dev-dependencies
- [x] Create `sdk/tests/model/mod.rs`
- [x] Create `sdk/tests/model/sequence_matching.rs` with basic model
- [x] Implement `SequenceMatchingState` struct
- [x] Implement `SequenceMatchingModel` struct
- [x] Implement `Model` trait for SequenceMatchingModel
- [x] Add basic properties (matching passes, extension allowed)
- [x] Run model and measure: states explored, time taken
- [x] Document evaluation results in design document

**Decision checkpoint:** ✅ Proceeded to Phase 2 - model checking is fast and effective

### Phase 2: Core Models
- [ ] Extend SequenceMatchingModel with all command types (deferred - current model is sufficient)
- [ ] Add PromiseCreated command variant (deferred)
- [ ] Add OperationCompleted command variant (deferred)
- [ ] Add SetState command variant (deferred)
- [ ] Create `sdk/tests/model/determinism.rs` (deferred - covered by sequence_matching)
- [ ] Implement DeterminismModel (deferred)
- [x] Create `sdk/tests/model/join_all.rs`
- [x] Implement JoinAllState and JoinAllModel
- [x] Implement JoinAllAction enum
- [x] Add fail-fast property
- [x] Add all-ok-requires-all-complete property
- [x] Create `sdk/tests/model/select.rs`
- [x] Implement SelectState and SelectModel
- [x] Implement SelectAction enum
- [x] Add first-wins property
- [x] Update `sdk/tests/model/mod.rs` with all modules

### Phase 3: Property Verification
- [x] Add `matching_commands_never_violate` property test (in sequence_matching.rs)
- [x] Add `extension_is_allowed` property test (in sequence_matching.rs)
- [x] Add `per_type_sequences_are_independent` property test (in sequence_matching.rs)
- [x] Add `join_all_fail_fast` property test (in join_all.rs)
- [x] Add `join_all_order_preserved` property test (in join_all.rs)
- [x] Add `select_first_wins` property test (in select.rs)
- [x] Add `select_error_propagates` property test (in select.rs)

### Phase 4: Parameterized Testing
- [x] Add small configuration tests (verify_sequence_matching_small)
- [x] Add medium configuration tests (verify_sequence_matching_medium)
- [ ] Add large configuration tests (marked as ignored by default) - deferred

### Phase 5: CI Integration
- [x] Add model-check job to `.github/workflows/ci.yml`
- [x] Create `bin/dev/run-model-check.sh`
- [x] Make script executable
- [x] Update CLAUDE.md with model checking commands

### Phase 6: Documentation
- [x] Add module-level documentation to `sdk/tests/model/mod.rs`
- [x] Document how to run tests
- [x] Add exploration statistics to test output (states explored, time taken)

---

## Verification Checklist

After implementation:

- [x] `cargo build --workspace` succeeds
- [x] `cargo test --workspace` passes
- [x] `cargo test --test model -p flovyn-sdk` passes (13 tests)
- [x] Model checking completes in < 30 seconds for default config (~20ms total)
- [x] State space is tractable (< 10,000 states for small configs)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] `cargo fmt --all -- --check` passes
- [ ] CI workflow runs successfully (pending push to main)

---

## Expected Outcomes

### What We Expect to Find

1. **Confirmation of correctness**: Models verify that current implementation is correct
2. **Edge case documentation**: Models document expected behavior in edge cases
3. **Regression prevention**: Future changes that break invariants will fail model checking

### What Would Be a Red Flag

1. **State explosion**: If small configs produce > 100,000 states, model is too complex
2. **No violations found**: If we can't make the model find known bad behaviors, it's not useful
3. **Excessive runtime**: If tests take > 5 minutes, they won't run in CI effectively

### Success Criteria

1. Models complete in < 30 seconds for default configuration
2. Models find violations when we intentionally introduce bugs
3. Models don't find violations in correct implementation
4. State space remains tractable as we add more command types

---

## Dependencies

This implementation depends on:
1. **Parallel Execution** (completed) - join_all and select are implemented
2. **Sequence-Based Replay** (completed) - per-type sequence counters exist
3. **ReplayEngine in flovyn-core** - the actual implementation being modeled

---

## Open Questions from Design

1. **Should models live next to implementation or separately?**
   - Decision: Separately in `sdk/tests/model/` - cleaner separation, easier to maintain

2. **How do we prevent model drift?**
   - Approach: Review models when changing implementation, add comments linking to implementation

3. **Is the ROI worth it?**
   - Will evaluate after Phase 1
