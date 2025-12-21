//! Authentication interceptor for gRPC requests
//!
//! Provides worker token authentication for all gRPC calls to the Flovyn server.

use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::{Request, Status};
use tracing::debug;

/// Interceptor that adds worker token authentication to gRPC requests.
///
/// This interceptor adds an `Authorization: Bearer <token>` header to all
/// outgoing gRPC requests.
///
/// # Example
///
/// ```ignore
/// let interceptor = WorkerTokenInterceptor::new("fwt_abc123...");
/// let client = WorkflowDispatchClient::with_interceptor(channel, interceptor);
/// ```
#[derive(Clone)]
pub struct WorkerTokenInterceptor {
    token: String,
}

impl WorkerTokenInterceptor {
    /// Create a new worker token interceptor.
    ///
    /// # Arguments
    ///
    /// * `token` - The worker token (must start with "fwt_" prefix)
    ///
    /// # Panics
    ///
    /// Panics if the token does not start with "fwt_" prefix.
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        assert!(
            token.starts_with("fwt_"),
            "Worker token must start with 'fwt_' prefix"
        );
        Self { token }
    }

    /// Get the authorization header value.
    fn authorization_value(&self) -> MetadataValue<tonic::metadata::Ascii> {
        format!("Bearer {}", self.token)
            .parse()
            .expect("Invalid authorization header value")
    }
}

impl Interceptor for WorkerTokenInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        request
            .metadata_mut()
            .insert("authorization", self.authorization_value());
        debug!("Attached worker token to gRPC request");
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interceptor_creation() {
        let interceptor = WorkerTokenInterceptor::new("fwt_test123");
        assert_eq!(interceptor.token, "fwt_test123");
    }

    #[test]
    #[should_panic(expected = "Worker token must start with 'fwt_' prefix")]
    fn test_interceptor_invalid_prefix() {
        WorkerTokenInterceptor::new("invalid_token");
    }

    #[test]
    fn test_authorization_value() {
        let interceptor = WorkerTokenInterceptor::new("fwt_test123");
        let value = interceptor.authorization_value();
        assert_eq!(value.to_str().unwrap(), "Bearer fwt_test123");
    }

    #[test]
    fn test_interceptor_call() {
        let mut interceptor = WorkerTokenInterceptor::new("fwt_test123");
        let request = Request::new(());
        let result = interceptor.call(request);
        assert!(result.is_ok());
        let request = result.unwrap();
        let auth = request.metadata().get("authorization").unwrap();
        assert_eq!(auth.to_str().unwrap(), "Bearer fwt_test123");
    }
}
