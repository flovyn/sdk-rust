//! Mutation Testing Framework for ReplayEngine
//!
//! This module introduces known bugs (mutations) into a simulated ReplayEngine
//! and verifies that tests can detect them. This validates that our test suite
//! is actually capable of catching real bugs.
//!
//! ## How It Works
//!
//! 1. Define mutations that alter specific behaviors
//! 2. Create a `MutatedReplayEngine` that applies these mutations
//! 3. Run test scenarios against the mutated engine
//! 4. Verify tests fail (mutation is "killed")
//! 5. If tests pass with a mutation, we have a test gap
//!
//! ## Usage
//!
//! ```ignore
//! let score = run_mutation_tests();
//! assert!(score.score() >= 0.9, "Mutation score too low");
//! ```

use std::sync::atomic::{AtomicU32, Ordering};

/// A mutation that can be applied to ReplayEngine behavior.
#[allow(dead_code)]
pub trait Mutation: Send + Sync {
    /// Name of this mutation for reporting.
    fn name(&self) -> &'static str;

    /// Description of what this mutation changes.
    fn description(&self) -> &'static str;

    /// Mutate the sequence counter increment.
    /// Called after fetch_add, receives the original returned value.
    fn mutate_sequence(&self, original: u32) -> u32 {
        original // Default: no mutation
    }

    /// Mutate the comparison between expected and actual task type.
    fn mutate_task_type_match(&self, expected: &str, actual: &str) -> bool {
        expected == actual // Default: normal comparison
    }

    /// Mutate the boundary check (is seq within events range).
    fn mutate_boundary_check(&self, seq: u32, event_count: usize) -> bool {
        (seq as usize) < event_count // Default: normal check
    }

    /// Whether this mutation affects task sequence.
    fn affects_task_seq(&self) -> bool {
        false
    }

    /// Whether this mutation affects timer sequence.
    fn affects_timer_seq(&self) -> bool {
        false
    }
}

// =============================================================================
// Mutation Implementations
// =============================================================================

/// Off-by-one mutation: sequence increments by 2 instead of 1.
pub struct OffByOneMutation;

impl Mutation for OffByOneMutation {
    fn name(&self) -> &'static str {
        "off-by-one"
    }

    fn description(&self) -> &'static str {
        "Sequence counter returns seq+1 (as if fetch_add(2))"
    }

    fn mutate_sequence(&self, original: u32) -> u32 {
        original + 1 // Returns wrong value, simulating fetch_add(2)
    }

    fn affects_task_seq(&self) -> bool {
        true
    }

    fn affects_timer_seq(&self) -> bool {
        true
    }
}

/// Always-zero mutation: sequence always returns 0.
pub struct AlwaysZeroMutation;

impl Mutation for AlwaysZeroMutation {
    fn name(&self) -> &'static str {
        "always-zero"
    }

    fn description(&self) -> &'static str {
        "Sequence counter always returns 0"
    }

    fn mutate_sequence(&self, _original: u32) -> u32 {
        0 // Always returns 0
    }

    fn affects_task_seq(&self) -> bool {
        true
    }

    fn affects_timer_seq(&self) -> bool {
        true
    }
}

/// Wrong counter mutation: task sequence uses timer counter behavior.
/// This simulates mixing up per-type counters.
pub struct WrongCounterMutation;

impl Mutation for WrongCounterMutation {
    fn name(&self) -> &'static str {
        "wrong-counter"
    }

    fn description(&self) -> &'static str {
        "Task and timer counters are swapped"
    }

    // This mutation is handled specially in MutatedReplayEngine
    fn affects_task_seq(&self) -> bool {
        true
    }

    fn affects_timer_seq(&self) -> bool {
        true
    }
}

/// Inverted comparison mutation: match returns true when types differ.
pub struct InvertedComparisonMutation;

impl Mutation for InvertedComparisonMutation {
    fn name(&self) -> &'static str {
        "inverted-comparison"
    }

    fn description(&self) -> &'static str {
        "Type comparison returns opposite result"
    }

    fn mutate_task_type_match(&self, expected: &str, actual: &str) -> bool {
        expected != actual // Inverted!
    }
}

/// Boundary off-by-one mutation: uses <= instead of <.
pub struct BoundaryOffByOneMutation;

impl Mutation for BoundaryOffByOneMutation {
    fn name(&self) -> &'static str {
        "boundary-off-by-one"
    }

    fn description(&self) -> &'static str {
        "Boundary check uses <= instead of <"
    }

    fn mutate_boundary_check(&self, seq: u32, event_count: usize) -> bool {
        (seq as usize) <= event_count // Wrong: should be <
    }
}

/// No boundary check mutation: always returns true.
pub struct NoBoundaryCheckMutation;

impl Mutation for NoBoundaryCheckMutation {
    fn name(&self) -> &'static str {
        "no-boundary-check"
    }

    fn description(&self) -> &'static str {
        "Boundary check always returns true (no limit)"
    }

    fn mutate_boundary_check(&self, _seq: u32, _event_count: usize) -> bool {
        true // Always in bounds (wrong!)
    }
}

// =============================================================================
// Mutated ReplayEngine
// =============================================================================

/// A simplified ReplayEngine that applies mutations.
///
/// This doesn't use the real ReplayEngine but simulates its behavior
/// so we can inject mutations without modifying production code.
#[allow(dead_code)]
pub struct MutatedReplayEngine<'a> {
    task_events: Vec<String>,  // task types
    timer_events: Vec<String>, // timer IDs

    next_task_seq: AtomicU32,
    next_timer_seq: AtomicU32,

    mutation: &'a dyn Mutation,
}

#[allow(dead_code)]
impl<'a> MutatedReplayEngine<'a> {
    /// Create a new mutated engine.
    pub fn new(
        task_events: Vec<String>,
        timer_events: Vec<String>,
        mutation: &'a dyn Mutation,
    ) -> Self {
        Self {
            task_events,
            timer_events,
            next_task_seq: AtomicU32::new(0),
            next_timer_seq: AtomicU32::new(0),
            mutation,
        }
    }

    /// Get next task sequence (with mutation applied).
    pub fn next_task_seq(&self) -> u32 {
        let original = self.next_task_seq.fetch_add(1, Ordering::SeqCst);

        // Handle WrongCounterMutation specially
        if self.mutation.name() == "wrong-counter" {
            // Use timer counter instead!
            return self.next_timer_seq.fetch_add(1, Ordering::SeqCst);
        }

        if self.mutation.affects_task_seq() {
            self.mutation.mutate_sequence(original)
        } else {
            original
        }
    }

    /// Get next timer sequence (with mutation applied).
    pub fn next_timer_seq(&self) -> u32 {
        let original = self.next_timer_seq.fetch_add(1, Ordering::SeqCst);

        // Handle WrongCounterMutation specially
        if self.mutation.name() == "wrong-counter" {
            // Use task counter instead!
            return self.next_task_seq.fetch_add(1, Ordering::SeqCst);
        }

        if self.mutation.affects_timer_seq() {
            self.mutation.mutate_sequence(original)
        } else {
            original
        }
    }

    /// Check if in replay mode for task at given sequence.
    pub fn is_replaying_task(&self, seq: u32) -> bool {
        self.mutation
            .mutate_boundary_check(seq, self.task_events.len())
    }

    /// Get task type at sequence (if in bounds).
    pub fn get_task_type(&self, seq: u32) -> Option<&str> {
        self.task_events.get(seq as usize).map(|s| s.as_str())
    }

    /// Check if task type matches expected (with mutation applied).
    pub fn task_type_matches(&self, expected: &str, actual: &str) -> bool {
        self.mutation.mutate_task_type_match(expected, actual)
    }

    /// Get timer ID at sequence (if in bounds).
    pub fn get_timer_id(&self, seq: u32) -> Option<&str> {
        self.timer_events.get(seq as usize).map(|s| s.as_str())
    }

    /// Number of task events.
    pub fn task_event_count(&self) -> usize {
        self.task_events.len()
    }

    /// Number of timer events.
    pub fn timer_event_count(&self) -> usize {
        self.timer_events.len()
    }
}

// =============================================================================
// No-Mutation Baseline
// =============================================================================

/// A "mutation" that doesn't mutate anything - used as baseline.
pub struct NoMutation;

impl Mutation for NoMutation {
    fn name(&self) -> &'static str {
        "no-mutation"
    }

    fn description(&self) -> &'static str {
        "No mutation applied (baseline)"
    }
}

// =============================================================================
// Test Scenarios
// =============================================================================

/// Result of running a test scenario.
#[derive(Debug, Clone, PartialEq)]
pub enum TestResult {
    /// Test passed (mutation not detected).
    Passed,
    /// Test failed with given message (mutation detected).
    Failed(String),
}

impl TestResult {
    pub fn is_passed(&self) -> bool {
        matches!(self, TestResult::Passed)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, TestResult::Failed(_))
    }
}

/// Run the "sequence increment" test scenario.
///
/// This test verifies that calling next_task_seq() returns sequential values.
pub fn test_sequence_increment(engine: &MutatedReplayEngine) -> TestResult {
    let seq1 = engine.next_task_seq();
    let seq2 = engine.next_task_seq();
    let seq3 = engine.next_task_seq();

    if seq1 != 0 {
        return TestResult::Failed(format!("Expected seq1=0, got {}", seq1));
    }
    if seq2 != 1 {
        return TestResult::Failed(format!("Expected seq2=1, got {}", seq2));
    }
    if seq3 != 2 {
        return TestResult::Failed(format!("Expected seq3=2, got {}", seq3));
    }

    TestResult::Passed
}

/// Run the "per-type independence" test scenario.
///
/// This test verifies that task and timer sequences are independent.
pub fn test_per_type_independence(engine: &MutatedReplayEngine) -> TestResult {
    // Interleave task and timer calls
    let task1 = engine.next_task_seq();
    let timer1 = engine.next_timer_seq();
    let task2 = engine.next_task_seq();
    let timer2 = engine.next_timer_seq();

    if task1 != 0 {
        return TestResult::Failed(format!("Expected task1=0, got {}", task1));
    }
    if timer1 != 0 {
        return TestResult::Failed(format!("Expected timer1=0, got {}", timer1));
    }
    if task2 != 1 {
        return TestResult::Failed(format!("Expected task2=1, got {}", task2));
    }
    if timer2 != 1 {
        return TestResult::Failed(format!("Expected timer2=1, got {}", timer2));
    }

    TestResult::Passed
}

/// Run the "task type matching" test scenario.
///
/// This test verifies that task types are compared correctly.
pub fn test_task_type_matching(engine: &MutatedReplayEngine) -> TestResult {
    // Same types should match
    if !engine.task_type_matches("task-A", "task-A") {
        return TestResult::Failed("Same types should match".to_string());
    }

    // Different types should not match
    if engine.task_type_matches("task-A", "task-B") {
        return TestResult::Failed("Different types should not match".to_string());
    }

    TestResult::Passed
}

/// Run the "boundary check" test scenario.
///
/// This test verifies the boundary between replay and extension.
pub fn test_boundary_check(engine: &MutatedReplayEngine) -> TestResult {
    let event_count = engine.task_event_count();

    // Sequences within range should be replaying
    for seq in 0..event_count as u32 {
        if !engine.is_replaying_task(seq) {
            return TestResult::Failed(format!(
                "Seq {} should be replaying (event_count={})",
                seq, event_count
            ));
        }
    }

    // Sequence at event_count should NOT be replaying (extension)
    if engine.is_replaying_task(event_count as u32) {
        return TestResult::Failed(format!(
            "Seq {} should NOT be replaying (extension)",
            event_count
        ));
    }

    TestResult::Passed
}

/// Run the "replay validation" test scenario.
///
/// This test simulates a full replay with task type validation.
#[allow(dead_code)]
pub fn test_replay_validation(engine: &MutatedReplayEngine) -> TestResult {
    let events = vec!["task-A", "task-B", "task-C"];

    for expected_type in events {
        let seq = engine.next_task_seq();

        if let Some(actual_type) = engine.get_task_type(seq) {
            if !engine.task_type_matches(expected_type, actual_type) {
                return TestResult::Failed(format!(
                    "Type mismatch at seq {}: expected '{}', got '{}'",
                    seq, expected_type, actual_type
                ));
            }
        }
    }

    TestResult::Passed
}

// =============================================================================
// Mutation Test Runner
// =============================================================================

/// Score from running mutation tests.
#[derive(Debug)]
pub struct MutationScore {
    /// Total number of mutations tested.
    pub total: usize,
    /// Number of mutations that were killed (detected by tests).
    pub killed: usize,
    /// Mutations that survived (not detected).
    pub surviving: Vec<String>,
}

impl MutationScore {
    /// Calculate the mutation score as a percentage.
    pub fn score(&self) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        self.killed as f64 / self.total as f64
    }

    /// Print a report of the mutation testing results.
    pub fn report(&self) {
        println!("=== Mutation Testing Report ===");
        println!("Total mutations:    {}", self.total);
        println!("Killed mutations:   {}", self.killed);
        println!("Surviving mutations: {}", self.surviving.len());
        println!("Mutation score:     {:.1}%", self.score() * 100.0);

        if !self.surviving.is_empty() {
            println!("\nSurviving mutations (test gaps!):");
            for name in &self.surviving {
                println!("  - {}", name);
            }
        }
    }
}

/// Run all mutation tests and return the score.
pub fn run_mutation_tests() -> MutationScore {
    let mutations: Vec<Box<dyn Mutation>> = vec![
        Box::new(OffByOneMutation),
        Box::new(AlwaysZeroMutation),
        Box::new(WrongCounterMutation),
        Box::new(InvertedComparisonMutation),
        Box::new(BoundaryOffByOneMutation),
        Box::new(NoBoundaryCheckMutation),
    ];

    let test_events = vec![
        "task-A".to_string(),
        "task-B".to_string(),
        "task-C".to_string(),
    ];
    let timer_events = vec!["timer-1".to_string(), "timer-2".to_string()];

    let mut killed = 0;
    let mut surviving = Vec::new();

    for mutation in &mutations {
        let engine =
            MutatedReplayEngine::new(test_events.clone(), timer_events.clone(), mutation.as_ref());

        // Run all test scenarios
        let results = [
            ("sequence_increment", test_sequence_increment(&engine)),
            ("per_type_independence", test_per_type_independence(&engine)),
            ("task_type_matching", test_task_type_matching(&engine)),
            ("boundary_check", test_boundary_check(&engine)),
        ];

        // Check if any test caught the mutation
        let mutation_killed = results.iter().any(|(_, r)| r.is_failed());

        if mutation_killed {
            killed += 1;
        } else {
            surviving.push(mutation.name().to_string());
        }
    }

    MutationScore {
        total: mutations.len(),
        killed,
        surviving,
    }
}

/// Verify a specific mutation is killed by at least one test.
pub fn verify_mutation_killed<M: Mutation>(mutation: &M, test_events: Vec<String>) -> bool {
    let timer_events = vec!["timer-1".to_string(), "timer-2".to_string()];
    let engine = MutatedReplayEngine::new(test_events, timer_events, mutation);

    let results = [
        test_sequence_increment(&engine),
        test_per_type_independence(&engine),
        test_task_type_matching(&engine),
        test_boundary_check(&engine),
    ];

    results.iter().any(|r| r.is_failed())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a fresh engine for each test.
    fn make_engine(mutation: &dyn Mutation) -> MutatedReplayEngine<'_> {
        let events = vec![
            "task-A".to_string(),
            "task-B".to_string(),
            "task-C".to_string(),
        ];
        let timer_events = vec!["timer-1".to_string(), "timer-2".to_string()];
        MutatedReplayEngine::new(events, timer_events, mutation)
    }

    /// Verify baseline (no mutation) passes all tests.
    #[test]
    fn baseline_passes_all_tests() {
        // Each test needs a fresh engine since counters accumulate
        assert!(
            test_sequence_increment(&make_engine(&NoMutation)).is_passed(),
            "sequence_increment failed"
        );
        assert!(
            test_per_type_independence(&make_engine(&NoMutation)).is_passed(),
            "per_type_independence failed"
        );
        assert!(
            test_task_type_matching(&make_engine(&NoMutation)).is_passed(),
            "task_type_matching failed"
        );
        assert!(
            test_boundary_check(&make_engine(&NoMutation)).is_passed(),
            "boundary_check failed"
        );
    }

    /// Verify off-by-one mutation is detected.
    #[test]
    fn off_by_one_mutation_killed() {
        let events = vec!["task-A".to_string()];
        assert!(
            verify_mutation_killed(&OffByOneMutation, events),
            "Off-by-one mutation should be killed"
        );
    }

    /// Verify always-zero mutation is detected.
    #[test]
    fn always_zero_mutation_killed() {
        let events = vec!["task-A".to_string(), "task-B".to_string()];
        assert!(
            verify_mutation_killed(&AlwaysZeroMutation, events),
            "Always-zero mutation should be killed"
        );
    }

    /// Verify wrong-counter mutation is detected.
    #[test]
    fn wrong_counter_mutation_killed() {
        let events = vec!["task-A".to_string()];
        assert!(
            verify_mutation_killed(&WrongCounterMutation, events),
            "Wrong-counter mutation should be killed"
        );
    }

    /// Verify inverted-comparison mutation is detected.
    #[test]
    fn inverted_comparison_mutation_killed() {
        let events = vec!["task-A".to_string()];
        assert!(
            verify_mutation_killed(&InvertedComparisonMutation, events),
            "Inverted-comparison mutation should be killed"
        );
    }

    /// Verify boundary-off-by-one mutation is detected.
    #[test]
    fn boundary_off_by_one_mutation_killed() {
        let events = vec!["task-A".to_string(), "task-B".to_string()];
        assert!(
            verify_mutation_killed(&BoundaryOffByOneMutation, events),
            "Boundary-off-by-one mutation should be killed"
        );
    }

    /// Verify no-boundary-check mutation is detected.
    #[test]
    fn no_boundary_check_mutation_killed() {
        let events = vec!["task-A".to_string()];
        assert!(
            verify_mutation_killed(&NoBoundaryCheckMutation, events),
            "No-boundary-check mutation should be killed"
        );
    }

    /// Run full mutation test suite and verify score.
    #[test]
    fn mutation_test_suite() {
        let score = run_mutation_tests();
        score.report();

        assert!(
            score.score() >= 0.9,
            "Mutation score too low: {:.1}%. Surviving: {:?}",
            score.score() * 100.0,
            score.surviving
        );
    }

    /// All critical mutations must be killed.
    #[test]
    fn all_critical_mutations_killed() {
        let score = run_mutation_tests();

        // These are critical - must be killed
        let critical = vec!["off-by-one", "wrong-counter", "inverted-comparison"];

        for name in critical {
            assert!(
                !score.surviving.contains(&name.to_string()),
                "Critical mutation '{}' survived!",
                name
            );
        }
    }
}
