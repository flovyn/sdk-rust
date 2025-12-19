//! Combinator functions for parallel workflow execution
//!
//! This module provides utility functions for combining and coordinating
//! multiple workflow futures. These combinators enable parallel execution
//! patterns while preserving determinism during replay.
//!
//! # Example
//!
//! ```ignore
//! use flovyn_sdk::workflow::combinators::{join_all, select};
//!
//! // Run multiple tasks in parallel and wait for all
//! let results = join_all(vec![
//!     ctx.schedule_async_raw("task-a", json!({})),
//!     ctx.schedule_async_raw("task-b", json!({})),
//!     ctx.schedule_async_raw("task-c", json!({})),
//! ]).await?;
//!
//! // Run multiple tasks and get the first result
//! let (index, result) = select(vec![
//!     ctx.schedule_async_raw("fast-task", json!({})),
//!     ctx.schedule_async_raw("slow-task", json!({})),
//! ]).await?;
//! ```

use crate::error::{FlovynError, Result};
use crate::workflow::future::CancellableFuture;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

// ============================================================================
// JoinAll - Wait for all futures to complete
// ============================================================================

/// Future that waits for all inner futures to complete.
///
/// Created by [`join_all`]. Returns `Ok(Vec<T>)` if all futures succeed,
/// or `Err` with the first error encountered.
pub struct JoinAll<F, T>
where
    F: Future<Output = Result<T>>,
{
    futures: Vec<Option<F>>,
    results: Vec<Option<T>>,
}

// JoinAll is Unpin if F is Unpin (Vec<Option<F>> is Unpin if F is Unpin)
impl<F, T> Unpin for JoinAll<F, T> where F: Future<Output = Result<T>> + Unpin {}

impl<F, T> JoinAll<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    fn new(futures: Vec<F>) -> Self {
        let len = futures.len();
        Self {
            futures: futures.into_iter().map(Some).collect(),
            results: (0..len).map(|_| None).collect(),
        }
    }
}

impl<F, T> Future for JoinAll<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    type Output = Result<Vec<T>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut all_done = true;

        for i in 0..this.futures.len() {
            if this.results[i].is_some() {
                continue; // Already completed
            }

            if let Some(ref mut future) = this.futures[i] {
                match Pin::new(future).poll(cx) {
                    Poll::Ready(Ok(result)) => {
                        this.results[i] = Some(result);
                        this.futures[i] = None;
                    }
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(Err(e));
                    }
                    Poll::Pending => {
                        all_done = false;
                    }
                }
            }
        }

        if all_done {
            let results: Vec<T> = this
                .results
                .iter_mut()
                .map(|r| r.take().expect("All results should be present"))
                .collect();
            Poll::Ready(Ok(results))
        } else {
            Poll::Pending
        }
    }
}

/// Wait for all futures to complete, returning their results in order.
///
/// Returns `Ok(Vec<T>)` if all futures succeed, or the first `Err` encountered.
/// Results are returned in the same order as the input futures.
///
/// # Example
///
/// ```ignore
/// let results = join_all(vec![
///     ctx.schedule_async_raw("task-a", json!({})),
///     ctx.schedule_async_raw("task-b", json!({})),
/// ]).await?;
/// ```
pub fn join_all<F, T>(futures: Vec<F>) -> JoinAll<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    JoinAll::new(futures)
}

// ============================================================================
// Select - Wait for first future to complete
// ============================================================================

/// Future that waits for the first inner future to complete.
///
/// Created by [`select`]. Returns the index and result of the first
/// future to complete. Does not cancel other futures - they remain
/// scheduled and will complete eventually.
pub struct Select<F, T>
where
    F: Future<Output = Result<T>>,
{
    futures: Vec<Option<F>>,
}

impl<F, T> Unpin for Select<F, T> where F: Future<Output = Result<T>> + Unpin {}

impl<F, T> Select<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    fn new(futures: Vec<F>) -> Self {
        Self {
            futures: futures.into_iter().map(Some).collect(),
        }
    }
}

impl<F, T> Future for Select<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    type Output = Result<(usize, T)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        for i in 0..this.futures.len() {
            if let Some(ref mut future) = this.futures[i] {
                match Pin::new(future).poll(cx) {
                    Poll::Ready(Ok(result)) => {
                        this.futures[i] = None;
                        return Poll::Ready(Ok((i, result)));
                    }
                    Poll::Ready(Err(e)) => {
                        this.futures[i] = None;
                        return Poll::Ready(Err(e));
                    }
                    Poll::Pending => {}
                }
            }
        }

        Poll::Pending
    }
}

/// Wait for the first future to complete, returning its index and result.
///
/// Returns `Ok((index, result))` for the first future that completes
/// successfully, or `Err` if the first completing future fails.
/// Other futures are NOT cancelled - they remain scheduled and will
/// complete eventually (their results are discarded).
///
/// # Example
///
/// ```ignore
/// let (winner_index, result) = select(vec![
///     ctx.schedule_async_raw("fast-task", json!({})),
///     ctx.schedule_async_raw("slow-task", json!({})),
/// ]).await?;
///
/// println!("Task {} finished first", winner_index);
/// ```
pub fn select<F, T>(futures: Vec<F>) -> Select<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    Select::new(futures)
}

// ============================================================================
// JoinN - Wait for N futures to complete
// ============================================================================

/// Future that waits for exactly N inner futures to complete.
///
/// Created by [`join_n`]. Returns the indices and results of the first
/// N futures to complete. Does not cancel remaining futures.
pub struct JoinN<F, T>
where
    F: Future<Output = Result<T>>,
{
    futures: Vec<Option<F>>,
    results: Vec<(usize, T)>,
    n: usize,
}

impl<F, T> Unpin for JoinN<F, T> where F: Future<Output = Result<T>> + Unpin {}

impl<F, T> JoinN<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    fn new(futures: Vec<F>, n: usize) -> Self {
        let n = n.min(futures.len()); // Can't wait for more than we have
        Self {
            futures: futures.into_iter().map(Some).collect(),
            results: Vec::with_capacity(n),
            n,
        }
    }
}

impl<F, T> Future for JoinN<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    type Output = Result<Vec<(usize, T)>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.results.len() >= this.n {
            let results = std::mem::take(&mut this.results);
            return Poll::Ready(Ok(results));
        }

        for i in 0..this.futures.len() {
            if this.results.len() >= this.n {
                break;
            }

            if let Some(ref mut future) = this.futures[i] {
                match Pin::new(future).poll(cx) {
                    Poll::Ready(Ok(result)) => {
                        this.futures[i] = None;
                        this.results.push((i, result));
                    }
                    Poll::Ready(Err(e)) => {
                        this.futures[i] = None;
                        return Poll::Ready(Err(e));
                    }
                    Poll::Pending => {}
                }
            }
        }

        if this.results.len() >= this.n {
            let results = std::mem::take(&mut this.results);
            Poll::Ready(Ok(results))
        } else {
            Poll::Pending
        }
    }
}

/// Wait for exactly N futures to complete, returning their indices and results.
///
/// Returns `Ok(Vec<(index, result)>)` with the first N results in completion
/// order. Returns `Err` if any of the first N futures to complete fails.
/// Remaining futures are NOT cancelled.
///
/// # Example
///
/// ```ignore
/// // Wait for any 2 of 3 tasks to complete
/// let winners = join_n(vec![
///     ctx.schedule_async_raw("task-a", json!({})),
///     ctx.schedule_async_raw("task-b", json!({})),
///     ctx.schedule_async_raw("task-c", json!({})),
/// ], 2).await?;
///
/// for (index, result) in winners {
///     println!("Task {} completed", index);
/// }
/// ```
pub fn join_n<F, T>(futures: Vec<F>, n: usize) -> JoinN<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    JoinN::new(futures, n)
}

// ============================================================================
// WithTimeout - Add timeout to a future
// ============================================================================

/// Future that wraps another future with a timeout.
///
/// Created by [`with_timeout`]. Returns the inner future's result if it
/// completes within the timeout, or `FlovynError::Timeout` if it doesn't.
///
/// Note: This requires a timer future from the workflow context to track
/// the timeout deterministically.
pub struct WithTimeout<F, T>
where
    F: Future<Output = Result<T>>,
{
    inner: F,
    timer: Option<crate::workflow::future::TimerFuture>,
    timed_out: bool,
}

impl<F, T> Unpin for WithTimeout<F, T> where F: Future<Output = Result<T>> + Unpin {}

impl<F, T> WithTimeout<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    fn new(inner: F, timer: crate::workflow::future::TimerFuture) -> Self {
        Self {
            inner,
            timer: Some(timer),
            timed_out: false,
        }
    }
}

impl<F, T> Future for WithTimeout<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Check if already timed out
        if this.timed_out {
            return Poll::Ready(Err(FlovynError::Timeout("Operation timed out".to_string())));
        }

        // Poll the inner future first
        match Pin::new(&mut this.inner).poll(cx) {
            Poll::Ready(result) => {
                // Cancel timer if inner completes first
                if let Some(ref timer) = this.timer {
                    timer.cancel();
                }
                return Poll::Ready(result);
            }
            Poll::Pending => {}
        }

        // Poll the timer
        if let Some(ref mut timer) = this.timer {
            match Pin::new(timer).poll(cx) {
                Poll::Ready(Ok(())) => {
                    // Timer fired - timeout occurred
                    this.timed_out = true;
                    this.timer = None;
                    return Poll::Ready(Err(FlovynError::Timeout(
                        "Operation timed out".to_string(),
                    )));
                }
                Poll::Ready(Err(_)) => {
                    // Timer was cancelled - this shouldn't happen normally
                    this.timer = None;
                }
                Poll::Pending => {}
            }
        }

        Poll::Pending
    }
}

/// Wrap a future with a timeout.
///
/// Returns the inner future's result if it completes within the timeout,
/// or `FlovynError::Timeout` if the timeout expires first.
///
/// # Arguments
///
/// * `future` - The future to wrap
/// * `timer` - A timer future from `ctx.sleep_async(duration)` for the timeout
///
/// # Example
///
/// ```ignore
/// // Create a task with a 30-second timeout
/// let task = ctx.schedule_async_raw("slow-task", json!({}));
/// let timer = ctx.sleep_async(Duration::from_secs(30));
///
/// match with_timeout(task, timer).await {
///     Ok(result) => println!("Task completed: {:?}", result),
///     Err(FlovynError::Timeout(_)) => println!("Task timed out!"),
///     Err(e) => println!("Task failed: {:?}", e),
/// }
/// ```
pub fn with_timeout<F, T>(
    future: F,
    timer: crate::workflow::future::TimerFuture,
) -> WithTimeout<F, T>
where
    F: Future<Output = Result<T>> + Unpin,
{
    WithTimeout::new(future, timer)
}

// ============================================================================
// Helper trait for cancellable futures in combinators
// ============================================================================

/// Extension trait for cancelling all futures in a collection.
pub trait CancelAll {
    /// Cancel all futures in this collection.
    fn cancel_all(&self);
}

impl<F: CancellableFuture> CancelAll for Vec<F> {
    fn cancel_all(&self) {
        for future in self.iter() {
            future.cancel();
        }
    }
}

impl<F: CancellableFuture> CancelAll for Vec<Option<F>> {
    fn cancel_all(&self) {
        for future in self.iter().flatten() {
            future.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::pin;

    // Helper to create a ready future
    fn ready_ok<T>(value: T) -> impl Future<Output = Result<T>> + Unpin {
        std::future::ready(Ok(value))
    }

    fn ready_err<T>(msg: &str) -> impl Future<Output = Result<T>> + Unpin {
        std::future::ready(Err(FlovynError::Other(msg.to_string())))
    }

    #[test]
    fn test_join_all_empty() {
        let futures: Vec<std::future::Ready<Result<i32>>> = vec![];
        let future = join_all(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(results)) => assert!(results.is_empty()),
            other => panic!("Expected Ready(Ok([])), got {:?}", other),
        }
    }

    #[test]
    fn test_join_all_all_ready() {
        let futures = vec![ready_ok(1), ready_ok(2), ready_ok(3)];
        let future = join_all(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(results)) => assert_eq!(results, vec![1, 2, 3]),
            other => panic!("Expected Ready(Ok([1, 2, 3])), got {:?}", other),
        }
    }

    #[test]
    fn test_join_all_one_error() {
        let futures: Vec<Box<dyn Future<Output = Result<i32>> + Unpin>> = vec![
            Box::new(ready_ok(1)),
            Box::new(ready_err("error")),
            Box::new(ready_ok(3)),
        ];
        let future = join_all(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(_)) => {} // Expected
            other => panic!("Expected Ready(Err(_)), got {:?}", other),
        }
    }

    #[test]
    fn test_select_returns_first_ready() {
        let futures = vec![ready_ok(1), ready_ok(2), ready_ok(3)];
        let future = select(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok((index, value))) => {
                assert_eq!(index, 0);
                assert_eq!(value, 1);
            }
            other => panic!("Expected Ready(Ok((0, 1))), got {:?}", other),
        }
    }

    #[test]
    fn test_select_empty() {
        let futures: Vec<std::future::Ready<Result<i32>>> = vec![];
        let future = select(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Pending => {} // Expected - no futures to complete
            other => panic!("Expected Pending, got {:?}", other),
        }
    }

    #[test]
    fn test_join_n_partial() {
        let futures = vec![ready_ok(1), ready_ok(2), ready_ok(3)];
        let future = join_n(futures, 2);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(results)) => {
                assert_eq!(results.len(), 2);
                // First two should be (0, 1) and (1, 2)
                assert_eq!(results[0], (0, 1));
                assert_eq!(results[1], (1, 2));
            }
            other => panic!("Expected Ready(Ok([...])), got {:?}", other),
        }
    }

    #[test]
    fn test_join_n_more_than_available() {
        let futures = vec![ready_ok(1), ready_ok(2)];
        let future = join_n(futures, 5); // Ask for 5 but only 2 available
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(results)) => {
                assert_eq!(results.len(), 2); // Should only get 2
            }
            other => panic!("Expected Ready(Ok([...])), got {:?}", other),
        }
    }

    #[test]
    fn test_join_n_zero() {
        let futures = vec![ready_ok(1), ready_ok(2)];
        let future = join_n(futures, 0);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(results)) => assert!(results.is_empty()),
            other => panic!("Expected Ready(Ok([])), got {:?}", other),
        }
    }

    #[test]
    fn test_join_n_with_error() {
        let futures: Vec<Box<dyn Future<Output = Result<i32>> + Unpin>> = vec![
            Box::new(ready_ok(1)),
            Box::new(ready_err("error")),
            Box::new(ready_ok(3)),
        ];
        let future = join_n(futures, 2);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        // First poll gets result 1, then hits error
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(_)) => {} // Expected - error in first N
            other => panic!("Expected Ready(Err(_)), got {:?}", other),
        }
    }

    #[test]
    fn test_select_returns_error_on_first_failure() {
        let futures: Vec<Box<dyn Future<Output = Result<i32>> + Unpin>> =
            vec![Box::new(ready_err("error")), Box::new(ready_ok(2))];
        let future = select(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(_)) => {} // Expected - first future failed
            other => panic!("Expected Ready(Err(_)), got {:?}", other),
        }
    }

    // Dummy timer context for tests
    struct DummyTimerCtx;
    impl crate::workflow::future::TimerFutureContext for DummyTimerCtx {
        fn find_timer_result(&self, _: &str) -> Option<crate::Result<()>> {
            None
        }
        fn record_cancel_timer(&self, _: &str) {}
    }

    #[test]
    fn test_with_timeout_returns_result_if_inner_completes_first() {
        // Create a ready inner future and a timer
        let inner = ready_ok(42);
        // Create a timer that won't fire (using Pending)
        let timer = crate::workflow::future::TimerFuture::new(
            0,
            "test-timer".to_string(),
            std::sync::Weak::<DummyTimerCtx>::new(),
        );

        let future = with_timeout(inner, timer);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(42)) => {} // Expected - inner completed first
            other => panic!("Expected Ready(Ok(42)), got {:?}", other),
        }
    }

    #[test]
    fn test_with_timeout_returns_timeout_error_if_timer_fires_first() {
        // Create an inner future that won't complete (Pending)
        struct PendingFuture;
        impl Future for PendingFuture {
            type Output = Result<i32>;
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
                Poll::Pending
            }
        }
        impl Unpin for PendingFuture {}

        // Create a timer that fires immediately (using from_replay with fired=true)
        let timer = crate::workflow::future::TimerFuture::from_replay(
            0,
            "test-timer".to_string(),
            std::sync::Weak::<DummyTimerCtx>::new(),
            true, // fired
        );

        let future = with_timeout(PendingFuture, timer);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Timeout(_))) => {} // Expected
            other => panic!("Expected Ready(Err(Timeout)), got {:?}", other),
        }
    }

    #[test]
    fn test_with_timeout_inner_error_propagates() {
        let inner = ready_err::<i32>("inner error");
        let timer = crate::workflow::future::TimerFuture::new(
            0,
            "test-timer".to_string(),
            std::sync::Weak::<DummyTimerCtx>::new(),
        );

        let future = with_timeout(inner, timer);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(msg))) if msg == "inner error" => {} // Expected
            other => panic!("Expected Ready(Err(Other(inner error))), got {:?}", other),
        }
    }

    #[test]
    fn test_join_all_preserves_order() {
        // Even if futures complete in different orders during replay,
        // results should be in input order
        let futures = vec![ready_ok(10), ready_ok(20), ready_ok(30)];
        let future = join_all(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(results)) => {
                assert_eq!(results, vec![10, 20, 30]);
            }
            other => panic!("Expected Ready(Ok([10, 20, 30])), got {:?}", other),
        }
    }

    #[test]
    fn test_join_all_single_future() {
        let futures = vec![ready_ok(42)];
        let future = join_all(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(results)) => assert_eq!(results, vec![42]),
            other => panic!("Expected Ready(Ok([42])), got {:?}", other),
        }
    }

    #[test]
    fn test_select_with_all_errors() {
        let futures: Vec<Box<dyn Future<Output = Result<i32>> + Unpin>> =
            vec![Box::new(ready_err("error1")), Box::new(ready_err("error2"))];
        let future = select(futures);
        let mut future = pin!(future);

        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(Err(FlovynError::Other(msg))) if msg == "error1" => {} // First error
            other => panic!("Expected Ready(Err(Other(error1))), got {:?}", other),
        }
    }
}
