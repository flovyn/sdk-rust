//! Worker metrics collection and tracking.

use std::time::Duration;

/// Runtime metrics for the worker.
#[derive(Debug, Clone, Default)]
pub struct WorkerMetrics {
    /// Total workflows executed since start.
    pub workflows_executed: u64,

    /// Total workflows currently in progress.
    pub workflows_in_progress: usize,

    /// Total tasks executed since start.
    pub tasks_executed: u64,

    /// Total tasks currently in progress.
    pub tasks_in_progress: usize,

    /// Total workflow execution errors.
    pub workflow_errors: u64,

    /// Total task execution errors.
    pub task_errors: u64,

    /// Average workflow execution time (milliseconds).
    pub avg_workflow_duration_ms: f64,

    /// Average task execution time (milliseconds).
    pub avg_task_duration_ms: f64,

    /// Time since worker started.
    pub uptime: Duration,

    // Internal tracking for average calculation (not exposed)
    total_workflow_duration_ms: u64,
    total_task_duration_ms: u64,
}

impl WorkerMetrics {
    /// Creates a new WorkerMetrics instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a workflow starting execution.
    pub fn record_workflow_started(&mut self) {
        self.workflows_in_progress += 1;
    }

    /// Records a workflow completing successfully.
    pub fn record_workflow_completed(&mut self, duration: Duration) {
        if self.workflows_in_progress > 0 {
            self.workflows_in_progress -= 1;
        }
        self.workflows_executed += 1;
        let duration_ms = duration.as_millis() as u64;
        self.total_workflow_duration_ms += duration_ms;
        self.avg_workflow_duration_ms =
            self.total_workflow_duration_ms as f64 / self.workflows_executed as f64;
    }

    /// Records a workflow failing.
    pub fn record_workflow_failed(&mut self, duration: Duration) {
        if self.workflows_in_progress > 0 {
            self.workflows_in_progress -= 1;
        }
        self.workflow_errors += 1;
        self.workflows_executed += 1;
        let duration_ms = duration.as_millis() as u64;
        self.total_workflow_duration_ms += duration_ms;
        self.avg_workflow_duration_ms =
            self.total_workflow_duration_ms as f64 / self.workflows_executed as f64;
    }

    /// Records a task starting execution.
    pub fn record_task_started(&mut self) {
        self.tasks_in_progress += 1;
    }

    /// Records a task completing successfully.
    pub fn record_task_completed(&mut self, duration: Duration) {
        if self.tasks_in_progress > 0 {
            self.tasks_in_progress -= 1;
        }
        self.tasks_executed += 1;
        let duration_ms = duration.as_millis() as u64;
        self.total_task_duration_ms += duration_ms;
        self.avg_task_duration_ms = self.total_task_duration_ms as f64 / self.tasks_executed as f64;
    }

    /// Records a task failing.
    pub fn record_task_failed(&mut self, duration: Duration) {
        if self.tasks_in_progress > 0 {
            self.tasks_in_progress -= 1;
        }
        self.task_errors += 1;
        self.tasks_executed += 1;
        let duration_ms = duration.as_millis() as u64;
        self.total_task_duration_ms += duration_ms;
        self.avg_task_duration_ms = self.total_task_duration_ms as f64 / self.tasks_executed as f64;
    }

    /// Updates the uptime value.
    pub fn update_uptime(&mut self, uptime: Duration) {
        self.uptime = uptime;
    }

    /// Returns the total error count (workflows + tasks).
    pub fn total_errors(&self) -> u64 {
        self.workflow_errors + self.task_errors
    }

    /// Returns the total executions count (workflows + tasks).
    pub fn total_executions(&self) -> u64 {
        self.workflows_executed + self.tasks_executed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_default() {
        let metrics = WorkerMetrics::default();
        assert_eq!(metrics.workflows_executed, 0);
        assert_eq!(metrics.workflows_in_progress, 0);
        assert_eq!(metrics.tasks_executed, 0);
        assert_eq!(metrics.tasks_in_progress, 0);
        assert_eq!(metrics.workflow_errors, 0);
        assert_eq!(metrics.task_errors, 0);
        assert_eq!(metrics.avg_workflow_duration_ms, 0.0);
        assert_eq!(metrics.avg_task_duration_ms, 0.0);
        assert_eq!(metrics.uptime, Duration::ZERO);
    }

    #[test]
    fn test_record_workflow_completed() {
        let mut metrics = WorkerMetrics::new();
        metrics.record_workflow_started();
        assert_eq!(metrics.workflows_in_progress, 1);

        metrics.record_workflow_completed(Duration::from_millis(100));
        assert_eq!(metrics.workflows_executed, 1);
        assert_eq!(metrics.workflows_in_progress, 0);
        assert_eq!(metrics.avg_workflow_duration_ms, 100.0);
    }

    #[test]
    fn test_record_workflow_failed() {
        let mut metrics = WorkerMetrics::new();
        metrics.record_workflow_started();
        metrics.record_workflow_failed(Duration::from_millis(50));

        assert_eq!(metrics.workflows_executed, 1);
        assert_eq!(metrics.workflow_errors, 1);
        assert_eq!(metrics.workflows_in_progress, 0);
    }

    #[test]
    fn test_record_task_completed() {
        let mut metrics = WorkerMetrics::new();
        metrics.record_task_started();
        assert_eq!(metrics.tasks_in_progress, 1);

        metrics.record_task_completed(Duration::from_millis(200));
        assert_eq!(metrics.tasks_executed, 1);
        assert_eq!(metrics.tasks_in_progress, 0);
        assert_eq!(metrics.avg_task_duration_ms, 200.0);
    }

    #[test]
    fn test_record_task_failed() {
        let mut metrics = WorkerMetrics::new();
        metrics.record_task_started();
        metrics.record_task_failed(Duration::from_millis(30));

        assert_eq!(metrics.tasks_executed, 1);
        assert_eq!(metrics.task_errors, 1);
        assert_eq!(metrics.tasks_in_progress, 0);
    }

    #[test]
    fn test_average_calculation() {
        let mut metrics = WorkerMetrics::new();

        // Execute 3 workflows with different durations
        metrics.record_workflow_started();
        metrics.record_workflow_completed(Duration::from_millis(100));

        metrics.record_workflow_started();
        metrics.record_workflow_completed(Duration::from_millis(200));

        metrics.record_workflow_started();
        metrics.record_workflow_completed(Duration::from_millis(300));

        assert_eq!(metrics.workflows_executed, 3);
        // Average should be (100 + 200 + 300) / 3 = 200
        assert_eq!(metrics.avg_workflow_duration_ms, 200.0);
    }

    #[test]
    fn test_total_errors() {
        let mut metrics = WorkerMetrics::new();
        metrics.record_workflow_started();
        metrics.record_workflow_failed(Duration::from_millis(10));
        metrics.record_task_started();
        metrics.record_task_failed(Duration::from_millis(10));

        assert_eq!(metrics.total_errors(), 2);
    }

    #[test]
    fn test_total_executions() {
        let mut metrics = WorkerMetrics::new();
        metrics.record_workflow_started();
        metrics.record_workflow_completed(Duration::from_millis(10));
        metrics.record_task_started();
        metrics.record_task_completed(Duration::from_millis(10));

        assert_eq!(metrics.total_executions(), 2);
    }

    #[test]
    fn test_update_uptime() {
        let mut metrics = WorkerMetrics::new();
        metrics.update_uptime(Duration::from_secs(60));
        assert_eq!(metrics.uptime, Duration::from_secs(60));
    }
}
