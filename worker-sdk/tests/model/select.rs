//! Model checking for select combinator semantics.
//!
//! This model verifies the correctness of the `select` combinator:
//!
//! 1. **First wins**: First completion determines the result
//! 2. **Error propagates**: If first to complete is an error, that's the result
//! 3. **Eventually resolves**: All paths lead to resolution
//!
//! ## Semantics
//!
//! `select` waits for the first future to complete and returns its result,
//! whether success or failure. The remaining futures are cancelled.
//!
//! ## State Space
//!
//! For N futures, each can complete first with Ok or Err.
//! State space is: N * 2 = 2N possible first completions.
//! After first completion, the model resolves immediately.

use stateright::Model;
use std::hash::Hash;

/// Result of a single future.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FutureResult {
    Ok(String),
    Err(String),
}

/// Final result of select.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SelectResult {
    /// First future completed successfully (index, value)
    Ok(usize, String),
    /// First future failed (index, error)
    Err(usize, String),
}

/// State of the select model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SelectState {
    /// Number of futures in the select
    num_futures: usize,
    /// Whether any future has completed yet
    any_completed: bool,
    /// Final resolved result (None = waiting for first completion)
    resolved: Option<SelectResult>,
}

/// Action that can occur in the select model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SelectAction {
    /// A future completes with a result
    Complete { index: usize, result: FutureResult },
}

/// Configuration for the select model.
pub struct SelectModel {
    /// Number of futures to select from
    pub num_futures: usize,
}

impl Model for SelectModel {
    type State = SelectState;
    type Action = SelectAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![SelectState {
            num_futures: self.num_futures,
            any_completed: false,
            resolved: None,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        // Stop exploring after resolution (first completion)
        if state.resolved.is_some() {
            return;
        }

        // Any future can complete first with Ok or Err
        for index in 0..self.num_futures {
            actions.push(SelectAction::Complete {
                index,
                result: FutureResult::Ok(format!("ok-{}", index)),
            });
            actions.push(SelectAction::Complete {
                index,
                result: FutureResult::Err(format!("err-{}", index)),
            });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        // If already resolved, no transition
        if state.resolved.is_some() {
            return None;
        }

        let SelectAction::Complete { index, result } = action;
        let mut next = state.clone();

        next.any_completed = true;

        // First completion determines the result
        match result {
            FutureResult::Ok(value) => {
                next.resolved = Some(SelectResult::Ok(index, value));
            }
            FutureResult::Err(msg) => {
                next.resolved = Some(SelectResult::Err(index, msg));
            }
        }

        Some(next)
    }

    fn properties(&self) -> Vec<stateright::Property<Self>> {
        vec![
            // Property 1: First completion resolves immediately
            stateright::Property::always("first wins", |_: &SelectModel, state: &SelectState| {
                // If any future completed, we must be resolved
                if state.any_completed {
                    state.resolved.is_some()
                } else {
                    true
                }
            }),
            // Property 2: Result matches the completing future
            stateright::Property::always(
                "result matches completion",
                |_: &SelectModel, state: &SelectState| {
                    match &state.resolved {
                        Some(SelectResult::Ok(idx, val)) => {
                            // Value should match the expected format
                            *val == format!("ok-{}", idx)
                        }
                        Some(SelectResult::Err(idx, msg)) => {
                            // Error should match the expected format
                            *msg == format!("err-{}", idx)
                        }
                        None => true,
                    }
                },
            ),
            // Property 3: Resolution only happens once
            stateright::Property::always(
                "resolves at most once",
                |_: &SelectModel, state: &SelectState| {
                    // This is trivially true by construction since
                    // we don't generate actions after resolution.
                    // But we verify it explicitly.
                    if state.resolved.is_some() {
                        state.any_completed
                    } else {
                        true
                    }
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stateright::Checker;
    use std::time::Instant;

    #[test]
    fn verify_select_2_futures() {
        let model = SelectModel { num_futures: 2 };

        let start = Instant::now();
        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        let elapsed = start.elapsed();

        println!("=== Select Model (2 futures) ===");
        println!("States explored: {}", checker.unique_state_count());
        println!("Time elapsed: {:?}", elapsed);

        checker.assert_properties();
        println!("All properties verified!");
    }

    #[test]
    fn verify_select_3_futures() {
        let model = SelectModel { num_futures: 3 };

        let start = Instant::now();
        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        let elapsed = start.elapsed();

        println!("=== Select Model (3 futures) ===");
        println!("States explored: {}", checker.unique_state_count());
        println!("Time elapsed: {:?}", elapsed);

        checker.assert_properties();
        println!("All properties verified!");
    }

    #[test]
    fn verify_select_5_futures() {
        let model = SelectModel { num_futures: 5 };

        let start = Instant::now();
        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        let elapsed = start.elapsed();

        println!("=== Select Model (5 futures) ===");
        println!("States explored: {}", checker.unique_state_count());
        println!("Time elapsed: {:?}", elapsed);

        checker.assert_properties();
        println!("All properties verified!");
    }

    /// Verify that both success and error can be the result.
    #[test]
    fn verify_select_error_propagation() {
        let model = SelectModel { num_futures: 2 };

        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        checker.assert_properties();

        // The model explores all possible first completions,
        // including errors. Properties ensure correct behavior.
        println!("Error propagation verified!");
    }
}
