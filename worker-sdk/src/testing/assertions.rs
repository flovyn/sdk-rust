//! Assert helpers for workflow and task testing.

use crate::error::{FlovynError, Result};
use serde_json::Value;

/// Assert that a workflow result is successful.
///
/// # Panics
///
/// Panics if the result is an error.
pub fn assert_workflow_completed(result: &Result<Value>) {
    match result {
        Ok(_) => {}
        Err(e) => panic!(
            "Expected workflow to complete successfully, but got error: {}",
            e
        ),
    }
}

/// Assert that a workflow result is successful and matches the expected output.
///
/// # Panics
///
/// Panics if the result is an error or if the output doesn't match.
pub fn assert_workflow_completed_with(result: &Result<Value>, expected: &Value) {
    match result {
        Ok(actual) => {
            if actual != expected {
                panic!(
                    "Workflow output mismatch.\nExpected: {}\nActual: {}",
                    serde_json::to_string_pretty(expected).unwrap(),
                    serde_json::to_string_pretty(actual).unwrap()
                );
            }
        }
        Err(e) => panic!(
            "Expected workflow to complete successfully, but got error: {}",
            e
        ),
    }
}

/// Assert that a workflow result is a failure.
///
/// # Panics
///
/// Panics if the result is successful.
pub fn assert_workflow_failed(result: &Result<Value>) {
    if let Ok(output) = result {
        panic!(
            "Expected workflow to fail, but got success with output: {}",
            serde_json::to_string_pretty(output).unwrap()
        )
    }
}

/// Assert that a workflow result is a specific error type.
///
/// # Panics
///
/// Panics if the result is successful or if the error message doesn't contain the expected text.
pub fn assert_workflow_failed_with(result: &Result<Value>, expected_error_contains: &str) {
    match result {
        Ok(output) => panic!(
            "Expected workflow to fail with error containing '{}', but got success with output: {}",
            expected_error_contains,
            serde_json::to_string_pretty(output).unwrap()
        ),
        Err(e) => {
            let error_msg = e.to_string();
            if !error_msg.contains(expected_error_contains) {
                panic!(
                    "Expected error to contain '{}', but got: {}",
                    expected_error_contains, error_msg
                );
            }
        }
    }
}

/// Assert that a workflow was cancelled.
///
/// # Panics
///
/// Panics if the result is not a cancellation error.
pub fn assert_workflow_cancelled(result: &Result<Value>) {
    match result {
        Ok(output) => panic!(
            "Expected workflow to be cancelled, but got success with output: {}",
            serde_json::to_string_pretty(output).unwrap()
        ),
        Err(FlovynError::WorkflowCancelled(_)) => {}
        Err(e) => panic!(
            "Expected workflow to be cancelled, but got different error: {}",
            e
        ),
    }
}

/// Assert that a task result is successful.
///
/// # Panics
///
/// Panics if the result is an error.
pub fn assert_task_succeeded(result: &Result<Value>) {
    match result {
        Ok(_) => {}
        Err(e) => panic!("Expected task to succeed, but got error: {}", e),
    }
}

/// Assert that a task result is successful and matches the expected output.
///
/// # Panics
///
/// Panics if the result is an error or if the output doesn't match.
pub fn assert_task_succeeded_with(result: &Result<Value>, expected: &Value) {
    match result {
        Ok(actual) => {
            if actual != expected {
                panic!(
                    "Task output mismatch.\nExpected: {}\nActual: {}",
                    serde_json::to_string_pretty(expected).unwrap(),
                    serde_json::to_string_pretty(actual).unwrap()
                );
            }
        }
        Err(e) => panic!("Expected task to succeed, but got error: {}", e),
    }
}

/// Assert that a task result is a failure.
///
/// # Panics
///
/// Panics if the result is successful.
pub fn assert_task_failed(result: &Result<Value>) {
    if let Ok(output) = result {
        panic!(
            "Expected task to fail, but got success with output: {}",
            serde_json::to_string_pretty(output).unwrap()
        )
    }
}

/// Assert that a task result is a specific error type.
///
/// # Panics
///
/// Panics if the result is successful or if the error message doesn't contain the expected text.
pub fn assert_task_failed_with(result: &Result<Value>, expected_error_contains: &str) {
    match result {
        Ok(output) => panic!(
            "Expected task to fail with error containing '{}', but got success with output: {}",
            expected_error_contains,
            serde_json::to_string_pretty(output).unwrap()
        ),
        Err(e) => {
            let error_msg = e.to_string();
            if !error_msg.contains(expected_error_contains) {
                panic!(
                    "Expected error to contain '{}', but got: {}",
                    expected_error_contains, error_msg
                );
            }
        }
    }
}

/// Assert that a task was cancelled.
///
/// # Panics
///
/// Panics if the result is not a cancellation error.
pub fn assert_task_cancelled(result: &Result<Value>) {
    match result {
        Ok(output) => panic!(
            "Expected task to be cancelled, but got success with output: {}",
            serde_json::to_string_pretty(output).unwrap()
        ),
        Err(FlovynError::TaskCancelled) => {}
        Err(e) => panic!(
            "Expected task to be cancelled, but got different error: {}",
            e
        ),
    }
}

/// Assert that a value contains a specific key with the expected value.
///
/// # Panics
///
/// Panics if the key doesn't exist or the value doesn't match.
pub fn assert_json_has(value: &Value, key: &str, expected: &Value) {
    match value.get(key) {
        Some(actual) => {
            if actual != expected {
                panic!(
                    "Key '{}' has unexpected value.\nExpected: {}\nActual: {}",
                    key,
                    serde_json::to_string_pretty(expected).unwrap(),
                    serde_json::to_string_pretty(actual).unwrap()
                );
            }
        }
        None => panic!("Expected key '{}' not found in value: {}", key, value),
    }
}

/// Assert that a value contains a specific key.
///
/// # Panics
///
/// Panics if the key doesn't exist.
pub fn assert_json_has_key(value: &Value, key: &str) {
    if value.get(key).is_none() {
        panic!(
            "Expected key '{}' not found in value: {}",
            key,
            serde_json::to_string_pretty(value).unwrap()
        );
    }
}

/// Assert that a value does not contain a specific key.
///
/// # Panics
///
/// Panics if the key exists.
pub fn assert_json_missing_key(value: &Value, key: &str) {
    if value.get(key).is_some() {
        panic!(
            "Expected key '{}' to be absent, but found in value: {}",
            key,
            serde_json::to_string_pretty(value).unwrap()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_assert_workflow_completed_success() {
        let result: Result<Value> = Ok(json!({"status": "done"}));
        assert_workflow_completed(&result); // Should not panic
    }

    #[test]
    #[should_panic(expected = "Expected workflow to complete successfully")]
    fn test_assert_workflow_completed_failure() {
        let result: Result<Value> = Err(FlovynError::Other("test error".to_string()));
        assert_workflow_completed(&result);
    }

    #[test]
    fn test_assert_workflow_completed_with_success() {
        let result: Result<Value> = Ok(json!({"value": 42}));
        assert_workflow_completed_with(&result, &json!({"value": 42}));
    }

    #[test]
    #[should_panic(expected = "Workflow output mismatch")]
    fn test_assert_workflow_completed_with_mismatch() {
        let result: Result<Value> = Ok(json!({"value": 42}));
        assert_workflow_completed_with(&result, &json!({"value": 100}));
    }

    #[test]
    fn test_assert_workflow_failed_success() {
        let result: Result<Value> = Err(FlovynError::Other("error".to_string()));
        assert_workflow_failed(&result);
    }

    #[test]
    #[should_panic(expected = "Expected workflow to fail")]
    fn test_assert_workflow_failed_actually_succeeded() {
        let result: Result<Value> = Ok(json!({}));
        assert_workflow_failed(&result);
    }

    #[test]
    fn test_assert_workflow_failed_with_success() {
        let result: Result<Value> = Err(FlovynError::Other("specific error message".to_string()));
        assert_workflow_failed_with(&result, "specific error");
    }

    #[test]
    #[should_panic(expected = "Expected error to contain")]
    fn test_assert_workflow_failed_with_wrong_error() {
        let result: Result<Value> = Err(FlovynError::Other("different error".to_string()));
        assert_workflow_failed_with(&result, "specific error");
    }

    #[test]
    fn test_assert_workflow_cancelled_success() {
        let result: Result<Value> = Err(FlovynError::WorkflowCancelled("cancelled".to_string()));
        assert_workflow_cancelled(&result);
    }

    #[test]
    #[should_panic(expected = "Expected workflow to be cancelled")]
    fn test_assert_workflow_cancelled_not_cancelled() {
        let result: Result<Value> = Err(FlovynError::Other("error".to_string()));
        assert_workflow_cancelled(&result);
    }

    #[test]
    fn test_assert_task_succeeded_success() {
        let result: Result<Value> = Ok(json!({"done": true}));
        assert_task_succeeded(&result);
    }

    #[test]
    fn test_assert_task_succeeded_with_success() {
        let result: Result<Value> = Ok(json!({"count": 5}));
        assert_task_succeeded_with(&result, &json!({"count": 5}));
    }

    #[test]
    fn test_assert_task_failed_success() {
        let result: Result<Value> = Err(FlovynError::Other("task error".to_string()));
        assert_task_failed(&result);
    }

    #[test]
    fn test_assert_task_cancelled_success() {
        let result: Result<Value> = Err(FlovynError::TaskCancelled);
        assert_task_cancelled(&result);
    }

    #[test]
    fn test_assert_json_has_success() {
        let value = json!({"name": "test", "count": 42});
        assert_json_has(&value, "name", &json!("test"));
        assert_json_has(&value, "count", &json!(42));
    }

    #[test]
    #[should_panic(expected = "Expected key 'missing'")]
    fn test_assert_json_has_missing_key() {
        let value = json!({"name": "test"});
        assert_json_has(&value, "missing", &json!("value"));
    }

    #[test]
    #[should_panic(expected = "has unexpected value")]
    fn test_assert_json_has_wrong_value() {
        let value = json!({"name": "test"});
        assert_json_has(&value, "name", &json!("wrong"));
    }

    #[test]
    fn test_assert_json_has_key_success() {
        let value = json!({"name": "test"});
        assert_json_has_key(&value, "name");
    }

    #[test]
    #[should_panic(expected = "Expected key 'missing'")]
    fn test_assert_json_has_key_missing() {
        let value = json!({"name": "test"});
        assert_json_has_key(&value, "missing");
    }

    #[test]
    fn test_assert_json_missing_key_success() {
        let value = json!({"name": "test"});
        assert_json_missing_key(&value, "missing");
    }

    #[test]
    #[should_panic(expected = "Expected key 'name' to be absent")]
    fn test_assert_json_missing_key_present() {
        let value = json!({"name": "test"});
        assert_json_missing_key(&value, "name");
    }
}
