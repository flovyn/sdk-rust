//! Authentication interceptor for gRPC requests
//!
//! Provides authentication for all gRPC calls to the Flovyn server.

use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::{Request, Status};
use tracing::debug;

/// Interceptor that adds authentication to gRPC requests.
///
/// This interceptor adds an `Authorization: Bearer <token>` header to all
/// outgoing gRPC requests.
///
/// # Example
///
/// ```ignore
/// let interceptor = AuthInterceptor::worker_token("my-token");
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
    /// * `token` - The worker token or API key
    pub fn worker_token(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
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
        let interceptor = AuthInterceptor::worker_token("test_token_123");
        assert_eq!(interceptor.token, "test_token_123");
    }

    #[test]
    fn test_worker_token_accepts_any_format() {
        // worker_token accepts any format without validation
        let interceptor = AuthInterceptor::worker_token("flovyn_wk_live_xyz123");
        assert_eq!(interceptor.token, "flovyn_wk_live_xyz123");

        let interceptor = AuthInterceptor::worker_token("simple-key");
        assert_eq!(interceptor.token, "simple-key");
    }

    #[test]
    fn test_authorization_value() {
        let interceptor = AuthInterceptor::worker_token("my_token");
        let value = interceptor.authorization_value();
        assert_eq!(value.to_str().unwrap(), "Bearer my_token");
    }

    #[test]
    fn test_interceptor_call() {
        let mut interceptor = AuthInterceptor::worker_token("my_token");
        let request = Request::new(());
        let result = interceptor.call(request);
        assert!(result.is_ok());
        let request = result.unwrap();
        let auth = request.metadata().get("authorization").unwrap();
        assert_eq!(auth.to_str().unwrap(), "Bearer my_token");
    }
}
