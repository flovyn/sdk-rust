# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2024-XX-XX

### Added

- Initial release extracted from flovyn monorepo
- `WorkflowContext` API for deterministic workflow execution
  - `run_raw()` for caching side effects
  - `schedule_raw()` for task scheduling
  - `current_time_millis()`, `random_uuid()`, `random()` for deterministic operations
  - State management: `get_raw()`, `set_raw()`, `clear()`, `clear_all()`
  - Timers: `sleep()`
  - Signals: `promise()`
- `TaskContext` API for long-running tasks
  - Progress reporting
  - Heartbeat
  - Logging
  - Cancellation checking
- `WorkflowDefinition` trait for typed workflows
- `TaskDefinition` trait for typed tasks
- Testing utilities
  - `MockWorkflowContext` for unit testing
  - `MockTaskContext` for task testing
  - `TestWorkflowEnvironment` for integration testing
- Examples
  - hello-world: Minimal workflow
  - ecommerce: Saga pattern with compensation
  - data-pipeline: DAG pattern with parallel tasks
  - patterns: Timers, promises, child workflows, retry

### Proto Compatibility

- Compatible with flovyn server v0.1.x
