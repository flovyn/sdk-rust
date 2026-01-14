//! Configuration presets for Flovyn client
//!
//! This module provides configuration options for workflow and task execution,
//! with sensible defaults and presets for common use cases.

use std::time::Duration;

/// Configuration for workflow execution
#[derive(Debug, Clone)]
pub struct WorkflowExecutorConfig {
    /// Maximum number of workflows executing concurrently
    pub max_concurrent: usize,
    /// Size of the internal workflow queue
    pub queue_size: usize,
    /// Timeout for polling workflows from server
    pub poll_timeout: Duration,
    /// Duration to keep idle threads alive
    pub keep_alive_time: Duration,
}

impl Default for WorkflowExecutorConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl WorkflowExecutorConfig {
    /// Default configuration suitable for most use cases
    pub const DEFAULT: Self = Self {
        max_concurrent: 10,
        queue_size: 100,
        poll_timeout: Duration::from_secs(60),
        keep_alive_time: Duration::from_secs(60),
    };

    /// High-throughput configuration for heavy workloads
    pub const HIGH_THROUGHPUT: Self = Self {
        max_concurrent: 50,
        queue_size: 500,
        poll_timeout: Duration::from_secs(30),
        keep_alive_time: Duration::from_secs(60),
    };

    /// Low-resource configuration for constrained environments
    pub const LOW_RESOURCE: Self = Self {
        max_concurrent: 2,
        queue_size: 20,
        poll_timeout: Duration::from_secs(60),
        keep_alive_time: Duration::from_secs(60),
    };

    /// Create a new configuration with validation
    pub fn new(
        max_concurrent: usize,
        queue_size: usize,
        poll_timeout: Duration,
        keep_alive_time: Duration,
    ) -> Result<Self, ConfigError> {
        if max_concurrent == 0 {
            return Err(ConfigError::InvalidValue(
                "max_concurrent must be positive".to_string(),
            ));
        }
        if queue_size == 0 {
            return Err(ConfigError::InvalidValue(
                "queue_size must be positive".to_string(),
            ));
        }
        if poll_timeout.is_zero() {
            return Err(ConfigError::InvalidValue(
                "poll_timeout must be positive".to_string(),
            ));
        }

        Ok(Self {
            max_concurrent,
            queue_size,
            poll_timeout,
            keep_alive_time,
        })
    }
}

/// Configuration for task execution
#[derive(Debug, Clone)]
pub struct TaskExecutorConfig {
    /// Maximum number of tasks executing concurrently
    pub max_concurrent: usize,
    /// Default timeout for task execution
    pub default_timeout: Duration,
    /// Maximum number of retry attempts for failed tasks
    pub max_retries: u32,
    /// Interval between heartbeats
    pub heartbeat_interval: Duration,
    /// Timeout for polling tasks from server
    pub poll_timeout: Duration,
}

impl Default for TaskExecutorConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TaskExecutorConfig {
    /// Default configuration suitable for most use cases
    pub const DEFAULT: Self = Self {
        max_concurrent: 20,
        default_timeout: Duration::from_secs(600), // 10 minutes
        max_retries: 3,
        heartbeat_interval: Duration::from_secs(30),
        poll_timeout: Duration::from_secs(60),
    };

    /// High-throughput configuration for heavy task workloads
    pub const HIGH_THROUGHPUT: Self = Self {
        max_concurrent: 100,
        default_timeout: Duration::from_secs(300), // 5 minutes
        max_retries: 2,
        heartbeat_interval: Duration::from_secs(15),
        poll_timeout: Duration::from_secs(30),
    };

    /// Long-running configuration for tasks that take significant time
    pub const LONG_RUNNING: Self = Self {
        max_concurrent: 5,
        default_timeout: Duration::from_secs(3600), // 60 minutes
        max_retries: 1,
        heartbeat_interval: Duration::from_secs(60),
        poll_timeout: Duration::from_secs(60),
    };

    /// Low-resource configuration for constrained environments
    pub const LOW_RESOURCE: Self = Self {
        max_concurrent: 5,
        default_timeout: Duration::from_secs(600), // 10 minutes
        max_retries: 3,
        heartbeat_interval: Duration::from_secs(45),
        poll_timeout: Duration::from_secs(60),
    };

    /// Create a new configuration with validation
    pub fn new(
        max_concurrent: usize,
        default_timeout: Duration,
        max_retries: u32,
        heartbeat_interval: Duration,
        poll_timeout: Duration,
    ) -> Result<Self, ConfigError> {
        if max_concurrent == 0 {
            return Err(ConfigError::InvalidValue(
                "max_concurrent must be positive".to_string(),
            ));
        }
        if poll_timeout.is_zero() {
            return Err(ConfigError::InvalidValue(
                "poll_timeout must be positive".to_string(),
            ));
        }

        Ok(Self {
            max_concurrent,
            default_timeout,
            max_retries,
            heartbeat_interval,
            poll_timeout,
        })
    }
}

/// Complete configuration for FlovynClient
#[derive(Debug, Clone)]
pub struct FlovynClientConfig {
    /// Configuration for workflow execution
    pub workflow_config: WorkflowExecutorConfig,
    /// Configuration for task execution
    pub task_config: TaskExecutorConfig,
    /// Labels for worker identification and task routing
    pub worker_labels: std::collections::HashMap<String, String>,
}

impl Default for FlovynClientConfig {
    fn default() -> Self {
        Self {
            workflow_config: WorkflowExecutorConfig::DEFAULT,
            task_config: TaskExecutorConfig::DEFAULT,
            worker_labels: std::collections::HashMap::new(),
        }
    }
}

impl FlovynClientConfig {
    /// High-throughput configuration for production workloads
    pub fn high_throughput() -> Self {
        Self {
            workflow_config: WorkflowExecutorConfig::HIGH_THROUGHPUT,
            task_config: TaskExecutorConfig::HIGH_THROUGHPUT,
            worker_labels: std::collections::HashMap::new(),
        }
    }

    /// Low-resource configuration for development or constrained environments
    pub fn low_resource() -> Self {
        Self {
            workflow_config: WorkflowExecutorConfig::LOW_RESOURCE,
            task_config: TaskExecutorConfig::LOW_RESOURCE,
            worker_labels: std::collections::HashMap::new(),
        }
    }

    /// Create a new configuration with custom settings
    pub fn new(
        workflow_config: WorkflowExecutorConfig,
        task_config: TaskExecutorConfig,
        worker_labels: std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            workflow_config,
            task_config,
            worker_labels,
        }
    }

    /// Set workflow configuration
    pub fn with_workflow_config(mut self, config: WorkflowExecutorConfig) -> Self {
        self.workflow_config = config;
        self
    }

    /// Set task configuration
    pub fn with_task_config(mut self, config: TaskExecutorConfig) -> Self {
        self.task_config = config;
        self
    }

    /// Set worker labels
    pub fn with_worker_labels(mut self, labels: std::collections::HashMap<String, String>) -> Self {
        self.worker_labels = labels;
        self
    }
}

/// Configuration error
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConfigError {
    /// Invalid configuration value
    #[error("Invalid configuration value: {0}")]
    InvalidValue(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_config_default() {
        let config = WorkflowExecutorConfig::default();
        assert_eq!(config.max_concurrent, 10);
        assert_eq!(config.queue_size, 100);
        assert_eq!(config.poll_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_workflow_config_high_throughput() {
        let config = WorkflowExecutorConfig::HIGH_THROUGHPUT;
        assert_eq!(config.max_concurrent, 50);
        assert_eq!(config.queue_size, 500);
        assert_eq!(config.poll_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_workflow_config_low_resource() {
        let config = WorkflowExecutorConfig::LOW_RESOURCE;
        assert_eq!(config.max_concurrent, 2);
        assert_eq!(config.queue_size, 20);
    }

    #[test]
    fn test_workflow_config_new_validation() {
        let result =
            WorkflowExecutorConfig::new(0, 100, Duration::from_secs(60), Duration::from_secs(60));
        assert!(result.is_err());

        let result =
            WorkflowExecutorConfig::new(10, 0, Duration::from_secs(60), Duration::from_secs(60));
        assert!(result.is_err());

        let result = WorkflowExecutorConfig::new(10, 100, Duration::ZERO, Duration::from_secs(60));
        assert!(result.is_err());

        let result =
            WorkflowExecutorConfig::new(10, 100, Duration::from_secs(60), Duration::from_secs(60));
        assert!(result.is_ok());
    }

    #[test]
    fn test_task_config_default() {
        let config = TaskExecutorConfig::default();
        assert_eq!(config.max_concurrent, 20);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
    }

    #[test]
    fn test_task_config_high_throughput() {
        let config = TaskExecutorConfig::HIGH_THROUGHPUT;
        assert_eq!(config.max_concurrent, 100);
        assert_eq!(config.max_retries, 2);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(15));
    }

    #[test]
    fn test_task_config_long_running() {
        let config = TaskExecutorConfig::LONG_RUNNING;
        assert_eq!(config.max_concurrent, 5);
        assert_eq!(config.default_timeout, Duration::from_secs(3600));
        assert_eq!(config.max_retries, 1);
    }

    #[test]
    fn test_client_config_default() {
        let config = FlovynClientConfig::default();
        assert_eq!(config.workflow_config.max_concurrent, 10);
        assert_eq!(config.task_config.max_concurrent, 20);
        assert!(config.worker_labels.is_empty());
    }

    #[test]
    fn test_client_config_high_throughput() {
        let config = FlovynClientConfig::high_throughput();
        assert_eq!(config.workflow_config.max_concurrent, 50);
        assert_eq!(config.task_config.max_concurrent, 100);
    }

    #[test]
    fn test_client_config_builder_pattern() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".to_string(), "production".to_string());

        let config = FlovynClientConfig::default()
            .with_workflow_config(WorkflowExecutorConfig::HIGH_THROUGHPUT)
            .with_worker_labels(labels.clone());

        assert_eq!(config.workflow_config.max_concurrent, 50);
        assert_eq!(
            config.worker_labels.get("env"),
            Some(&"production".to_string())
        );
    }
}
