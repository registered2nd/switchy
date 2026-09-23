//! Gemini (Google) Provider Adapter
//!
//! Supports both API key and OAuth auth
//!
//! ## Auth modes
//! - **Gemini**: API key auth (x-goog-api-key)
//! - **GeminiCli**: OAuth Bearer auth (for Gemini CLI)

use super::{AuthInfo, AuthStrategy, ProviderAdapter, ProviderType};
use crate::provider::Provider;
use crate::proxy::error::ProxyError;

/// Gemini adapter
pub struct GeminiAdapter;

/// OAuth credentials
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OAuthCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[allow(dead_code)]
impl OAuthCredentials {
    /// Whether the token needs a refresh (refresh_token present but no valid access_token)
    pub fn needs_refresh(&self) -> bool {
        self.refresh_token.is_some() && self.access_token.is_empty()
    }

    /// Whether the token can be refreshed
    pub fn can_refresh(&self) -> bool {
        self.refresh_token.is_some() && self.client_id.is_some() && self.client_secret.is_some()
    }
}

impl GeminiAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Returns the provider type
    ///
    /// Detected from the API key format:
    /// - GeminiCli: an access_token (starting with ya29.) or JSON credentials
    /// - Gemini: a plain API key
    pub fn provider_type(&self, provider: &Provider) -> ProviderType {
        if let Some(key) = self.extract_key_raw(provider) {
            // OAuth access_tokens start with ya29.
            if key.starts_with("ya29.") {
                return ProviderType::GeminiCli;
            }
            // OAuth credentials as JSON
            if key.starts_with('{') {
                return ProviderType::GeminiCli;
            }
        }
        ProviderType::Gemini
    }

    /// Detects the auth type
    pub fn detect_auth_type(&self, provider: &Provider) -> AuthStrategy {
        match self.provider_type(provider) {
            ProviderType::GeminiCli => AuthStrategy::GoogleOAuth,
            _ => AuthStrategy::Google,
        }
    }

    /// Parses OAuth credentials
    pub fn parse_oauth_credentials(&self, key: &str) -> Option<OAuthCredentials> {
        // A bare access_token
        if key.starts_with("ya29.") {
            return Some(OAuthCredentials {
                access_token: key.to_string(),
                refresh_token: None,
                client_id: None,
                client_secret: None,
            });
        }

        // JSON format
        if key.starts_with('{') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(key) {
                let access_token = json
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let refresh_token = json
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let client_id = json
                    .get("client_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let client_secret = json
                    .get("client_secret")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                // Return credentials if there is an access_token or refresh_token
                if !access_token.is_empty() || refresh_token.is_some() {
                    return Some(OAuthCredentials {
                        access_token,
                        refresh_token,
                        client_id,
                        client_secret,
                    });
                }
            }
        }

        None
    }

    /// Extracts the raw API key from the provider config
    fn extract_key_raw(&self, provider: &Provider) -> Option<String> {
        if let Some(env) = provider.settings_config.get("env") {
            // Use GEMINI_API_KEY
            if let Some(key) = env.get("GEMINI_API_KEY").and_then(|v| v.as_str()) {
                return Some(key.to_string());
            }
        }

        // Try a top-level field
        if let Some(key) = provider
            .settings_config
            .get("apiKey")
            .or_else(|| provider.settings_config.get("api_key"))
            .and_then(|v| v.as_str())
        {
            return Some(key.to_string());
        }

        None
    }
}

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "Gemini"
    }

    fn extract_base_url(&self, provider: &Provider) -> Result<String, ProxyError> {
        // From env
        if let Some(env) = provider.settings_config.get("env") {
            if let Some(url) = env.get("GOOGLE_GEMINI_BASE_URL").and_then(|v| v.as_str()) {
                return Ok(url.trim_end_matches('/').to_string());
            }
        }

        // Try a top-level field
        if let Some(url) = provider
            .settings_config
            .get("base_url")
            .and_then(|v| v.as_str())
        {
            return Ok(url.trim_end_matches('/').to_string());
        }

        if let Some(url) = provider
            .settings_config
            .get("baseURL")
            .and_then(|v| v.as_str())
        {
            return Ok(url.trim_end_matches('/').to_string());
        }

        Err(ProxyError::ConfigError(
            "The Gemini provider has no base_url configured".to_string(),
        ))
    }

    fn extract_auth(&self, provider: &Provider) -> Option<AuthInfo> {
        let key = self.extract_key_raw(provider)?;
        let strategy = self.detect_auth_type(provider);

        match strategy {
            AuthStrategy::GoogleOAuth => {
                // Parse OAuth credentials
                if let Some(creds) = self.parse_oauth_credentials(&key) {
                    Some(AuthInfo::with_access_token(key, creds.access_token))
                } else {
                    // Fall back to a plain API key
                    Some(AuthInfo::new(key, AuthStrategy::Google))
                }
            }
            _ => Some(AuthInfo::new(key, AuthStrategy::Google)),
        }
    }

    fn build_url(&self, base_url: &str, endpoint: &str) -> String {
        let base_trimmed = base_url.trim_end_matches('/');
        let endpoint_trimmed = endpoint.trim_start_matches('/');

        let mut url = format!("{base_trimmed}/{endpoint_trimmed}");

        // De-duplicate the /v1beta path
        let version_patterns = ["/v1beta", "/v1"];
        for pattern in &version_patterns {
            let duplicate = format!("{pattern}{pattern}");
            if url.contains(&duplicate) {
                url = url.replace(&duplicate, pattern);
            }
        }

        url
    }

    fn get_auth_headers(&self, auth: &AuthInfo) -> Vec<(http::HeaderName, http::HeaderValue)> {
        use http::{HeaderName, HeaderValue};
        match auth.strategy {
            AuthStrategy::GoogleOAuth => {
                let token = auth.access_token.as_ref().unwrap_or(&auth.api_key);
                vec![
                    (
                        HeaderName::from_static("authorization"),
                        HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
                    ),
                    (
                        HeaderName::from_static("x-goog-api-client"),
                        HeaderValue::from_static("GeminiCLI/1.0"),
                    ),
                ]
            }
            _ => vec![(
                HeaderName::from_static("x-goog-api-key"),
                HeaderValue::from_str(&auth.api_key).unwrap(),
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_provider(config: serde_json::Value) -> Provider {
        Provider {
            id: "test".to_string(),
            name: "Test Gemini".to_string(),
            settings_config: config,
            website_url: None,
            category: Some("gemini".to_string()),
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    #[test]
    fn test_extract_base_url_from_env() {
        let adapter = GeminiAdapter::new();
        let provider = create_provider(json!({
            "env": {
                "GOOGLE_GEMINI_BASE_URL": "https://generativelanguage.googleapis.com/v1beta"
            }
        }));

        let url = adapter.extract_base_url(&provider).unwrap();
        assert_eq!(url, "https://generativelanguage.googleapis.com/v1beta");
    }

    #[test]
    fn test_extract_auth_api_key() {
        let adapter = GeminiAdapter::new();
        let provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "AIza-test-key-12345678"
            }
        }));

        let auth = adapter.extract_auth(&provider).unwrap();
        assert_eq!(auth.api_key, "AIza-test-key-12345678");
        assert_eq!(auth.strategy, AuthStrategy::Google);
        assert!(auth.access_token.is_none());
    }

    #[test]
    fn test_extract_auth_oauth_access_token() {
        let adapter = GeminiAdapter::new();
        let provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "ya29.test-access-token-12345"
            }
        }));

        let auth = adapter.extract_auth(&provider).unwrap();
        assert_eq!(auth.strategy, AuthStrategy::GoogleOAuth);
        assert_eq!(
            auth.access_token,
            Some("ya29.test-access-token-12345".to_string())
        );
    }

    #[test]
    fn test_extract_auth_oauth_json() {
        let adapter = GeminiAdapter::new();
        let provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "{\"access_token\":\"ya29.test-token\",\"refresh_token\":\"1//refresh\"}"
            }
        }));

        let auth = adapter.extract_auth(&provider).unwrap();
        assert_eq!(auth.strategy, AuthStrategy::GoogleOAuth);
        assert_eq!(auth.access_token, Some("ya29.test-token".to_string()));
    }

    #[test]
    fn test_provider_type_detection() {
        let adapter = GeminiAdapter::new();

        // API Key
        let api_key_provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "AIza-test-key"
            }
        }));
        assert_eq!(
            adapter.provider_type(&api_key_provider),
            ProviderType::Gemini
        );

        // OAuth access_token
        let oauth_provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "ya29.test-token"
            }
        }));
        assert_eq!(
            adapter.provider_type(&oauth_provider),
            ProviderType::GeminiCli
        );

        // OAuth JSON
        let oauth_json_provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "{\"access_token\":\"ya29.test\"}"
            }
        }));
        assert_eq!(
            adapter.provider_type(&oauth_json_provider),
            ProviderType::GeminiCli
        );
    }

    #[test]
    fn test_extract_auth_fallback() {
        let adapter = GeminiAdapter::new();
        let provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "AIza-fallback-key"
            }
        }));

        let auth = adapter.extract_auth(&provider).unwrap();
        assert_eq!(auth.api_key, "AIza-fallback-key");
    }

    #[test]
    fn test_build_url_dedup() {
        let adapter = GeminiAdapter::new();
        // base_url already contains /v1beta, and so does endpoint
        let url = adapter.build_url(
            "https://generativelanguage.googleapis.com/v1beta",
            "/v1beta/models/gemini-pro:generateContent",
        );
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent"
        );
    }

    #[test]
    fn test_build_url_normal() {
        let adapter = GeminiAdapter::new();
        let url = adapter.build_url(
            "https://generativelanguage.googleapis.com/v1beta",
            "/models/gemini-pro:generateContent",
        );
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro:generateContent"
        );
    }

    #[test]
    fn test_parse_oauth_credentials_direct_token() {
        let adapter = GeminiAdapter::new();
        let creds = adapter
            .parse_oauth_credentials("ya29.test-access-token")
            .unwrap();
        assert_eq!(creds.access_token, "ya29.test-access-token");
        assert!(creds.refresh_token.is_none());
    }

    #[test]
    fn test_parse_oauth_credentials_json() {
        let adapter = GeminiAdapter::new();
        let creds = adapter
            .parse_oauth_credentials(
                "{\"access_token\":\"ya29.test\",\"refresh_token\":\"1//refresh\"}",
            )
            .unwrap();
        assert_eq!(creds.access_token, "ya29.test");
        assert_eq!(creds.refresh_token, Some("1//refresh".to_string()));
    }

    #[test]
    fn test_parse_oauth_credentials_invalid() {
        let adapter = GeminiAdapter::new();
        assert!(adapter.parse_oauth_credentials("AIza-api-key").is_none());
        assert!(adapter.parse_oauth_credentials("invalid-json{").is_none());
    }
}
