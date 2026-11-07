use serde::{Deserialize, Serialize};

/// Proxy server config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// Listen address
    pub listen_address: String,
    /// Listen port
    pub listen_port: u16,
    /// Maximum retries
    pub max_retries: u8,
    /// Request timeout (seconds). Deprecated, kept for compatibility
    pub request_timeout: u64,
    /// Whether logging is enabled
    pub enable_logging: bool,
    /// Whether the live config is currently taken over
    #[serde(default)]
    pub live_takeover_active: bool,
    /// Streaming first-byte timeout (seconds): max wait for the first chunk, range 1-120, default 60
    #[serde(default = "default_streaming_first_byte_timeout")]
    pub streaming_first_byte_timeout: u64,
    /// Streaming idle timeout (seconds): max gap between chunks, range 60-600, 0 disables (guards against mid-stream stalls)
    #[serde(default = "default_streaming_idle_timeout")]
    pub streaming_idle_timeout: u64,
    /// Non-streaming total timeout (seconds): total timeout for non-streaming requests, range 60-1200, default 600 (10 minutes)
    #[serde(default = "default_non_streaming_timeout")]
    pub non_streaming_timeout: u64,
}

fn default_streaming_first_byte_timeout() -> u64 {
    60
}

fn default_streaming_idle_timeout() -> u64 {
    120
}

fn default_non_streaming_timeout() -> u64 {
    600
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_address: "127.0.0.1".to_string(),
            listen_port: 15721, // a rarely used high port
            max_retries: 3,
            request_timeout: 600,
            enable_logging: true,
            live_takeover_active: false,
            streaming_first_byte_timeout: 60,
            streaming_idle_timeout: 120,
            non_streaming_timeout: 600,
        }
    }
}

/// Proxy server status
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyStatus {
    /// Whether it is running
    pub running: bool,
    /// Listen address
    pub address: String,
    /// Listen port
    pub port: u16,
    /// Active connections
    pub active_connections: usize,
    /// Total requests
    pub total_requests: u64,
    /// Successful requests
    pub success_requests: u64,
    /// Failed requests
    pub failed_requests: u64,
    /// Success rate (0-100)
    pub success_rate: f32,
    /// Uptime (seconds)
    pub uptime_seconds: u64,
    /// Name of the provider currently in use
    pub current_provider: Option<String>,
    /// ID of the current provider
    pub current_provider_id: Option<String>,
    /// Time of the last request
    pub last_request_at: Option<String>,
    /// Last error message
    pub last_error: Option<String>,
    /// Provider failover count
    pub failover_count: u64,
    /// Currently active proxy targets
    #[serde(default)]
    pub active_targets: Vec<ActiveTarget>,
}

/// Active proxy target info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTarget {
    pub app_type: String, // "Claude" | "Codex" | "Gemini"
    pub provider_name: String,
    pub provider_id: String,
}

/// Proxy server info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyServerInfo {
    pub address: String,
    pub port: u16,
    pub started_at: String,
}

/// Per-app takeover state (whether the app's live config is rewritten to point at the local proxy)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyTakeoverStatus {
    pub claude: bool,
    pub codex: bool,
    pub gemini: bool,
    pub opencode: bool,
    pub openclaw: bool,
}

/// API format type (reserved; no format conversion needed yet)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ApiFormat {
    Claude,
    OpenAI,
    Gemini,
}

/// Provider health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider_id: String,
    pub app_type: String,
    pub is_healthy: bool,
    pub consecutive_failures: u32,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// Live config backup record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveBackup {
    /// App type (claude/codex/gemini)
    pub app_type: String,
    /// Original config JSON
    pub original_config: String,
    /// Backup time
    pub backed_up_at: String,
}

/// Global proxy config (unified fields, mirrored across three rows)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalProxyConfig {
    /// Proxy master switch
    pub proxy_enabled: bool,
    /// Listen address
    pub listen_address: String,
    /// Listen port
    pub listen_port: u16,
    /// Whether logging is enabled
    pub enable_logging: bool,
}

/// Per-app proxy config (independent per app)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppProxyConfig {
    /// App type (claude/codex/gemini)
    pub app_type: String,
    /// Proxy enabled switch for this app
    pub enabled: bool,
    /// Auto failover switch for this app
    pub auto_failover_enabled: bool,
    /// Maximum retries
    pub max_retries: u32,
    /// Streaming first-byte timeout (seconds)
    pub streaming_first_byte_timeout: u32,
    /// Streaming idle timeout (seconds)
    pub streaming_idle_timeout: u32,
    /// Non-streaming total timeout (seconds)
    pub non_streaming_timeout: u32,
    /// Circuit breaker failure threshold
    pub circuit_failure_threshold: u32,
    /// Circuit breaker recovery threshold
    pub circuit_success_threshold: u32,
    /// Circuit breaker recovery wait (seconds)
    pub circuit_timeout_seconds: u32,
    /// Error rate threshold
    pub circuit_error_rate_threshold: f64,
    /// Minimum requests before computing the error rate
    pub circuit_min_requests: u32,
}

/// Rectifier config
///
/// Stored in the settings table
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectifierConfig {
    /// Master switch: whether rectifiers are enabled (default on)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Request rectification: enable the thinking signature rectifier (default on)
    ///
    /// Handles the error: Invalid 'signature' in 'thinking' block
    #[serde(default = "default_true")]
    pub request_thinking_signature: bool,
    /// Request rectification: enable the thinking budget rectifier (default on)
    ///
    /// Handles errors about budget_tokens + thinking constraints
    #[serde(default = "default_true")]
    pub request_thinking_budget: bool,
}

fn default_true() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for RectifierConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            request_thinking_signature: true,
            request_thinking_budget: true,
        }
    }
}

/// Request optimizer config
///
/// Stored in the settings table, key = "optimizer_config"
/// Applies only to Bedrock providers (CLAUDE_CODE_USE_BEDROCK = "1")
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerConfig {
    /// Master switch (default off; the user must enable it)
    #[serde(default)]
    pub enabled: bool,
    /// Thinking optimization sub-switch (on by default once the master switch is on)
    #[serde(default = "default_true")]
    pub thinking_optimizer: bool,
    /// Cache injection sub-switch (on by default once the master switch is on)
    #[serde(default = "default_true")]
    pub cache_injection: bool,
    /// Cache TTL: "5m" | "1h" (default "1h")
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl: String,
}

fn default_cache_ttl() -> String {
    "1h".to_string()
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            thinking_optimizer: true,
            cache_injection: true,
            cache_ttl: "1h".to_string(),
        }
    }
}

/// Copilot optimizer config
///
/// Stored in the settings table, key = "copilot_optimizer_config"
/// Fixes abnormal usage consumption through the Copilot proxy (Issue #1813)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotOptimizerConfig {
    /// Master switch (default on; essential for Copilot users)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// x-initiator request classification (default on, P0 priority)
    #[serde(default = "default_true")]
    pub request_classification: bool,
    /// Tool result message merging (default on, P1 priority)
    #[serde(default = "default_true")]
    pub tool_result_merging: bool,
    /// Compact request detection (default on, P2 priority)
    #[serde(default = "default_true")]
    pub compact_detection: bool,
    /// Deterministic request ID (default on, P3 priority)
    #[serde(default = "default_true")]
    pub deterministic_request_id: bool,
    /// Warmup small-model downgrade (default off, P4 priority, opt-in)
    #[serde(default)]
    pub warmup_downgrade: bool,
    /// Model used for the warmup downgrade (default "gpt-4o-mini")
    #[serde(default = "default_warmup_model")]
    pub warmup_model: String,
}

fn default_warmup_model() -> String {
    "gpt-4o-mini".to_string()
}

impl Default for CopilotOptimizerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            request_classification: true,
            tool_result_merging: true,
            compact_detection: true,
            deterministic_request_id: true,
            warmup_downgrade: false,
            warmup_model: "gpt-4o-mini".to_string(),
        }
    }
}

/// Log config
///
/// Stored in the log_config field of the settings table (JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogConfig {
    /// Master switch: whether logging is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Log level: error, warn, info, debug, trace
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "info".to_string(),
        }
    }
}

impl LogConfig {
    /// Convert the config to a log::LevelFilter
    pub fn to_level_filter(&self) -> log::LevelFilter {
        if !self.enabled {
            return log::LevelFilter::Off;
        }
        match self.level.to_lowercase().as_str() {
            "error" => log::LevelFilter::Error,
            "warn" => log::LevelFilter::Warn,
            "info" => log::LevelFilter::Info,
            "debug" => log::LevelFilter::Debug,
            "trace" => log::LevelFilter::Trace,
            _ => log::LevelFilter::Info,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rectifier_config_default_enabled() {
        // RectifierConfig::default() should have everything on
        let config = RectifierConfig::default();
        assert!(
            config.enabled,
            "rectifier master switch should default to true"
        );
        assert!(
            config.request_thinking_signature,
            "thinking signature rectifier should default to true"
        );
        assert!(
            config.request_thinking_budget,
            "thinking budget rectifier should default to true"
        );
    }

    #[test]
    fn test_rectifier_config_serde_default() {
        // Missing fields deserialize to the default true
        let json = "{}";
        let config: RectifierConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert!(config.request_thinking_signature);
        assert!(config.request_thinking_budget);
    }

    #[test]
    fn test_rectifier_config_serde_explicit_true() {
        // Explicit true values deserialize correctly
        let json =
            r#"{"enabled": true, "requestThinkingSignature": true, "requestThinkingBudget": true}"#;
        let config: RectifierConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert!(config.request_thinking_signature);
        assert!(config.request_thinking_budget);
    }

    #[test]
    fn test_rectifier_config_serde_partial_fields() {
        // With only some fields set, missing fields default to true
        let json = r#"{"enabled": true, "requestThinkingSignature": false}"#;
        let config: RectifierConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert!(!config.request_thinking_signature);
        assert!(config.request_thinking_budget);
    }

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert!(config.enabled);
        assert_eq!(config.level, "info");
    }

    #[test]
    fn test_log_config_serde_default() {
        let json = "{}";
        let config: LogConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.level, "info");
    }

    #[test]
    fn test_log_config_to_level_filter() {
        let config = LogConfig {
            level: "error".to_string(),
            ..Default::default()
        };
        assert_eq!(config.to_level_filter(), log::LevelFilter::Error);

        let config = LogConfig {
            level: "warn".to_string(),
            ..Default::default()
        };
        assert_eq!(config.to_level_filter(), log::LevelFilter::Warn);

        let config = LogConfig {
            level: "info".to_string(),
            ..Default::default()
        };
        assert_eq!(config.to_level_filter(), log::LevelFilter::Info);

        let config = LogConfig {
            level: "debug".to_string(),
            ..Default::default()
        };
        assert_eq!(config.to_level_filter(), log::LevelFilter::Debug);

        let config = LogConfig {
            level: "trace".to_string(),
            ..Default::default()
        };
        assert_eq!(config.to_level_filter(), log::LevelFilter::Trace);

        // Invalid level falls back to info
        let config = LogConfig {
            level: "invalid".to_string(),
            ..Default::default()
        };
        assert_eq!(config.to_level_filter(), log::LevelFilter::Info);

        // Disabled returns Off
        let config = LogConfig {
            enabled: false,
            level: "debug".to_string(),
        };
        assert_eq!(config.to_level_filter(), log::LevelFilter::Off);
    }

    #[test]
    fn test_log_config_serde_roundtrip() {
        let config = LogConfig {
            enabled: true,
            level: "debug".to_string(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: LogConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.level, "debug");
    }
}
