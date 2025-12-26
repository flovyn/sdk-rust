//! OAuth2 client credentials flow for authentication.
//!
//! This module provides OAuth2 token fetching using the client credentials grant type.
//! It can be used by both the Rust SDK and FFI bindings.
//!
//! # Example
//!
//! ```ignore
//! use flovyn_sdk_core::client::oauth2::{OAuth2Credentials, fetch_access_token};
//!
//! let credentials = OAuth2Credentials::new(
//!     "my-client-id",
//!     "my-client-secret",
//!     "https://idp.example.com/token"
//! );
//!
//! let token = fetch_access_token(&credentials).await?;
//! ```

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// OAuth2 client credentials configuration
#[derive(Clone)]
pub struct OAuth2Credentials {
    /// OAuth2 client ID
    pub client_id: String,
    /// OAuth2 client secret
    pub client_secret: String,
    /// Token endpoint URL
    pub token_endpoint: String,
    /// Optional additional scopes (defaults to none)
    pub scopes: Option<Vec<String>>,
}

impl OAuth2Credentials {
    /// Create new OAuth2 credentials
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        token_endpoint: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            token_endpoint: token_endpoint.into(),
            scopes: None,
        }
    }

    /// Add scopes to the token request
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = Some(scopes);
        self
    }
}

impl std::fmt::Debug for OAuth2Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Credentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("token_endpoint", &self.token_endpoint)
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// OAuth2 token response
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    /// The access token (JWT)
    pub access_token: String,
    /// Token type (usually "Bearer")
    pub token_type: String,
    /// Token expiry in seconds (optional)
    pub expires_in: Option<u64>,
    /// Refresh token (optional, not used in client credentials flow)
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Scopes granted (optional)
    #[serde(default)]
    pub scope: Option<String>,
}

/// OAuth2 error response
#[derive(Debug, Deserialize)]
pub struct TokenError {
    /// Error code
    pub error: String,
    /// Error description
    pub error_description: Option<String>,
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(desc) = &self.error_description {
            write!(f, "{}: {}", self.error, desc)
        } else {
            write!(f, "{}", self.error)
        }
    }
}

impl std::error::Error for TokenError {}

/// Token request body for client credentials flow
#[derive(Serialize)]
struct TokenRequest<'a> {
    grant_type: &'static str,
    client_id: &'a str,
    client_secret: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

/// Cached access token with expiry
#[derive(Debug, Clone)]
pub struct CachedToken {
    /// The access token
    pub access_token: String,
    /// When the token was obtained
    pub obtained_at: Instant,
    /// Token lifetime in seconds
    pub expires_in: Option<u64>,
}

impl CachedToken {
    /// Check if the token is expired (with 30 second buffer)
    pub fn is_expired(&self) -> bool {
        if let Some(expires_in) = self.expires_in {
            let buffer = Duration::from_secs(30);
            let elapsed = self.obtained_at.elapsed();
            elapsed + buffer > Duration::from_secs(expires_in)
        } else {
            // No expiry info, assume valid
            false
        }
    }
}

/// Fetch an access token using OAuth2 client credentials flow
///
/// # Arguments
///
/// * `credentials` - OAuth2 client credentials
///
/// # Returns
///
/// The token response on success, or an error message on failure.
pub async fn fetch_access_token(credentials: &OAuth2Credentials) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();

    let scope = credentials.scopes.as_ref().map(|s| s.join(" "));

    let request_body = TokenRequest {
        grant_type: "client_credentials",
        client_id: &credentials.client_id,
        client_secret: &credentials.client_secret,
        scope,
    };

    debug!(
        "Fetching OAuth2 token from {} for client {}",
        credentials.token_endpoint, credentials.client_id
    );

    let response = client
        .post(&credentials.token_endpoint)
        .form(&request_body)
        .send()
        .await
        .map_err(|e| format!("Failed to send token request: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    if status.is_success() {
        let token_response: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse token response: {}", e))?;
        debug!(
            "Successfully obtained OAuth2 token (expires_in: {:?})",
            token_response.expires_in
        );
        Ok(token_response)
    } else {
        // Try to parse error response
        if let Ok(error) = serde_json::from_str::<TokenError>(&body) {
            warn!("OAuth2 token request failed: {}", error);
            Err(format!("OAuth2 token request failed: {}", error))
        } else {
            warn!(
                "OAuth2 token request failed with status {}: {}",
                status, body
            );
            Err(format!(
                "OAuth2 token request failed with status {}: {}",
                status, body
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth2_credentials_new() {
        let creds =
            OAuth2Credentials::new("my-client", "my-secret", "https://idp.example.com/token");
        assert_eq!(creds.client_id, "my-client");
        assert_eq!(creds.client_secret, "my-secret");
        assert_eq!(creds.token_endpoint, "https://idp.example.com/token");
        assert!(creds.scopes.is_none());
    }

    #[test]
    fn test_oauth2_credentials_with_scopes() {
        let creds = OAuth2Credentials::new("client", "secret", "https://example.com/token")
            .with_scopes(vec!["openid".to_string(), "profile".to_string()]);
        assert_eq!(
            creds.scopes,
            Some(vec!["openid".to_string(), "profile".to_string()])
        );
    }

    #[test]
    fn test_oauth2_credentials_debug_redacts_secret() {
        let creds =
            OAuth2Credentials::new("client", "super-secret-value", "https://example.com/token");
        let debug_str = format!("{:?}", creds);
        assert!(debug_str.contains("client"));
        assert!(debug_str.contains("[REDACTED]"));
        assert!(!debug_str.contains("super-secret-value"));
    }

    #[test]
    fn test_cached_token_not_expired() {
        let token = CachedToken {
            access_token: "test-token".to_string(),
            obtained_at: Instant::now(),
            expires_in: Some(3600), // 1 hour
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn test_cached_token_no_expiry() {
        let token = CachedToken {
            access_token: "test-token".to_string(),
            obtained_at: Instant::now(),
            expires_in: None,
        };
        assert!(!token.is_expired());
    }

    #[test]
    fn test_token_error_display() {
        let error = TokenError {
            error: "invalid_client".to_string(),
            error_description: Some("Client authentication failed".to_string()),
        };
        assert_eq!(
            format!("{}", error),
            "invalid_client: Client authentication failed"
        );

        let error_no_desc = TokenError {
            error: "invalid_grant".to_string(),
            error_description: None,
        };
        assert_eq!(format!("{}", error_no_desc), "invalid_grant");
    }

    #[test]
    fn test_token_response_deserialize() {
        let json = r#"{
            "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
            "token_type": "Bearer",
            "expires_in": 300
        }"#;

        let response: TokenResponse = serde_json::from_str(json).unwrap();
        assert!(response.access_token.starts_with("eyJ"));
        assert_eq!(response.token_type, "Bearer");
        assert_eq!(response.expires_in, Some(300));
        assert!(response.refresh_token.is_none());
    }

    #[test]
    fn test_token_response_deserialize_with_optional_fields() {
        let json = r#"{
            "access_token": "token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "refresh",
            "scope": "openid profile"
        }"#;

        let response: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.access_token, "token");
        assert_eq!(response.expires_in, Some(3600));
        assert_eq!(response.refresh_token, Some("refresh".to_string()));
        assert_eq!(response.scope, Some("openid profile".to_string()));
    }
}
