//! Command recording for workflow execution

use crate::error::DeterminismViolationError;
use crate::worker::determinism::DeterminismValidator;
use crate::workflow::command::WorkflowCommand;
use crate::workflow::event::ReplayEvent;

/// Trait for recording workflow commands during execution.
/// Implementations can validate commands against state machine rules.
pub trait CommandRecorder: Send + Sync {
    /// Record a command generated during workflow execution.
    /// Returns an error if the command violates determinism rules.
    fn record_command(&mut self, command: WorkflowCommand)
        -> Result<(), DeterminismViolationError>;

    /// Get all recorded commands
    fn get_commands(&self) -> Vec<WorkflowCommand>;

    /// Take all recorded commands (clears the internal list)
    fn take_commands(&mut self) -> Vec<WorkflowCommand>;

    /// Get the number of recorded commands
    fn command_count(&self) -> usize {
        self.get_commands().len()
    }
}

/// Simple command collector that records commands without validation.
/// Used for fresh workflow execution (no replay).
#[derive(Debug, Default)]
pub struct CommandCollector {
    commands: Vec<WorkflowCommand>,
}

impl CommandCollector {
    /// Create a new command collector
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

impl CommandRecorder for CommandCollector {
    fn record_command(
        &mut self,
        command: WorkflowCommand,
    ) -> Result<(), DeterminismViolationError> {
        self.commands.push(command);
        Ok(())
    }

    fn get_commands(&self) -> Vec<WorkflowCommand> {
        self.commands.clone()
    }

    fn take_commands(&mut self) -> Vec<WorkflowCommand> {
        std::mem::take(&mut self.commands)
    }

    fn command_count(&self) -> usize {
        self.commands.len()
    }
}

/// Validating command collector that validates each command against historical events.
/// Used for workflow replay to detect determinism violations.
#[derive(Debug)]
pub struct ValidatingCommandRecorder {
    validator: DeterminismValidator,
    existing_events: Vec<ReplayEvent>,
    commands: Vec<WorkflowCommand>,
}

impl ValidatingCommandRecorder {
    /// Create a new validating command recorder
    pub fn new(validator: DeterminismValidator, existing_events: Vec<ReplayEvent>) -> Self {
        Self {
            validator,
            existing_events,
            commands: Vec::new(),
        }
    }

    /// Get the existing events being validated against
    pub fn existing_events(&self) -> &[ReplayEvent] {
        &self.existing_events
    }
}

impl CommandRecorder for ValidatingCommandRecorder {
    fn record_command(
        &mut self,
        command: WorkflowCommand,
    ) -> Result<(), DeterminismViolationError> {
        // Find corresponding event for this command
        let event_index = (command.sequence_number() - 1) as usize;
        let event = self.existing_events.get(event_index);

        // Validate command against event (state machine rules)
        self.validator.validate_command(&command, event)?;

        // Validation passed - record command
        self.commands.push(command);
        Ok(())
    }

    fn get_commands(&self) -> Vec<WorkflowCommand> {
        self.commands.clone()
    }

    fn take_commands(&mut self) -> Vec<WorkflowCommand> {
        std::mem::take(&mut self.commands)
    }

    fn command_count(&self) -> usize {
        self.commands.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::event::EventType;
    use chrono::Utc;
    use serde_json::json;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn test_command_collector_new() {
        let collector = CommandCollector::new();
        assert_eq!(collector.command_count(), 0);
        assert!(collector.get_commands().is_empty());
    }

    #[test]
    fn test_command_collector_record() {
        let mut collector = CommandCollector::new();

        let cmd = WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: "test".to_string(),
            result: json!(42),
        };

        let result = collector.record_command(cmd.clone());
        assert!(result.is_ok());
        assert_eq!(collector.command_count(), 1);

        let commands = collector.get_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0], cmd);
    }

    #[test]
    fn test_command_collector_multiple_commands() {
        let mut collector = CommandCollector::new();

        collector
            .record_command(WorkflowCommand::RecordOperation {
                sequence_number: 1,
                operation_name: "step-1".to_string(),
                result: json!(1),
            })
            .unwrap();

        collector
            .record_command(WorkflowCommand::RecordOperation {
                sequence_number: 2,
                operation_name: "step-2".to_string(),
                result: json!(2),
            })
            .unwrap();

        collector
            .record_command(WorkflowCommand::CompleteWorkflow {
                sequence_number: 3,
                output: json!({"result": 3}),
            })
            .unwrap();

        assert_eq!(collector.command_count(), 3);
    }

    #[test]
    fn test_validating_recorder_records_valid_sequence() {
        let events = vec![
            ReplayEvent::new(
                1,
                EventType::OperationCompleted,
                json!({"operationName": "step-1", "result": 10}),
                now(),
            ),
            ReplayEvent::new(
                2,
                EventType::OperationCompleted,
                json!({"operationName": "step-2", "result": 20}),
                now(),
            ),
        ];

        let mut recorder = ValidatingCommandRecorder::new(DeterminismValidator::new(), events);

        // Record matching commands
        let result = recorder.record_command(WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: "step-1".to_string(),
            result: json!(10),
        });
        assert!(result.is_ok());

        let result = recorder.record_command(WorkflowCommand::RecordOperation {
            sequence_number: 2,
            operation_name: "step-2".to_string(),
            result: json!(20),
        });
        assert!(result.is_ok());

        assert_eq!(recorder.command_count(), 2);
    }

    #[test]
    fn test_validating_recorder_rejects_invalid_command() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            json!({"operationName": "expected-op", "result": {}}),
            now(),
        )];

        let mut recorder = ValidatingCommandRecorder::new(DeterminismValidator::new(), events);

        // Try to record mismatched command - should fail
        let result = recorder.record_command(WorkflowCommand::RecordOperation {
            sequence_number: 1,
            operation_name: "wrong-op".to_string(),
            result: json!({}),
        });

        assert!(result.is_err());
        assert_eq!(recorder.command_count(), 0); // No commands recorded
    }

    #[test]
    fn test_validating_recorder_allows_new_commands_beyond_history() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            json!({"operationName": "step-1", "result": 10}),
            now(),
        )];

        let mut recorder = ValidatingCommandRecorder::new(DeterminismValidator::new(), events);

        // First command matches history
        recorder
            .record_command(WorkflowCommand::RecordOperation {
                sequence_number: 1,
                operation_name: "step-1".to_string(),
                result: json!(10),
            })
            .unwrap();

        // Second command is beyond history - should be allowed (new command)
        let result = recorder.record_command(WorkflowCommand::RecordOperation {
            sequence_number: 2,
            operation_name: "step-2".to_string(),
            result: json!(20),
        });

        assert!(result.is_ok());
        assert_eq!(recorder.command_count(), 2);
    }

    #[test]
    fn test_validating_recorder_rejects_type_mismatch() {
        let events = vec![ReplayEvent::new(
            1,
            EventType::OperationCompleted,
            json!({"operationName": "op", "result": {}}),
            now(),
        )];

        let mut recorder = ValidatingCommandRecorder::new(DeterminismValidator::new(), events);

        // Try to record a different command type - should fail
        let result = recorder.record_command(WorkflowCommand::SetState {
            sequence_number: 1,
            key: "key".to_string(),
            value: json!({}),
        });

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeterminismViolationError::TypeMismatch { .. }
        ));
    }

    #[test]
    fn test_validating_recorder_existing_events_accessor() {
        let events = vec![
            ReplayEvent::new(1, EventType::OperationCompleted, json!({}), now()),
            ReplayEvent::new(2, EventType::OperationCompleted, json!({}), now()),
        ];

        let recorder = ValidatingCommandRecorder::new(DeterminismValidator::new(), events.clone());

        assert_eq!(recorder.existing_events().len(), 2);
        assert_eq!(recorder.existing_events()[0].sequence_number(), 1);
        assert_eq!(recorder.existing_events()[1].sequence_number(), 2);
    }
}
