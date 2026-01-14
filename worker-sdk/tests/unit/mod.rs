//! Unit tests for flovyn-sdk

// Tests are in the source files using #[cfg(test)] modules
// This file is a placeholder for additional integration-style unit tests

mod determinism_props;
mod mutations;
mod shared_corpus;
mod state_machine_props;

#[test]
fn test_sdk_compiles() {
    // Verify the SDK compiles and basic types are accessible
    use flovyn_worker_sdk::prelude::*;

    let _version = SemanticVersion::new(1, 0, 0);
    let _validator = DeterminismValidator::new();
    let _collector = CommandCollector::new();
}
