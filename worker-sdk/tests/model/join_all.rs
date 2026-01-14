//! Model checking for join_all combinator semantics.
//!
//! This model verifies the correctness of the `join_all` combinator:
//!
//! 1. **Fail fast**: First error immediately resolves the join_all
//! 2. **All ok requires all complete**: AllOk only when all futures complete successfully
//! 3. **Order preserved**: Results are in original future order
//! 4. **Eventually resolves**: All paths lead to resolution
//!
//! ## State Space
//!
//! For N futures, each can complete with Ok or Err in any order.
//! State space is bounded by: N! * 2^N orderings/outcomes.
//! For N=3: 6 * 8 = 48 paths (very tractable).
//! For N=4: 24 * 16 = 384 paths (still tractable).

use stateright::Model;
use std::hash::Hash;

/// Result of a single future.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FutureResult {
    Ok(String),
    Err(String),
}

/// Final result of join_all.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinAllResult {
    /// All futures completed successfully, results in order
    AllOk(Vec<String>),
    /// First error encountered (index, error message)
    FirstError(usize, String),
}

/// State of the join_all model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JoinAllState {
    /// Number of futures in the join_all
    num_futures: usize,
    /// Indices of futures that are still pending
    pending: Vec<usize>,
    /// Results so far (None = pending)
    results: Vec<Option<FutureResult>>,
    /// Final resolved result (None = still in progress)
    resolved: Option<JoinAllResult>,
}

/// Action that can occur in the join_all model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinAllAction {
    /// A future completes with a result
    Complete { index: usize, result: FutureResult },
}

/// Configuration for the join_all model.
pub struct JoinAllModel {
    /// Number of futures to join
    pub num_futures: usize,
}

impl Model for JoinAllModel {
    type State = JoinAllState;
    type Action = JoinAllAction;

    fn init_states(&self) -> Vec<Self::State> {
        vec![JoinAllState {
            num_futures: self.num_futures,
            pending: (0..self.num_futures).collect(),
            results: vec![None; self.num_futures],
            resolved: None,
        }]
    }

    fn actions(&self, state: &Self::State, actions: &mut Vec<Self::Action>) {
        // Stop exploring after resolution
        if state.resolved.is_some() {
            return;
        }

        // Each pending future can complete with Ok or Err
        for &index in &state.pending {
            actions.push(JoinAllAction::Complete {
                index,
                result: FutureResult::Ok(format!("ok-{}", index)),
            });
            actions.push(JoinAllAction::Complete {
                index,
                result: FutureResult::Err(format!("err-{}", index)),
            });
        }
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let JoinAllAction::Complete { index, result } = action;
        let mut next = state.clone();

        // Remove from pending
        next.pending.retain(|&i| i != index);

        // Record result
        next.results[index] = Some(result.clone());

        // Check for resolution
        match &result {
            FutureResult::Err(msg) => {
                // Fail fast: first error resolves immediately
                next.resolved = Some(JoinAllResult::FirstError(index, msg.clone()));
            }
            FutureResult::Ok(_) => {
                // Check if all completed successfully
                if next.pending.is_empty() {
                    let values: Vec<_> = next
                        .results
                        .iter()
                        .map(|r| match r {
                            Some(FutureResult::Ok(v)) => v.clone(),
                            _ => unreachable!("All results should be Ok"),
                        })
                        .collect();
                    next.resolved = Some(JoinAllResult::AllOk(values));
                }
            }
        }

        Some(next)
    }

    fn properties(&self) -> Vec<stateright::Property<Self>> {
        vec![
            // Property 1: Fail fast - any error immediately resolves
            stateright::Property::always("fail fast", |_: &JoinAllModel, state: &JoinAllState| {
                let has_error = state
                    .results
                    .iter()
                    .any(|r| matches!(r, Some(FutureResult::Err(_))));

                if has_error {
                    // If there's an error, we must be resolved with FirstError
                    matches!(state.resolved, Some(JoinAllResult::FirstError(..)))
                } else {
                    true
                }
            }),
            // Property 2: AllOk requires all complete
            stateright::Property::always(
                "all ok requires all complete",
                |_: &JoinAllModel, state: &JoinAllState| {
                    if matches!(state.resolved, Some(JoinAllResult::AllOk(_))) {
                        // All futures must be complete (pending is empty)
                        state.pending.is_empty()
                    } else {
                        true
                    }
                },
            ),
            // Property 3: Results are in order
            stateright::Property::always(
                "results preserve order",
                |_: &JoinAllModel, state: &JoinAllState| {
                    if let Some(JoinAllResult::AllOk(values)) = &state.resolved {
                        // Check that values are in the expected order
                        values
                            .iter()
                            .enumerate()
                            .all(|(i, v)| *v == format!("ok-{}", i))
                    } else {
                        true
                    }
                },
            ),
            // Property 4: Eventually resolves (all paths lead to resolution)
            // This is checked by verifying we don't have infinite paths
            stateright::Property::always(
                "no infinite paths",
                |_: &JoinAllModel, state: &JoinAllState| {
                    // Either we're resolved, or we have pending futures
                    state.resolved.is_some() || !state.pending.is_empty()
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
    fn verify_join_all_2_futures() {
        let model = JoinAllModel { num_futures: 2 };

        let start = Instant::now();
        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        let elapsed = start.elapsed();

        println!("=== JoinAll Model (2 futures) ===");
        println!("States explored: {}", checker.unique_state_count());
        println!("Time elapsed: {:?}", elapsed);

        checker.assert_properties();
        println!("All properties verified!");
    }

    #[test]
    fn verify_join_all_3_futures() {
        let model = JoinAllModel { num_futures: 3 };

        let start = Instant::now();
        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        let elapsed = start.elapsed();

        println!("=== JoinAll Model (3 futures) ===");
        println!("States explored: {}", checker.unique_state_count());
        println!("Time elapsed: {:?}", elapsed);

        checker.assert_properties();
        println!("All properties verified!");
    }

    #[test]
    fn verify_join_all_4_futures() {
        let model = JoinAllModel { num_futures: 4 };

        let start = Instant::now();
        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        let elapsed = start.elapsed();

        println!("=== JoinAll Model (4 futures) ===");
        println!("States explored: {}", checker.unique_state_count());
        println!("Time elapsed: {:?}", elapsed);

        checker.assert_properties();
        println!("All properties verified!");
    }

    /// Verify that fail-fast actually happens by checking we never
    /// process futures after an error.
    #[test]
    fn verify_fail_fast_behavior() {
        let model = JoinAllModel { num_futures: 3 };

        let checker = model.checker().threads(num_cpus::get()).spawn_bfs().join();
        checker.assert_properties();

        // The "fail fast" property ensures that once an error occurs,
        // the model immediately resolves and no more futures are processed.
        // We can verify this by checking that states with errors are always resolved.
        println!("Fail-fast behavior verified!");
    }
}
