//! TCK tests for flovyn-sdk
//!
//! These tests validate the SDK against the TCK scenarios in spec/scenarios/
//! and determinism corpus tests in tests/shared/replay-corpus/

mod determinism_corpus;

#[test]
#[ignore] // Enable when full TCK runner is implemented
fn test_tck_scenarios() {
    // TODO: Implement full TCK runner for spec/scenarios
}
