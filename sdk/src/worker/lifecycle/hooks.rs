//! Worker lifecycle hooks for observing and reacting to events.

use super::events::WorkerLifecycleEvent;
use super::types::WorkerStatus;
use async_trait::async_trait;
use std::sync::Arc;

/// Hook for observing and reacting to worker lifecycle events.
///
/// Implement this trait to receive notifications about worker lifecycle
/// changes, including status transitions, events, and control requests.
///
/// All methods have default no-op implementations, so you only need to
/// implement the methods you're interested in.
///
/// # Example
///
/// ```ignore
/// use flovyn_sdk::worker::lifecycle::{WorkerLifecycleHook, WorkerLifecycleEvent};
///
/// struct LoggingHook;
///
/// #[async_trait::async_trait]
/// impl WorkerLifecycleHook for LoggingHook {
///     async fn on_event(&self, event: WorkerLifecycleEvent) {
///         println!("Event: {:?}", event);
///     }
/// }
/// ```
#[async_trait]
pub trait WorkerLifecycleHook: Send + Sync {
    /// Called when a lifecycle event occurs.
    async fn on_event(&self, event: WorkerLifecycleEvent) {
        let _ = event;
    }

    /// Called when the worker status changes.
    async fn on_status_change(&self, old: WorkerStatus, new: WorkerStatus) {
        let _ = (old, new);
    }

    /// Called before the worker starts polling.
    ///
    /// Return `false` to prevent the worker from starting.
    async fn on_before_start(&self) -> bool {
        true
    }

    /// Called before the worker stops.
    ///
    /// Can perform cleanup operations.
    async fn on_before_stop(&self, graceful: bool) {
        let _ = graceful;
    }

    /// Called when the worker should pause.
    ///
    /// Return `false` to reject the pause request.
    async fn on_pause_requested(&self, reason: &str) -> bool {
        let _ = reason;
        true
    }

    /// Called when the worker should resume.
    ///
    /// Return `false` to reject the resume request.
    async fn on_resume_requested(&self) -> bool {
        true
    }
}

/// A chain of lifecycle hooks that are executed in order.
pub struct HookChain {
    hooks: Vec<Arc<dyn WorkerLifecycleHook>>,
}

impl Default for HookChain {
    fn default() -> Self {
        Self::new()
    }
}

impl HookChain {
    /// Creates a new empty hook chain.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Adds a hook to the chain.
    pub fn add<H: WorkerLifecycleHook + 'static>(&mut self, hook: H) {
        self.hooks.push(Arc::new(hook));
    }

    /// Adds an Arc-wrapped hook to the chain.
    pub fn add_arc(&mut self, hook: Arc<dyn WorkerLifecycleHook>) {
        self.hooks.push(hook);
    }

    /// Returns the number of hooks in the chain.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Returns true if the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Emits an event to all hooks in order.
    pub async fn emit(&self, event: WorkerLifecycleEvent) {
        for hook in &self.hooks {
            hook.on_event(event.clone()).await;
        }
    }

    /// Notifies all hooks of a status change.
    pub async fn on_status_change(&self, old: WorkerStatus, new: WorkerStatus) {
        for hook in &self.hooks {
            hook.on_status_change(old.clone(), new.clone()).await;
        }
    }

    /// Calls `on_before_start` on all hooks.
    ///
    /// Returns `false` if any hook returns `false`.
    pub async fn before_start(&self) -> bool {
        for hook in &self.hooks {
            if !hook.on_before_start().await {
                return false;
            }
        }
        true
    }

    /// Calls `on_before_stop` on all hooks.
    pub async fn before_stop(&self, graceful: bool) {
        for hook in &self.hooks {
            hook.on_before_stop(graceful).await;
        }
    }

    /// Calls `on_pause_requested` on all hooks.
    ///
    /// Returns `false` if any hook returns `false`.
    pub async fn on_pause_requested(&self, reason: &str) -> bool {
        for hook in &self.hooks {
            if !hook.on_pause_requested(reason).await {
                return false;
            }
        }
        true
    }

    /// Calls `on_resume_requested` on all hooks.
    ///
    /// Returns `false` if any hook returns `false`.
    pub async fn on_resume_requested(&self) -> bool {
        for hook in &self.hooks {
            if !hook.on_resume_requested().await {
                return false;
            }
        }
        true
    }
}

impl Clone for HookChain {
    fn clone(&self) -> Self {
        Self {
            hooks: self.hooks.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::SystemTime;

    struct CountingHook {
        event_count: AtomicU32,
        status_change_count: AtomicU32,
    }

    impl CountingHook {
        fn new() -> Self {
            Self {
                event_count: AtomicU32::new(0),
                status_change_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl WorkerLifecycleHook for CountingHook {
        async fn on_event(&self, _event: WorkerLifecycleEvent) {
            self.event_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn on_status_change(&self, _old: WorkerStatus, _new: WorkerStatus) {
            self.status_change_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct RejectingHook;

    #[async_trait]
    impl WorkerLifecycleHook for RejectingHook {
        async fn on_before_start(&self) -> bool {
            false
        }

        async fn on_pause_requested(&self, _reason: &str) -> bool {
            false
        }

        async fn on_resume_requested(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn test_hook_chain_empty() {
        let chain = HookChain::new();
        assert!(chain.is_empty());
        assert_eq!(chain.len(), 0);
    }

    #[tokio::test]
    async fn test_hook_chain_add() {
        let mut chain = HookChain::new();
        chain.add(CountingHook::new());
        assert!(!chain.is_empty());
        assert_eq!(chain.len(), 1);
    }

    #[tokio::test]
    async fn test_hook_chain_emit() {
        let hook = Arc::new(CountingHook::new());
        let mut chain = HookChain::new();
        chain.add_arc(hook.clone());

        chain.emit(WorkerLifecycleEvent::HeartbeatSent).await;
        chain.emit(WorkerLifecycleEvent::Reconnected).await;

        assert_eq!(hook.event_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_hook_chain_status_change() {
        let hook = Arc::new(CountingHook::new());
        let mut chain = HookChain::new();
        chain.add_arc(hook.clone());

        chain
            .on_status_change(
                WorkerStatus::Initializing,
                WorkerStatus::Running {
                    server_worker_id: None,
                    started_at: SystemTime::now(),
                },
            )
            .await;

        assert_eq!(hook.status_change_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_hook_chain_before_start_all_accept() {
        let mut chain = HookChain::new();
        chain.add(CountingHook::new());
        chain.add(CountingHook::new());

        assert!(chain.before_start().await);
    }

    #[tokio::test]
    async fn test_hook_chain_before_start_one_rejects() {
        let mut chain = HookChain::new();
        chain.add(CountingHook::new());
        chain.add(RejectingHook);

        assert!(!chain.before_start().await);
    }

    #[tokio::test]
    async fn test_hook_chain_pause_requested() {
        let mut chain = HookChain::new();
        chain.add(CountingHook::new());
        assert!(chain.on_pause_requested("maintenance").await);

        chain.add(RejectingHook);
        assert!(!chain.on_pause_requested("maintenance").await);
    }

    #[tokio::test]
    async fn test_hook_chain_resume_requested() {
        let mut chain = HookChain::new();
        chain.add(CountingHook::new());
        assert!(chain.on_resume_requested().await);

        chain.add(RejectingHook);
        assert!(!chain.on_resume_requested().await);
    }

    #[tokio::test]
    async fn test_hook_chain_clone() {
        let hook = Arc::new(CountingHook::new());
        let mut chain = HookChain::new();
        chain.add_arc(hook.clone());

        let cloned = chain.clone();
        cloned.emit(WorkerLifecycleEvent::HeartbeatSent).await;

        // Original hook should receive the event from cloned chain
        assert_eq!(hook.event_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_hooks_called_in_order() {
        use std::sync::Mutex;

        struct OrderTrackingHook {
            order: Arc<Mutex<Vec<u32>>>,
            id: u32,
        }

        #[async_trait]
        impl WorkerLifecycleHook for OrderTrackingHook {
            async fn on_event(&self, _event: WorkerLifecycleEvent) {
                self.order.lock().unwrap().push(self.id);
            }
        }

        let order = Arc::new(Mutex::new(Vec::new()));
        let mut chain = HookChain::new();

        chain.add(OrderTrackingHook {
            order: order.clone(),
            id: 1,
        });
        chain.add(OrderTrackingHook {
            order: order.clone(),
            id: 2,
        });
        chain.add(OrderTrackingHook {
            order: order.clone(),
            id: 3,
        });

        chain.emit(WorkerLifecycleEvent::HeartbeatSent).await;

        let recorded_order = order.lock().unwrap().clone();
        assert_eq!(recorded_order, vec![1, 2, 3]);
    }
}
