//! Authentication Types
//!
//! Auth info and auth strategies for the various upstream providers.

/// Auth info
///
/// Holds the API key and its auth strategy
#[derive(Debug, Clone)]
pub struct AuthInfo {
    /// API Key
    pub api_key: String,
    /// Auth strategy
    pub strategy: AuthStrategy,
    /// OAuth access_token (for the GoogleOAuth strategy)
    pub access_token: Option<String>,
}

impl AuthInfo {
    /// Creates new auth info
    pub fn new(api_key: String, strategy: AuthStrategy) -> Self {
        Self {
            api_key,
            strategy,
            access_token: None,
        }
    }

    /// Creates auth info with an access_token (for OAuth)
    pub fn with_access_token(api_key: String, access_token: String) -> Self {
        Self {
            api_key,
            strategy: AuthStrategy::GoogleOAuth,
            access_token: Some(access_token),
        }
    }

    /// Returns the masked API key (for logging)
    ///
    /// Shows the first 4 and last 4 characters with `...` in between.
    /// Returns `***` if the key has 8 characters or fewer.
    #[allow(dead_code)]
    pub fn masked_key(&self) -> String {
        if self.api_key.chars().count() > 8 {
            let prefix: String = self.api_key.chars().take(4).collect();
            let suffix: String = self
                .api_key
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{prefix}...{suffix}")
        } else {
            "***".to_string()
        }
    }

    /// Returns the masked access_token (for logging)
    #[allow(dead_code)]
    pub fn masked_access_token(&self) -> Option<String> {
        self.access_token.as_ref().map(|token| {
            if token.chars().count() > 8 {
                let prefix: String = token.chars().take(4).collect();
                let suffix: String = token
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("{prefix}...{suffix}")
            } else {
                "***".to_string()
            }
        })
    }
}

/// Auth strategy
///
/// Each provider uses its own auth method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStrategy {
    /// Anthropic auth
    /// - Header: `x-api-key: <api_key>`
    /// - Header: `anthropic-version: 2023-06-01`
    Anthropic,

    /// Claude relay auth (Bearer only, no x-api-key)
    ///
    /// - Header: `Authorization: Bearer <api_key>`
    ///
    /// For relays that do not accept x-api-key
    ClaudeAuth,

    /// Bearer token auth (OpenAI and others)
    ///
    /// - Header: `Authorization: Bearer <api_key>`
    Bearer,

    /// Google API key auth
    ///
    /// - Header: `x-goog-api-key: <api_key>`
    Google,

    /// Google OAuth auth
    ///
    /// - Header: `Authorization: Bearer <access_token>`
    ///
    /// For OAuth clients such as Gemini CLI
    GoogleOAuth,

    /// GitHub Copilot auth
    ///
    /// - Header: `Authorization: Bearer <copilot_token>`
    ///
    /// Uses a dynamically fetched Copilot token (obtained through the GitHub OAuth device-code flow)
    GitHubCopilot,

    /// ChatGPT login of an Official Codex provider
    ///
    /// - Header: `Authorization: Bearer <access_token>`
    /// - Header: `chatgpt-account-id: <account id>`
    ///
    /// The forwarder resolves both from `codex_pool`, which refreshes the
    /// login when its access token runs out.
    ChatGpt,

    /// Captured login of an Official Claude provider
    ///
    /// - Header: `Authorization: Bearer <access token>`
    /// - `anthropic-beta` carries `oauth-2025-04-20`
    ///
    /// The forwarder resolves the token from `claude_pool`, which refreshes
    /// the login when its access token runs out.
    ClaudeOAuth,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_masked_key_long() {
        let auth = AuthInfo::new("sk-1234567890abcdef".to_string(), AuthStrategy::Bearer);
        assert_eq!(auth.masked_key(), "sk-1...cdef");
    }

    #[test]
    fn test_masked_key_short() {
        let auth = AuthInfo::new("short".to_string(), AuthStrategy::Bearer);
        assert_eq!(auth.masked_key(), "***");
    }

    #[test]
    fn test_masked_key_exactly_8() {
        let auth = AuthInfo::new("12345678".to_string(), AuthStrategy::Bearer);
        assert_eq!(auth.masked_key(), "***");
    }

    #[test]
    fn test_masked_key_9_chars() {
        let auth = AuthInfo::new("123456789".to_string(), AuthStrategy::Bearer);
        assert_eq!(auth.masked_key(), "1234...6789");
    }

    #[test]
    fn test_masked_key_utf8_safe() {
        let auth = AuthInfo::new("tëst⚠️1234567890".to_string(), AuthStrategy::Bearer);
        let masked = auth.masked_key();
        assert!(!masked.is_empty());
    }

    #[test]
    fn test_auth_strategy_equality() {
        assert_eq!(AuthStrategy::Anthropic, AuthStrategy::Anthropic);
        assert_ne!(AuthStrategy::Anthropic, AuthStrategy::Bearer);
        assert_ne!(AuthStrategy::Bearer, AuthStrategy::Google);
    }

    #[test]
    fn test_auth_info_new_has_no_access_token() {
        let auth = AuthInfo::new("api-key".to_string(), AuthStrategy::Bearer);
        assert!(auth.access_token.is_none());
    }

    #[test]
    fn test_auth_info_with_access_token() {
        let auth = AuthInfo::with_access_token(
            "refresh-token".to_string(),
            "ya29.access-token-12345".to_string(),
        );
        assert_eq!(auth.api_key, "refresh-token");
        assert_eq!(auth.strategy, AuthStrategy::GoogleOAuth);
        assert_eq!(
            auth.access_token,
            Some("ya29.access-token-12345".to_string())
        );
    }

    #[test]
    fn test_masked_access_token_long() {
        let auth =
            AuthInfo::with_access_token("refresh".to_string(), "ya29.1234567890abcdef".to_string());
        assert_eq!(auth.masked_access_token(), Some("ya29...cdef".to_string()));
    }

    #[test]
    fn test_masked_access_token_utf8_safe() {
        let auth =
            AuthInfo::with_access_token("refresh".to_string(), "tökén⚠️1234567890".to_string());
        let masked = auth.masked_access_token().unwrap();
        assert!(!masked.is_empty());
    }

    #[test]
    fn test_masked_access_token_short() {
        let auth = AuthInfo::with_access_token("refresh".to_string(), "short".to_string());
        assert_eq!(auth.masked_access_token(), Some("***".to_string()));
    }

    #[test]
    fn test_masked_access_token_none() {
        let auth = AuthInfo::new("api-key".to_string(), AuthStrategy::Bearer);
        assert!(auth.masked_access_token().is_none());
    }

    #[test]
    fn test_claude_auth_strategy() {
        let auth = AuthInfo::new("sk-test".to_string(), AuthStrategy::ClaudeAuth);
        assert_eq!(auth.strategy, AuthStrategy::ClaudeAuth);
        assert_ne!(auth.strategy, AuthStrategy::Anthropic);
        assert_ne!(auth.strategy, AuthStrategy::Bearer);
    }

    #[test]
    fn test_google_oauth_strategy() {
        let auth = AuthInfo::new("refresh-token".to_string(), AuthStrategy::GoogleOAuth);
        assert_eq!(auth.strategy, AuthStrategy::GoogleOAuth);
        assert_ne!(auth.strategy, AuthStrategy::Google);
    }

    #[test]
    fn test_all_strategies_are_distinct() {
        let strategies = [
            AuthStrategy::Anthropic,
            AuthStrategy::ClaudeAuth,
            AuthStrategy::Bearer,
            AuthStrategy::Google,
            AuthStrategy::GoogleOAuth,
            AuthStrategy::GitHubCopilot,
            AuthStrategy::ChatGpt,
            AuthStrategy::ClaudeOAuth,
        ];

        for (i, s1) in strategies.iter().enumerate() {
            for (j, s2) in strategies.iter().enumerate() {
                if i == j {
                    assert_eq!(s1, s2);
                } else {
                    assert_ne!(s1, s2);
                }
            }
        }
    }
}
