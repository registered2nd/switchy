use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// SSOT mode: per-provider copy files are no longer written

/// A provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(rename = "settingsConfig")]
    pub settings_config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortIndex")]
    pub sort_index: Option<usize>,
    /// Notes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Provider metadata (never written to the live config; stored only in ~/.switchy/config.json)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
    /// Icon name (e.g. "openai", "anthropic")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Icon color (hex, e.g. "#00A67E")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "iconColor")]
    pub icon_color: Option<String>,
    /// Whether the provider is in the failover queue
    #[serde(default)]
    #[serde(rename = "inFailoverQueue")]
    pub in_failover_queue: bool,
}

impl Provider {
    /// The account a pooled provider signs in as: a Codex ChatGPT login or a
    /// captured Claude login.
    pub fn account_email(&self) -> Option<String> {
        self.settings_config
            .get("auth")
            .and_then(crate::services::codex_account::inspect)
            .and_then(|login| login.email)
            .or_else(|| {
                self.meta
                    .as_ref()
                    .and_then(|meta| meta.captured_claude_account.as_ref())
                    .map(|account| account.email_address.clone())
            })
    }

    /// Create a provider with an existing ID
    pub fn with_id(
        id: String,
        name: String,
        settings_config: Value,
        website_url: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            settings_config,
            website_url,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }
}

/// Usage query script configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageScript {
    pub enabled: bool,
    pub language: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// API key used only for usage queries (general template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    /// Base URL used only for usage queries (general and NewAPI templates)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
    /// Access token for endpoints that require login (NewAPI template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessToken")]
    pub access_token: Option<String>,
    /// User ID for endpoints that require a user identifier (NewAPI template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    /// Template type (the backend uses it to pick validation rules)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "templateType")]
    pub template_type: Option<String>,
    /// Auto-query interval in minutes; 0 disables auto-query
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "autoQueryInterval")]
    pub auto_query_interval: Option<u64>,
    /// Coding Plan provider identifier (e.g. "kimi", "zhipu", "minimax")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "codingPlanProvider")]
    pub coding_plan_provider: Option<String>,
}

/// Usage data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "planName")]
    pub plan_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isValid")]
    pub is_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "invalidMessage")]
    pub invalid_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// Usage query result (supports multiple plans)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<UsageData>>, // may return several plans
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Per-provider model test configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderTestConfig {
    /// Whether the per-provider settings apply (false uses the global settings)
    #[serde(default)]
    pub enabled: bool,
    /// Model used for the test (overrides the global setting)
    #[serde(rename = "testModel", skip_serializing_if = "Option::is_none")]
    pub test_model: Option<String>,
    /// Timeout in seconds
    #[serde(rename = "timeoutSecs", skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Test prompt
    #[serde(rename = "testPrompt", skip_serializing_if = "Option::is_none")]
    pub test_prompt: Option<String>,
    /// Degraded threshold in milliseconds
    #[serde(
        rename = "degradedThresholdMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub degraded_threshold_ms: Option<u64>,
    /// Maximum retries
    #[serde(rename = "maxRetries", skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
}

/// Per-provider proxy configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderProxyConfig {
    /// Whether the per-provider settings apply (false uses the global/system proxy)
    #[serde(default)]
    pub enabled: bool,
    /// Proxy type: http, https, socks5
    #[serde(rename = "proxyType", skip_serializing_if = "Option::is_none")]
    pub proxy_type: Option<String>,
    /// Proxy host
    #[serde(rename = "proxyHost", skip_serializing_if = "Option::is_none")]
    pub proxy_host: Option<String>,
    /// Proxy port
    #[serde(rename = "proxyPort", skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<u16>,
    /// Proxy username (optional)
    #[serde(rename = "proxyUsername", skip_serializing_if = "Option::is_none")]
    pub proxy_username: Option<String>,
    /// Proxy password (optional)
    #[serde(rename = "proxyPassword", skip_serializing_if = "Option::is_none")]
    pub proxy_password: Option<String>,
}

/// Where authentication comes from
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthBindingSource {
    /// Read credentials from the provider's own config (default)
    #[default]
    ProviderConfig,
    /// Use a managed account (e.g. GitHub Copilot OAuth)
    ManagedAccount,
}

/// Generic authentication binding
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthBinding {
    /// Authentication source
    #[serde(default)]
    pub source: AuthBindingSource,
    /// Managed auth provider identifier (e.g. github_copilot)
    #[serde(rename = "authProvider", skip_serializing_if = "Option::is_none")]
    pub auth_provider: Option<String>,
    /// Managed account ID; empty means follow that auth provider's default account
    #[serde(rename = "accountId", skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

/// Provider metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderMeta {
    /// Custom endpoints (stored deduplicated by URL)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_endpoints: HashMap<String, crate::settings::CustomEndpoint>,
    /// Whether to apply the common config snippet when writing the live config
    #[serde(
        rename = "commonConfigEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub common_config_enabled: Option<bool>,
    /// Usage query script configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_script: Option<UsageScript>,
    /// Endpoint management: pick the fastest endpoint after a speed test
    #[serde(rename = "endpointAutoSelect", skip_serializing_if = "Option::is_none")]
    pub endpoint_auto_select: Option<bool>,
    /// Partner flag (the frontend uses isPartner; keep the field name in sync)
    #[serde(rename = "isPartner", skip_serializing_if = "Option::is_none")]
    pub is_partner: Option<bool>,
    /// Partner promotion key, used to recognise special providers such as PackyCode
    #[serde(
        rename = "partnerPromotionKey",
        skip_serializing_if = "Option::is_none"
    )]
    pub partner_promotion_key: Option<String>,
    /// Cost multiplier (used to compute actual cost)
    #[serde(rename = "costMultiplier", skip_serializing_if = "Option::is_none")]
    pub cost_multiplier: Option<String>,
    /// Pricing model source (response/request)
    #[serde(rename = "pricingModelSource", skip_serializing_if = "Option::is_none")]
    pub pricing_model_source: Option<String>,
    /// Daily spend limit (USD)
    #[serde(rename = "limitDailyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_daily_usd: Option<String>,
    /// Monthly spend limit (USD)
    #[serde(rename = "limitMonthlyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_monthly_usd: Option<String>,
    /// Per-provider model test configuration
    #[serde(rename = "testConfig", skip_serializing_if = "Option::is_none")]
    pub test_config: Option<ProviderTestConfig>,
    /// Per-provider proxy configuration
    #[serde(rename = "proxyConfig", skip_serializing_if = "Option::is_none")]
    pub proxy_config: Option<ProviderProxyConfig>,
    /// Claude API format (Claude providers only)
    /// - "anthropic": native Anthropic Messages API, passed through as-is
    /// - "openai_chat": OpenAI Chat Completions format, needs conversion
    /// - "openai_responses": OpenAI Responses API format, needs conversion
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// Generic authentication binding (provider_config / managed_account)
    ///
    /// New code should write only this field; githubAccountId is kept for reading old data.
    #[serde(rename = "authBinding", skip_serializing_if = "Option::is_none")]
    pub auth_binding: Option<AuthBinding>,
    /// Claude auth field name ("ANTHROPIC_AUTH_TOKEN" or "ANTHROPIC_API_KEY")
    #[serde(rename = "apiKeyField", skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    /// Whether base_url is a complete API endpoint (no endpoint path appended)
    #[serde(rename = "isFullUrl", skip_serializing_if = "Option::is_none")]
    pub is_full_url: Option<bool>,
    /// Prompt cache key for OpenAI-compatible endpoints.
    /// When set, injected into converted requests to improve cache hit rate.
    /// If not set, provider ID is used automatically during format conversion.
    #[serde(rename = "promptCacheKey", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    /// For additive-mode apps: whether this provider has been written to the live config.
    /// `None` means legacy data / unknown; `Some(false)` means it exists only in the database.
    #[serde(rename = "liveConfigManaged", skip_serializing_if = "Option::is_none")]
    pub live_config_managed: Option<bool>,
    /// Provider type identifier (used to detect special providers)
    /// - "github_copilot": GitHub Copilot provider
    #[serde(rename = "providerType", skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// Linked GitHub Copilot account ID (github_copilot providers only)
    /// Supports multiple accounts by linking to a specific GitHub account
    #[serde(rename = "githubAccountId", skip_serializing_if = "Option::is_none")]
    pub github_account_id: Option<String>,
    /// Captured Claude OAuth identity for this provider (Official/Claude only).
    /// Presence implies a snapshot exists under ~/.switchy/accounts/{id}/.
    #[serde(
        rename = "capturedClaudeAccount",
        skip_serializing_if = "Option::is_none"
    )]
    pub captured_claude_account: Option<CapturedClaudeAccountMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedClaudeAccountMeta {
    pub account_uuid: String,
    pub email_address: String,
    pub captured_at: i64,
}

impl ProviderMeta {
    /// Resolve the account ID bound to the given managed auth provider.
    ///
    /// Reads authBinding first; falls back to the legacy githubAccountId.
    pub fn managed_account_id_for(&self, auth_provider: &str) -> Option<String> {
        if let Some(binding) = self.auth_binding.as_ref() {
            if binding.source == AuthBindingSource::ManagedAccount
                && binding.auth_provider.as_deref() == Some(auth_provider)
            {
                return binding.account_id.clone();
            }
        }

        if auth_provider == "github_copilot" {
            return self.github_account_id.clone();
        }

        None
    }
}

// ============================================================================
// Universal Provider: configuration shared across apps
// ============================================================================

/// Which apps a universal provider is enabled for
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UniversalProviderApps {
    #[serde(default)]
    pub claude: bool,
    #[serde(default)]
    pub codex: bool,
    #[serde(default)]
    pub gemini: bool,
}

/// Claude model configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeModelConfig {
    /// Main model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Default Haiku model
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "haikuModel")]
    pub haiku_model: Option<String>,
    /// Default Sonnet model
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sonnetModel")]
    pub sonnet_model: Option<String>,
    /// Default Opus model
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "opusModel")]
    pub opus_model: Option<String>,
}

/// Codex model configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexModelConfig {
    /// Model name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reasoning effort
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "reasoningEffort")]
    pub reasoning_effort: Option<String>,
}

/// Gemini model configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiModelConfig {
    /// Model name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Per-app model configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UniversalProviderModels {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude: Option<ClaudeModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gemini: Option<GeminiModelConfig>,
}

/// Universal provider (configuration shared across apps)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalProvider {
    /// Unique identifier
    pub id: String,
    /// Provider name
    pub name: String,
    /// Provider type (e.g. "newapi", "custom")
    #[serde(rename = "providerType")]
    pub provider_type: String,
    /// Which apps are enabled
    pub apps: UniversalProviderApps,
    /// API base URL
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// API key
    #[serde(rename = "apiKey")]
    pub api_key: String,
    /// Per-app model configuration
    #[serde(default)]
    pub models: UniversalProviderModels,
    /// Website URL
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    /// Notes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Icon name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Icon color
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "iconColor")]
    pub icon_color: Option<String>,
    /// Metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
    /// Creation timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    /// Sort index
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortIndex")]
    pub sort_index: Option<usize>,
}

impl UniversalProvider {
    /// Create a new universal provider
    pub fn new(
        id: String,
        name: String,
        provider_type: String,
        base_url: String,
        api_key: String,
    ) -> Self {
        Self {
            id,
            name,
            provider_type,
            apps: UniversalProviderApps::default(),
            base_url,
            api_key,
            models: UniversalProviderModels::default(),
            website_url: None,
            notes: None,
            icon: None,
            icon_color: None,
            meta: None,
            created_at: Some(chrono::Utc::now().timestamp_millis()),
            sort_index: None,
        }
    }

    /// Build the Claude provider config
    pub fn to_claude_provider(&self) -> Option<Provider> {
        if !self.apps.claude {
            return None;
        }

        let models = self.models.claude.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
        let haiku = models
            .and_then(|m| m.haiku_model.clone())
            .unwrap_or_else(|| model.clone());
        let sonnet = models
            .and_then(|m| m.sonnet_model.clone())
            .unwrap_or_else(|| model.clone());
        let opus = models
            .and_then(|m| m.opus_model.clone())
            .unwrap_or_else(|| model.clone());

        let settings_config = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": self.base_url,
                "ANTHROPIC_AUTH_TOKEN": self.api_key,
                "ANTHROPIC_MODEL": model,
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": haiku,
                "ANTHROPIC_DEFAULT_SONNET_MODEL": sonnet,
                "ANTHROPIC_DEFAULT_OPUS_MODEL": opus,
            }
        });

        Some(Provider {
            id: format!("universal-claude-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// Build the Codex provider config
    pub fn to_codex_provider(&self) -> Option<Provider> {
        if !self.apps.codex {
            return None;
        }

        let models = self.models.codex.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "gpt-4o".to_string());
        let reasoning_effort = models
            .and_then(|m| m.reasoning_effort.clone())
            .unwrap_or_else(|| "high".to_string());

        // A Codex/OpenAI base_url may be a bare origin (needs /v1 appended) or carry a custom prefix (must not have a version forced on)
        let base_trimmed = self.base_url.trim_end_matches('/');
        let origin_only = match base_trimmed.split_once("://") {
            Some((_scheme, rest)) => !rest.contains('/'),
            None => !base_trimmed.contains('/'),
        };
        let codex_base_url = if base_trimmed.ends_with("/v1") {
            base_trimmed.to_string()
        } else if origin_only {
            format!("{base_trimmed}/v1")
        } else {
            base_trimmed.to_string()
        };

        // Build Codex's config.toml content
        let config_toml = format!(
            r#"model_provider = "newapi"
model = "{model}"
model_reasoning_effort = "{reasoning_effort}"
disable_response_storage = true

[model_providers.newapi]
name = "NewAPI"
base_url = "{codex_base_url}"
wire_api = "responses"
requires_openai_auth = true"#
        );

        let settings_config = serde_json::json!({
            "auth": {
                "OPENAI_API_KEY": self.api_key
            },
            "config": config_toml
        });

        Some(Provider {
            id: format!("universal-codex-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// Build the Gemini provider config
    pub fn to_gemini_provider(&self) -> Option<Provider> {
        if !self.apps.gemini {
            return None;
        }

        let models = self.models.gemini.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "gemini-2.5-pro".to_string());

        let settings_config = serde_json::json!({
            "env": {
                "GOOGLE_GEMINI_BASE_URL": self.base_url,
                "GEMINI_API_KEY": self.api_key,
                "GEMINI_MODEL": model,
            }
        });

        Some(Provider {
            id: format!("universal-gemini-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }
}

// ============================================================================
// OpenCode provider config structures
// ============================================================================

/// settings_config structure of an OpenCode provider
///
/// OpenCode names the provider type by its AI SDK package name, unlike the other apps' config formats.
/// Example:
/// ```json
/// {
///   "npm": "@ai-sdk/openai-compatible",
///   "options": { "baseURL": "https://api.example.com/v1", "apiKey": "sk-xxx" },
///   "models": { "gpt-4o": { "name": "GPT-4o" } }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeProviderConfig {
    /// AI SDK package name, e.g. "@ai-sdk/openai-compatible", "@ai-sdk/anthropic"
    pub npm: String,

    /// Provider name (optional, for display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Provider options (API key, base URL, etc.)
    #[serde(default)]
    pub options: OpenCodeProviderOptions,

    /// Model definitions
    #[serde(default)]
    pub models: HashMap<String, OpenCodeModel>,
}

impl Default for OpenCodeProviderConfig {
    fn default() -> Self {
        Self {
            npm: "@ai-sdk/openai-compatible".to_string(),
            name: None,
            options: OpenCodeProviderOptions::default(),
            models: HashMap::new(),
        }
    }
}

/// OpenCode provider options
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenCodeProviderOptions {
    /// API base URL
    #[serde(rename = "baseURL", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// API key (supports env var references such as "{env:API_KEY}")
    #[serde(rename = "apiKey", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Custom request headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Extra options (timeout, setCacheKey, etc.)
    /// flatten captures every field not defined explicitly
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

/// OpenCode model definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeModel {
    /// Model display name
    pub name: String,

    /// Model limits (context and output tokens)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<OpenCodeModelLimit>,

    /// Extra model options (provider routing, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, Value>>,

    /// Extra fields (cost, modalities, thinking, variants, etc.)
    /// flatten captures every field not defined explicitly
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

/// OpenCode model limits
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenCodeModelLimit {
    /// Context token limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,

    /// Output token limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{
        CapturedClaudeAccountMeta, ClaudeModelConfig, CodexModelConfig, GeminiModelConfig,
        OpenCodeProviderConfig, Provider, ProviderMeta, UniversalProvider,
    };
    use serde_json::json;

    #[test]
    fn provider_meta_captured_claude_account_round_trip() {
        let mut meta = ProviderMeta::default();
        meta.captured_claude_account = Some(CapturedClaudeAccountMeta {
            account_uuid: "11111111-2222-3333-4444-555555555555".into(),
            email_address: "alice@example.com".into(),
            captured_at: 1_760_000_000,
        });

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        let captured = value
            .get("capturedClaudeAccount")
            .expect("capturedClaudeAccount key present")
            .as_object()
            .expect("capturedClaudeAccount is object");
        assert_eq!(
            captured.get("accountUuid").and_then(|v| v.as_str()),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(
            captured.get("emailAddress").and_then(|v| v.as_str()),
            Some("alice@example.com")
        );
        assert_eq!(
            captured.get("capturedAt").and_then(|v| v.as_i64()),
            Some(1_760_000_000)
        );

        let round_tripped: ProviderMeta =
            serde_json::from_value(value).expect("deserialize ProviderMeta");
        let captured = round_tripped
            .captured_claude_account
            .expect("captured preserved");
        assert_eq!(
            captured.account_uuid,
            "11111111-2222-3333-4444-555555555555"
        );
        assert_eq!(captured.email_address, "alice@example.com");
        assert_eq!(captured.captured_at, 1_760_000_000);
    }

    #[test]
    fn provider_meta_omits_captured_claude_account_when_none() {
        let meta = ProviderMeta::default();
        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        assert!(value.get("capturedClaudeAccount").is_none());
    }

    #[test]
    fn provider_meta_deserializes_legacy_json_without_captured_field() {
        // Simulates loading a pre-feature config.json — no capturedClaudeAccount key.
        let legacy = serde_json::json!({});
        let meta: ProviderMeta = serde_json::from_value(legacy).expect("legacy load");
        assert!(meta.captured_claude_account.is_none());
    }

    #[test]
    fn provider_meta_serializes_pricing_model_source() {
        let mut meta = ProviderMeta::default();
        meta.pricing_model_source = Some("response".to_string());

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert_eq!(
            value
                .get("pricingModelSource")
                .and_then(|item| item.as_str()),
            Some("response")
        );
        assert!(value.get("pricing_model_source").is_none());
    }

    #[test]
    fn provider_meta_omits_pricing_model_source_when_none() {
        let meta = ProviderMeta::default();
        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert!(value.get("pricingModelSource").is_none());
    }

    #[test]
    fn provider_with_id_populates_defaults() {
        let settings_config = json!({
            "env": { "API_KEY": "test" }
        });
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            settings_config.clone(),
            Some("https://example.com".to_string()),
        );

        assert_eq!(provider.id, "provider-1");
        assert_eq!(provider.name, "Provider");
        assert_eq!(provider.settings_config, settings_config);
        assert_eq!(provider.website_url.as_deref(), Some("https://example.com"));
        assert!(provider.category.is_none());
        assert!(provider.created_at.is_none());
        assert!(provider.sort_index.is_none());
        assert!(provider.notes.is_none());
        assert!(provider.meta.is_none());
        assert!(provider.icon.is_none());
        assert!(provider.icon_color.is_none());
        assert!(!provider.in_failover_queue);
    }

    #[test]
    fn universal_provider_to_claude_provider_uses_models() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.claude = true;
        universal.models.claude = Some(ClaudeModelConfig {
            model: Some("claude-main".to_string()),
            haiku_model: Some("claude-haiku".to_string()),
            sonnet_model: Some("claude-sonnet".to_string()),
            opus_model: Some("claude-opus".to_string()),
        });

        let provider = universal.to_claude_provider().expect("claude provider");

        assert_eq!(provider.id, "universal-claude-u1");
        assert_eq!(provider.name, "Universal");
        assert_eq!(provider.category.as_deref(), Some("aggregator"));
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-main")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-haiku")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-sonnet")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-opus")
        );
    }

    #[test]
    fn universal_provider_to_claude_provider_disabled_returns_none() {
        let universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );

        assert!(universal.to_claude_provider().is_none());
    }

    #[test]
    fn universal_provider_to_codex_provider_appends_v1() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.codex = true;
        universal.models.codex = Some(CodexModelConfig {
            model: Some("gpt-4o-mini".to_string()),
            reasoning_effort: Some("low".to_string()),
        });

        let provider = universal.to_codex_provider().expect("codex provider");
        let config = provider
            .settings_config
            .get("config")
            .and_then(|item| item.as_str())
            .expect("config toml");

        assert!(config.contains("base_url = \"https://api.example.com/v1\""));
        assert_eq!(
            provider
                .settings_config
                .pointer("/auth/OPENAI_API_KEY")
                .and_then(|item| item.as_str()),
            Some("api-key")
        );
    }

    #[test]
    fn universal_provider_to_codex_provider_keeps_v1_suffix() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com/v1".to_string(),
            "api-key".to_string(),
        );
        universal.apps.codex = true;

        let provider = universal.to_codex_provider().expect("codex provider");
        let config = provider
            .settings_config
            .get("config")
            .and_then(|item| item.as_str())
            .expect("config toml");

        assert!(config.contains("base_url = \"https://api.example.com/v1\""));
    }

    #[test]
    fn universal_provider_to_codex_provider_disabled_returns_none() {
        let universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );

        assert!(universal.to_codex_provider().is_none());
    }

    #[test]
    fn universal_provider_to_gemini_provider_defaults_model() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.gemini = true;

        let provider = universal.to_gemini_provider().expect("gemini provider");

        assert_eq!(
            provider
                .settings_config
                .pointer("/env/GEMINI_MODEL")
                .and_then(|item| item.as_str()),
            Some("gemini-2.5-pro")
        );
    }

    #[test]
    fn universal_provider_to_gemini_provider_uses_model() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.gemini = true;
        universal.models.gemini = Some(GeminiModelConfig {
            model: Some("gemini-custom".to_string()),
        });

        let provider = universal.to_gemini_provider().expect("gemini provider");

        assert_eq!(
            provider
                .settings_config
                .pointer("/env/GEMINI_MODEL")
                .and_then(|item| item.as_str()),
            Some("gemini-custom")
        );
    }

    #[test]
    fn opencode_provider_config_defaults() {
        let config = OpenCodeProviderConfig::default();
        assert_eq!(config.npm, "@ai-sdk/openai-compatible");
        assert!(config.name.is_none());
        assert!(config.models.is_empty());
        assert!(config.options.base_url.is_none());
        assert!(config.options.api_key.is_none());
        assert!(config.options.headers.is_none());
        assert!(config.options.extra.is_empty());
    }

    #[test]
    fn universal_codex_provider_origin_base_url_adds_v1() {
        let mut p = UniversalProvider::new(
            "id".to_string(),
            "Test".to_string(),
            "custom".to_string(),
            "https://api.openai.com".to_string(),
            "sk-test".to_string(),
        );
        p.apps.codex = true;

        let provider = p.to_codex_provider().expect("should build codex provider");
        let toml = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config should be a toml string");

        assert!(toml.contains("base_url = \"https://api.openai.com/v1\""));
    }

    #[test]
    fn universal_codex_provider_custom_prefix_does_not_force_v1() {
        let mut p = UniversalProvider::new(
            "id".to_string(),
            "Test".to_string(),
            "custom".to_string(),
            "https://example.com/openai".to_string(),
            "sk-test".to_string(),
        );
        p.apps.codex = true;

        let provider = p.to_codex_provider().expect("should build codex provider");
        let toml = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config should be a toml string");

        assert!(toml.contains("base_url = \"https://example.com/openai\""));
        assert!(!toml.contains("https://example.com/openai/v1"));
    }
}
