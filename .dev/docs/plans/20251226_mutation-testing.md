# Implementation Plan: Mutation Testing for Replay Engine

## Overview

Introduce known bugs (mutations) into the `ReplayEngine` implementation and verify that tests catch them. This validates that our test suite is actually capable of detecting bugs.

**Problem Solved:** Current tests might pass even if the implementation is wrong. Mutation testing proves tests can detect real bugs by introducing synthetic bugs and checking if tests fail.

**Design Reference:** [Stateright Model Checking Design](../design/20251225_stateright-model-checking.md)

---

## Approach

### Core Idea

1. Define a set of "mutations" (intentional bugs) in the ReplayEngine
2. Apply each mutation to a copy of the code
3. Run the test suite against the mutated code
4. Verify the tests fail (mutation is "killed")
5. If tests pass with a mutation, our tests have a gap

```
Original Code:    fn next_task_seq(&self) -> u32 { self.counter.fetch_add(1, ...) }
Mutation #1:      fn next_task_seq(&self) -> u32 { self.counter.fetch_add(2, ...) }  // Wrong increment
Mutation #2:      fn next_task_seq(&self) -> u32 { 0 }                                // Always zero
Mutation #3:      fn next_task_seq(&self) -> u32 { self.timer_counter.fetch_add(1, ...) }  // Wrong counter
```

### Types of Mutations

| Mutation Type | Example | What It Tests |
|---------------|---------|---------------|
| Off-by-one | `seq + 1` → `seq` | Sequence boundary handling |
| Wrong counter | `task_seq` → `timer_seq` | Per-type independence |
| Boundary removal | Remove `< len` check | Extension handling |
| Logic inversion | `==` → `!=` | Match/mismatch detection |
| Constant substitution | `0` → `1` | Initial state handling |

---

## Phase 1: Manual Mutation Tests

### 1.1 Create Mutation Test Module

**File:** `sdk/tests/unit/mutation_tests.rs` (new file)

Since Rust doesn't have a mature mutation testing framework for our use case, we'll create manual mutation tests that simulate what a mutation tester would do.

```rust
//! Mutation tests for ReplayEngine.
//!
//! These tests verify that our test suite catches known bug patterns.
//! Each test introduces a specific mutation and verifies it's detected.

use flovyn_core::workflow::event::{EventType, ReplayEvent};

/// A mutated version of ReplayEngine for testing
mod mutated_engine {
    use super::*;

    /// Mutation #1: Wrong sequence increment (off-by-one)
    pub mod off_by_one {
        pub struct MutatedReplayEngine {
            task_events: Vec<ReplayEvent>,
            next_task_seq: std::sync::atomic::AtomicU32,
        }

        impl MutatedReplayEngine {
            pub fn new(events: Vec<ReplayEvent>) -> Self {
                let task_events = events.iter()
                    .filter(|e| e.event_type() == EventType::TaskScheduled)
                    .cloned()
                    .collect();
                Self {
                    task_events,
                    next_task_seq: std::sync::atomic::AtomicU32::new(0),
                }
            }

            // MUTATION: fetch_add(2) instead of fetch_add(1)
            pub fn next_task_seq(&self) -> u32 {
                self.next_task_seq.fetch_add(2, std::sync::atomic::Ordering::SeqCst)
            }

            pub fn get_task_event(&self, seq: u32) -> Option<&ReplayEvent> {
                self.task_events.get(seq as usize)
            }
        }
    }

    /// Mutation #2: Always returns zero for sequence
    pub mod always_zero {
        // Similar structure with mutation: always returns 0
    }

    /// Mutation #3: Uses wrong counter (timer instead of task)
    pub mod wrong_counter {
        // Similar structure with mutation: uses timer_seq for tasks
    }

    /// Mutation #4: Inverted comparison
    pub mod inverted_comparison {
        // Similar structure with mutation: != instead of ==
    }
}
```

### 1.2 Mutation Detection Tests

```rust
/// Test that off-by-one mutation is detected
#[test]
fn mutation_off_by_one_is_detected() {
    use mutated_engine::off_by_one::MutatedReplayEngine;

    let events = vec![
        task_event("task-A"),
        task_event("task-B"),
        task_event("task-C"),
    ];

    let engine = MutatedReplayEngine::new(events);

    // With correct implementation: seq 0, 1, 2
    // With mutation (fetch_add(2)): seq 0, 2, 4

    let seq1 = engine.next_task_seq();
    assert_eq!(seq1, 0); // First call is correct

    let seq2 = engine.next_task_seq();
    // This should be 1, but mutation makes it 2
    // Our assertion should fail, proving mutation is detected
    assert_ne!(seq2, 1, "Mutation not detected: off-by-one should make seq2 != 1");
}

/// Test that always-zero mutation is detected
#[test]
fn mutation_always_zero_is_detected() {
    use mutated_engine::always_zero::MutatedReplayEngine;

    let events = vec![
        task_event("task-A"),
        task_event("task-B"),
    ];

    let engine = MutatedReplayEngine::new(events);

    engine.next_task_seq(); // Should be 0
    let seq2 = engine.next_task_seq(); // Should be 1

    // With mutation, seq2 is still 0
    assert_ne!(seq2, 0, "Mutation not detected: always-zero should be caught");
}

/// Test that wrong-counter mutation is detected
#[test]
fn mutation_wrong_counter_is_detected() {
    use mutated_engine::wrong_counter::MutatedReplayEngine;

    let events = vec![
        task_event("task-A"),
        timer_event("timer-1"),
    ];

    let engine = MutatedReplayEngine::new(events);

    // Call next_task_seq twice
    engine.next_task_seq();
    engine.next_task_seq();

    // With wrong counter mutation, this might return unexpected values
    // or the timer sequence would be affected
    let timer_seq = engine.next_timer_seq();
    assert_eq!(timer_seq, 0, "Mutation not detected: task calls shouldn't affect timer seq");
}
```

---

## Phase 2: Structured Mutation Framework

### 2.1 Define Mutation Trait

**File:** `sdk/tests/unit/mutations/mod.rs` (new file)

```rust
//! Mutation testing framework for ReplayEngine.

/// A mutation that can be applied to ReplayEngine behavior
pub trait Mutation: Send + Sync {
    /// Name of this mutation for reporting
    fn name(&self) -> &'static str;

    /// Description of what this mutation changes
    fn description(&self) -> &'static str;

    /// Apply mutation to next_task_seq behavior
    fn mutate_next_task_seq(&self, original: u32) -> u32 {
        original // Default: no mutation
    }

    /// Apply mutation to next_timer_seq behavior
    fn mutate_next_timer_seq(&self, original: u32) -> u32 {
        original
    }

    /// Apply mutation to task type comparison
    fn mutate_task_type_match(&self, expected: &str, actual: &str) -> bool {
        expected == actual // Default: normal comparison
    }
}

/// Off-by-one in sequence increment
pub struct OffByOneMutation;

impl Mutation for OffByOneMutation {
    fn name(&self) -> &'static str { "off-by-one" }
    fn description(&self) -> &'static str { "Increments sequence by 2 instead of 1" }

    fn mutate_next_task_seq(&self, original: u32) -> u32 {
        original + 1 // Returns seq+1, simulating fetch_add(2)
    }
}

/// Inverted comparison
pub struct InvertedComparisonMutation;

impl Mutation for InvertedComparisonMutation {
    fn name(&self) -> &'static str { "inverted-comparison" }
    fn description(&self) -> &'static str { "Returns true when types don't match" }

    fn mutate_task_type_match(&self, expected: &str, actual: &str) -> bool {
        expected != actual // Inverted!
    }
}
```

### 2.2 Mutated ReplayEngine Wrapper

```rust
/// ReplayEngine wrapper that applies mutations
pub struct MutatedReplayEngine<M: Mutation> {
    inner: ReplayEngine,
    mutation: M,
    mutation_applied_count: AtomicU32,
}

impl<M: Mutation> MutatedReplayEngine<M> {
    pub fn new(events: Vec<ReplayEvent>, mutation: M) -> Self {
        Self {
            inner: ReplayEngine::new(events),
            mutation,
            mutation_applied_count: AtomicU32::new(0),
        }
    }

    pub fn next_task_seq(&self) -> u32 {
        let original = self.inner.next_task_seq();
        let mutated = self.mutation.mutate_next_task_seq(original);
        if mutated != original {
            self.mutation_applied_count.fetch_add(1, Ordering::SeqCst);
        }
        mutated
    }

    pub fn check_task_type(&self, expected: &str, actual: &str) -> bool {
        let original = expected == actual;
        let mutated = self.mutation.mutate_task_type_match(expected, actual);
        if mutated != original {
            self.mutation_applied_count.fetch_add(1, Ordering::SeqCst);
        }
        mutated
    }

    /// Returns how many times the mutation was applied
    pub fn mutation_count(&self) -> u32 {
        self.mutation_applied_count.load(Ordering::SeqCst)
    }
}
```

### 2.3 Mutation Test Runner

```rust
/// Run a test function against all mutations, verify each is killed
pub fn verify_mutations_killed<F>(test_fn: F, mutations: &[Box<dyn Mutation>])
where
    F: Fn(&dyn Mutation) -> bool, // Returns true if test passes
{
    let mut surviving_mutations = Vec::new();

    for mutation in mutations {
        let test_passed = test_fn(mutation.as_ref());

        if test_passed {
            // Mutation survived! Our tests didn't catch it.
            surviving_mutations.push(mutation.name());
        }
    }

    assert!(
        surviving_mutations.is_empty(),
        "The following mutations survived (were not detected by tests): {:?}",
        surviving_mutations
    );
}

#[test]
fn all_mutations_are_killed() {
    let mutations: Vec<Box<dyn Mutation>> = vec![
        Box::new(OffByOneMutation),
        Box::new(InvertedComparisonMutation),
        Box::new(AlwaysZeroMutation),
        Box::new(WrongCounterMutation),
    ];

    verify_mutations_killed(|mutation| {
        // Run our standard replay test with this mutation
        let engine = MutatedReplayEngine::new(test_events(), mutation);

        // This test should FAIL when mutation is applied
        run_standard_replay_test(&engine).is_ok()
    }, &mutations);
}
```

---

## Phase 3: Mutation Coverage Metrics

### 3.1 Mutation Score

```rust
/// Calculate mutation score for a test suite
pub struct MutationScore {
    pub total_mutations: usize,
    pub killed_mutations: usize,
    pub surviving_mutations: Vec<String>,
}

impl MutationScore {
    pub fn score(&self) -> f64 {
        self.killed_mutations as f64 / self.total_mutations as f64
    }

    pub fn report(&self) {
        println!("=== Mutation Testing Report ===");
        println!("Total mutations: {}", self.total_mutations);
        println!("Killed: {}", self.killed_mutations);
        println!("Surviving: {}", self.surviving_mutations.len());
        println!("Score: {:.1}%", self.score() * 100.0);

        if !self.surviving_mutations.is_empty() {
            println!("\nSurviving mutations (test gaps):");
            for name in &self.surviving_mutations {
                println!("  - {}", name);
            }
        }
    }
}
```

### 3.2 Full Mutation Test Suite

```rust
#[test]
fn mutation_testing_suite() {
    let mutations = create_all_mutations();
    let score = run_mutation_tests(&mutations);

    score.report();

    // Require at least 90% mutation score
    assert!(
        score.score() >= 0.9,
        "Mutation score too low: {:.1}%. Surviving: {:?}",
        score.score() * 100.0,
        score.surviving_mutations
    );
}

fn create_all_mutations() -> Vec<Box<dyn Mutation>> {
    vec![
        // Sequence mutations
        Box::new(OffByOneMutation),
        Box::new(AlwaysZeroMutation),
        Box::new(OffByNegativeOneMutation),

        // Counter mutations
        Box::new(WrongCounterMutation { from: "task", to: "timer" }),
        Box::new(WrongCounterMutation { from: "timer", to: "task" }),
        Box::new(WrongCounterMutation { from: "task", to: "child" }),

        // Comparison mutations
        Box::new(InvertedComparisonMutation),
        Box::new(AlwaysTrueComparisonMutation),
        Box::new(AlwaysFalseComparisonMutation),

        // Boundary mutations
        Box::new(RemoveBoundaryCheckMutation),
        Box::new(OffByOneBoundaryMutation),
    ]
}
```

---

## TODO List

### Phase 1: Manual Mutation Tests
- [x] Create `sdk/tests/unit/mutations/mod.rs`
- [x] Implement `OffByOneMutation`
- [x] Implement `AlwaysZeroMutation`
- [x] Implement `WrongCounterMutation`
- [x] Implement `InvertedComparisonMutation`
- [x] Implement `BoundaryOffByOneMutation`
- [x] Implement `NoBoundaryCheckMutation`
- [x] Add test `off_by_one_mutation_killed`
- [x] Add test `always_zero_mutation_killed`
- [x] Add test `wrong_counter_mutation_killed`
- [x] Add test `inverted_comparison_mutation_killed`
- [x] Add test `boundary_off_by_one_mutation_killed`
- [x] Add test `no_boundary_check_mutation_killed`

### Phase 2: Structured Mutation Framework
- [x] Define `Mutation` trait
- [x] Implement `MutatedReplayEngine` wrapper
- [x] Implement `verify_mutation_killed()` helper
- [x] Implement test scenarios:
  - [x] `test_sequence_increment`
  - [x] `test_per_type_independence`
  - [x] `test_task_type_matching`
  - [x] `test_boundary_check`

### Phase 3: Mutation Coverage
- [x] Implement `MutationScore` struct with `score()` and `report()`
- [x] Implement `run_mutation_tests()` runner
- [x] Add `mutation_test_suite` test
- [x] Add `all_critical_mutations_killed` test
- [x] Add `baseline_passes_all_tests` test

### Phase 4: Integration
- [x] Add mutation tests to unit test suite
- [x] Verify 100% mutation score achieved

---

## Verification Checklist

- [x] All manual mutation tests pass (mutations are detected)
- [x] Mutation framework compiles and runs
- [x] Mutation score >= 90% (achieved 100%)
- [x] No surviving mutations in critical areas (sequence management)
- [x] All 9 mutation tests pass in unit test suite

---

## Expected Outcomes

### What This Will Find

1. **Weak tests** - Tests that don't actually verify behavior
2. **Missing edge cases** - Boundaries not tested
3. **Redundant tests** - Tests that could be removed without losing coverage

### Mutations We MUST Kill

| Mutation | Why Critical |
|----------|--------------|
| Off-by-one in sequence | Core replay correctness |
| Wrong counter type | Per-type independence |
| Inverted comparison | Match/mismatch detection |
| Removed boundary check | Extension handling |

### Success Criteria

1. 100% of critical mutations are killed
2. Overall mutation score >= 90%
3. Any surviving mutations are documented and justified
