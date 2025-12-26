//! Model checking tests using Stateright.
//!
//! These tests exhaustively verify correctness properties of the SDK
//! by exploring all possible state transitions.
//!
//! ## What is Model Checking?
//!
//! Model checking is a formal verification technique that exhaustively explores
//! all possible states of a system to verify correctness properties. Unlike
//! unit tests that check specific scenarios, model checking explores all possible
//! interleavings and edge cases.
//!
//! ## Models in this Module
//!
//! - `sequence_matching`: Verifies per-type sequence matching in replay engine
//! - `join_all`: Verifies join_all combinator semantics (fail-fast, order preservation)
//! - `select`: Verifies select combinator semantics (first-wins, error propagation)
//!
//! ## Running Model Checks
//!
//! ```bash
//! cargo test --test model -p flovyn-sdk -- --nocapture
//! ```

mod bridge;
mod comparison;
mod join_all;
mod select;
mod sequence_matching;
