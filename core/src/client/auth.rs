//! Authentication interceptor for gRPC requests
//!
//! Provides authentication for all gRPC calls to the Flovyn server.
//! Supports both worker tokens (fwt_/fct_ prefix) and static API keys.

use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::{Request, Status};
use tracing::debug;

/// Interceptor that adds authentication to gRPC requests.
///
/// This interceptor adds an `Authorization: Bearer <token>` header to all
/// outgoing gRPC requests. It supports both worker tokens (with prefix validation)
/// and static API keys (no prefix requirement).
///
/// # Example
///
/// ```ignore
/// // Using worker token (validates fwt_/fct_ prefix)
/// let interceptor = AuthInterceptor::worker_token("fwt_abc123...");
///
/// // Using static API key (no prefix validation)
/// let interceptor = AuthInterceptor::api_key("my-api-key-xyz");
///
/// let client = WorkflowDispatchClient::with_interceptor(channel, interceptor);
/// ```
#[derive(Clone)]
pub struct AuthInterceptor {
    token: String,
}

impl AuthInterceptor {
    /// Create a new auth interceptor with a worker token.
    ///
    /// # Arguments
    ///
    /// * `token` - The worker token (must start with "fwt_" or "fct_" prefix)
    ///
    /// # Panics
    ///
    /// Panics if the token does not start with "fwt_" or "fct_" prefix.
    pub fn worker_token(token: impl Into<String>) -> Self {
        let token = token.into();
        assert!(
            token.starts_with("fwt_") || token.starts_with("fct_"),
            "Worker token must start with 'fwt_' or 'fct_' prefix"
        );
        Self { token }
    }

    /// Create a new auth interceptor with a static API key.
    ///
    /// Unlike `worker_token()`, this method accepts any key without prefix validation.
    /// Use this when the server is configured with static API keys.
    ///
    /// # Arguments
    ///
    /// * `key` - The API key (any format accepted)
    pub fn api_key(key: impl Into<String>) -> Self {
        Self { token: key.into() }
    }

    /// Get the authorization header value.
    fn authorization_value(&self) -> MetadataValue<tonic::metadata::Ascii> {
        format!("Bearer {}", self.token)
            .parse()
            .expect("Invalid authorization header value")
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        request
            .metadata_mut()
            .insert("authorization", self.authorization_value());
        debug!("Attached auth token to gRPC request");
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_token_creation() {
        let interceptor = AuthInterceptor::worker_token("fwt_test123");
        assert_eq!(interceptor.token, "fwt_test123");
    }

    #[test]
    #[should_panic(expected = "Worker token must start with 'fwt_' or 'fct_' prefix")]
    fn test_worker_token_invalid_prefix() {
        AuthInterceptor::worker_token("invalid_token");
    }

    #[test]
    fn test_worker_token_client_token() {
        let interceptor = AuthInterceptor::worker_token("fct_client123");
        assert_eq!(interceptor.token, "fct_client123");
    }

    #[test]
    fn test_api_key_creation() {
        let interceptor = AuthInterceptor::api_key("my-static-api-key");
        assert_eq!(interceptor.token, "my-static-api-key");
    }

    #[test]
    fn test_api_key_accepts_any_format() {
        // API key accepts any format without validation
        let interceptor = AuthInterceptor::api_key("flovyn_wk_live_xyz123");
        assert_eq!(interceptor.token, "flovyn_wk_live_xyz123");

        let interceptor = AuthInterceptor::api_key("simple-key");
        assert_eq!(interceptor.token, "simple-key");

        let interceptor = AuthInterceptor::api_key("uuid-like-550e8400-e29b-41d4");
        assert_eq!(interceptor.token, "uuid-like-550e8400-e29b-41d4");
    }

    #[test]
    fn test_authorization_value() {
        let interceptor = AuthInterceptor::worker_token("fwt_test123");
        let value = interceptor.authorization_value();
        assert_eq!(value.to_str().unwrap(), "Bearer fwt_test123");
    }

    #[test]
    fn test_api_key_authorization_value() {
        let interceptor = AuthInterceptor::api_key("my-api-key");
        let value = interceptor.authorization_value();
        assert_eq!(value.to_str().unwrap(), "Bearer my-api-key");
    }

    #[test]
    fn test_interceptor_call() {
        let mut interceptor = AuthInterceptor::worker_token("fwt_test123");
        let request = Request::new(());
        let result = interceptor.call(request);
        assert!(result.is_ok());
        let request = result.unwrap();
        let auth = request.metadata().get("authorization").unwrap();
        assert_eq!(auth.to_str().unwrap(), "Bearer fwt_test123");
    }

    #[test]
    fn test_api_key_interceptor_call() {
        let mut interceptor = AuthInterceptor::api_key("static-key-abc");
        let request = Request::new(());
        let result = interceptor.call(request);
        assert!(result.is_ok());
        let request = result.unwrap();
        let auth = request.metadata().get("authorization").unwrap();
        assert_eq!(auth.to_str().unwrap(), "Bearer static-key-abc");
    }
}
