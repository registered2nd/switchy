//! Provider Adapter Trait
//!
//! Common interface for provider adapters, abstracting how each upstream provider is handled.

use super::auth::AuthInfo;
use crate::provider::Provider;
use crate::proxy::error::ProxyError;
use serde_json::Value;

/// Provider adapter trait
///
/// Every provider adapter implements this trait, giving one interface for:
/// - URL building
/// - Auth extraction and header injection
/// - Request/response format conversion (optional)
pub trait ProviderAdapter: Send + Sync {
    /// Adapter name (for logging and debugging)
    fn name(&self) -> &'static str;

    /// Extracts base_url from the provider config
    fn extract_base_url(&self, provider: &Provider) -> Result<String, ProxyError>;

    /// Extracts auth info from the provider config
    fn extract_auth(&self, provider: &Provider) -> Option<AuthInfo>;

    /// Builds the request URL
    fn build_url(&self, base_url: &str, endpoint: &str) -> String;

    /// Return auth headers as `(name, value)` pairs.
    ///
    /// The forwarder inserts these at the position of the original auth header
    /// so that header order is preserved.
    fn get_auth_headers(&self, auth: &AuthInfo) -> Vec<(http::HeaderName, http::HeaderValue)>;

    /// Whether format conversion is needed
    fn needs_transform(&self, _provider: &Provider) -> bool {
        false
    }

    /// Converts the request body
    fn transform_request(&self, body: Value, _provider: &Provider) -> Result<Value, ProxyError> {
        Ok(body)
    }

    /// Converts the response body
    #[allow(dead_code)]
    fn transform_response(&self, body: Value) -> Result<Value, ProxyError> {
        Ok(body)
    }
}
