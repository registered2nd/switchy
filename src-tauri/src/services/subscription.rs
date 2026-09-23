//! Official subscription quota service
//!
//! Reads the OAuth credentials the CLI tools already have and queries the official subscription quota.
//! Layer one: reads credentials only; no login or refresh.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use std::collections::HashMap;

use crate::config;

// ── Data types ────────────────────────────────────────────

/// Credential status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Valid,
    Expired,
    /// The provider refused the refresh token: sign the account in again.
    SignedOut,
    NotFound,
    ParseError,
}

/// A single rate-limit window (e.g. 5-hour session, 7-day period)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaTier {
    /// Window id: five_hour, seven_day, seven_day_opus, seven_day_sonnet, etc.
    pub name: String,
    /// Utilization percentage, 0-100
    pub utilization: f64,
    /// Reset time, ISO 8601
    pub resets_at: Option<String>,
}

/// Extra usage info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraUsage {
    pub is_enabled: bool,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: Option<String>,
}

/// Subscription quota query result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionQuota {
    pub tool: String,
    pub credential_status: CredentialStatus,
    pub credential_message: Option<String>,
    pub success: bool,
    pub tiers: Vec<QuotaTier>,
    pub extra_usage: Option<ExtraUsage>,
    pub error: Option<String>,
    pub queried_at: Option<i64>,
}

impl SubscriptionQuota {
    pub(crate) fn not_found(tool: &str) -> Self {
        Self {
            tool: tool.to_string(),
            credential_status: CredentialStatus::NotFound,
            credential_message: None,
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: None,
            queried_at: None,
        }
    }

    /// The account's refresh token was refused; it needs signing in again.
    fn signed_out(tool: &str) -> Self {
        Self::error(
            tool,
            CredentialStatus::SignedOut,
            "The login was refused; sign this account in again".to_string(),
        )
    }

    fn error(tool: &str, status: CredentialStatus, message: String) -> Self {
        Self {
            tool: tool.to_string(),
            credential_status: status,
            credential_message: Some(message.clone()),
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some(message),
            queried_at: Some(now_millis()),
        }
    }
}

// ── Claude credential reading ────────────────────────────

/// Nested structure in the Claude OAuth credentials file
#[derive(Deserialize)]
struct ClaudeOAuthEntry {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<serde_json::Value>,
}

/// Read the Claude OAuth credentials
///
/// Tries these sources in order:
/// 1. macOS Keychain (service: "Claude Code-credentials")
/// 2. Credentials file ~/.claude/.credentials.json
///
/// JSON format (both keys accepted):
/// {"claudeAiOauth": {"accessToken": "...", "expiresAt": ...}}
/// {"claude.ai_oauth": {"accessToken": "...", "expiresAt": ...}}
fn read_claude_credentials() -> (Option<String>, CredentialStatus, Option<String>) {
    // Source 1: macOS Keychain
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = read_claude_credentials_from_keychain() {
            return result;
        }
    }

    // Source 2: credentials file
    read_claude_credentials_from_file()
}

/// Read the Claude credentials from the macOS Keychain
#[cfg(target_os = "macos")]
fn read_claude_credentials_from_keychain(
) -> Option<(Option<String>, CredentialStatus, Option<String>)> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Claude Code-credentials",
            "-w",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None; // No such Keychain entry; fall back to the file
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    let json_str = json_str.trim();
    if json_str.is_empty() {
        return None;
    }

    Some(parse_claude_credentials_json(json_str))
}

/// Read the Claude credentials from the file
fn read_claude_credentials_from_file() -> (Option<String>, CredentialStatus, Option<String>) {
    let cred_path = config::get_claude_config_dir().join(".credentials.json");

    if !cred_path.exists() {
        return (None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&cred_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read credentials file: {e}")),
            );
        }
    };

    parse_claude_credentials_json(&content)
}

/// Parse the Claude credentials JSON (shared by the Keychain and file paths)
fn parse_claude_credentials_json(
    content: &str,
) -> (Option<String>, CredentialStatus, Option<String>) {
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse credentials JSON: {e}")),
            );
        }
    };

    // Accept both key names
    let entry_value = parsed
        .get("claudeAiOauth")
        .or_else(|| parsed.get("claude.ai_oauth"));

    let entry_value = match entry_value {
        Some(v) => v,
        None => {
            return (
                None,
                CredentialStatus::ParseError,
                Some("No OAuth entry found in credentials".to_string()),
            );
        }
    };

    let entry: ClaudeOAuthEntry = match serde_json::from_value(entry_value.clone()) {
        Ok(e) => e,
        Err(e) => {
            return (
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse OAuth entry: {e}")),
            );
        }
    };

    let access_token = match entry.access_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                None,
                CredentialStatus::ParseError,
                Some("accessToken is empty or missing".to_string()),
            );
        }
    };

    // Check whether the token has expired
    if let Some(expires_at) = entry.expires_at {
        if is_token_expired(&expires_at) {
            return (
                Some(access_token),
                CredentialStatus::Expired,
                Some("OAuth token has expired".to_string()),
            );
        }
    }

    (Some(access_token), CredentialStatus::Valid, None)
}

/// Whether the token has expired; accepts Unix timestamps (seconds or milliseconds) and ISO strings
fn is_token_expired(expires_at: &serde_json::Value) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    match expires_at {
        serde_json::Value::Number(n) => {
            if let Some(ts) = n.as_u64() {
                // Seconds vs milliseconds (millisecond timestamps are above 1e12)
                let ts_secs = if ts > 1_000_000_000_000 {
                    ts / 1000
                } else {
                    ts
                };
                ts_secs < now_secs
            } else {
                false
            }
        }
        serde_json::Value::String(s) => {
            // Try parsing as ISO 8601
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                (dt.timestamp() as u64) < now_secs
            } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
            {
                (dt.and_utc().timestamp() as u64) < now_secs
            } else {
                false // Unparseable means not expired
            }
        }
        _ => false,
    }
}

// ── Claude API query ─────────────────────────────────────

/// A single window in the Claude OAuth usage API response
#[derive(Deserialize)]
struct ApiUsageWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

/// Extra usage in the Claude OAuth usage API response
#[derive(Deserialize)]
struct ApiExtraUsage {
    is_enabled: Option<bool>,
    monthly_limit: Option<f64>,
    used_credits: Option<f64>,
    utilization: Option<f64>,
    currency: Option<String>,
}

/// Known Claude usage window names
const KNOWN_TIERS: &[&str] = &[
    "five_hour",
    "seven_day",
    "seven_day_opus",
    "seven_day_sonnet",
];

/// Anthropic's usage endpoint answers HTTP 429 to any User-Agent that is not
/// `claude-code/<version>` (verified 2026-09-17: no UA, curl, Mozilla and
/// `claude-cli/...` all 429; only this form gets 200). Same shape Claude Code
/// itself sends on this call.
const CLAUDE_USAGE_USER_AGENT: &str = "claude-code/2.1.276";

/// Query the official Claude subscription quota
async fn query_claude_quota(access_token: &str) -> SubscriptionQuota {
    let client = crate::proxy::http_client::get();

    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .header("User-Agent", CLAUDE_USAGE_USER_AGENT)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            return SubscriptionQuota::error(
                "claude",
                CredentialStatus::Valid,
                format!("Network error: {e}"),
            );
        }
    };

    let status = resp.status();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return SubscriptionQuota::error(
            "claude",
            CredentialStatus::Expired,
            format!("Authentication failed (HTTP {status}). Please re-login with Claude CLI."),
        );
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return SubscriptionQuota::error(
            "claude",
            CredentialStatus::Valid,
            format!("API error (HTTP {status}): {body}"),
        );
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return SubscriptionQuota::error(
                "claude",
                CredentialStatus::Valid,
                format!("Failed to parse API response: {e}"),
            );
        }
    };

    // Parse the known tier windows
    let mut tiers = Vec::new();
    for &tier_name in KNOWN_TIERS {
        if let Some(window) = body.get(tier_name) {
            if let Ok(w) = serde_json::from_value::<ApiUsageWindow>(window.clone()) {
                if let Some(util) = w.utilization {
                    tiers.push(QuotaTier {
                        name: tier_name.to_string(),
                        utilization: util,
                        resets_at: w.resets_at,
                    });
                }
            }
        }
    }

    // Also parse unknown windows (the API may add new window types)
    if let Some(obj) = body.as_object() {
        for (key, value) in obj {
            if key == "extra_usage" || KNOWN_TIERS.contains(&key.as_str()) {
                continue;
            }
            if let Ok(w) = serde_json::from_value::<ApiUsageWindow>(value.clone()) {
                if let Some(util) = w.utilization {
                    tiers.push(QuotaTier {
                        name: key.clone(),
                        utilization: util,
                        resets_at: w.resets_at,
                    });
                }
            }
        }
    }

    // Parse extra usage
    let extra_usage = body.get("extra_usage").and_then(|v| {
        serde_json::from_value::<ApiExtraUsage>(v.clone())
            .ok()
            .map(|e| ExtraUsage {
                is_enabled: e.is_enabled.unwrap_or(false),
                monthly_limit: e.monthly_limit,
                used_credits: e.used_credits,
                utilization: e.utilization,
                currency: e.currency,
            })
    });

    SubscriptionQuota {
        tool: "claude".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage,
        error: None,
        queried_at: Some(now_millis()),
    }
}

// ── Codex credential reading ─────────────────────────────

#[derive(Deserialize)]
struct CodexAuthJson {
    auth_mode: Option<String>,
    tokens: Option<CodexTokens>,
    last_refresh: Option<String>,
}

#[derive(Deserialize)]
struct CodexTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

/// (access_token, account_id, status, message)
type CodexCredentials = (
    Option<String>,
    Option<String>,
    CredentialStatus,
    Option<String>,
);

/// Read the Codex OAuth credentials
///
/// Tries these sources in order:
/// 1. macOS Keychain (service: "Codex Auth")
/// 2. Credentials file ~/.codex/auth.json
///
/// Valid only when auth_mode == "chatgpt" (OAuth); API key mode does not support usage queries.
fn read_codex_credentials() -> CodexCredentials {
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = read_codex_credentials_from_keychain() {
            return result;
        }
    }

    read_codex_credentials_from_file()
}

/// Read the Codex credentials from the macOS Keychain
#[cfg(target_os = "macos")]
fn read_codex_credentials_from_keychain() -> Option<CodexCredentials> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "Codex Auth", "-w"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    let json_str = json_str.trim();
    if json_str.is_empty() {
        return None;
    }

    Some(parse_codex_credentials_json(json_str))
}

/// Read the Codex credentials from the file
fn read_codex_credentials_from_file() -> CodexCredentials {
    let auth_path = crate::codex_config::get_codex_auth_path();

    if !auth_path.exists() {
        return (None, None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&auth_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read Codex auth file: {e}")),
            );
        }
    };

    parse_codex_credentials_json(&content)
}

/// Parse the Codex credentials JSON (shared by the Keychain and file paths)
fn parse_codex_credentials_json(content: &str) -> CodexCredentials {
    let auth: CodexAuthJson = match serde_json::from_str(content) {
        Ok(a) => a,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Codex auth JSON: {e}")),
            );
        }
    };

    // Only OAuth mode has usage data. Newer Codex no longer writes `auth_mode`, so the presence
    // of `tokens` decides; reject only when another mode is declared explicitly.
    let oauth_mode = match auth.auth_mode.as_deref() {
        Some("chatgpt") => true,
        Some(_) => false,
        None => auth.tokens.is_some(),
    };
    if !oauth_mode {
        return (
            None,
            None,
            CredentialStatus::NotFound,
            Some("Codex not using OAuth mode".to_string()),
        );
    }

    let tokens = match auth.tokens {
        Some(t) => t,
        None => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some("No tokens in Codex auth".to_string()),
            );
        }
    };

    let access_token = match tokens.access_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some("access_token is empty or missing".to_string()),
            );
        }
    };

    // Check whether the token may have expired (last refresh more than 8 days ago)
    if let Some(ref last_refresh) = auth.last_refresh {
        if is_codex_token_stale(last_refresh) {
            return (
                Some(access_token),
                tokens.account_id,
                CredentialStatus::Expired,
                Some("Codex token may be stale (>8 days since last refresh)".to_string()),
            );
        }
    }

    (
        Some(access_token),
        tokens.account_id,
        CredentialStatus::Valid,
        None,
    )
}

/// Whether the Codex token may have expired (the Codex CLI refreshes it after 8 days)
fn is_codex_token_stale(last_refresh: &str) -> bool {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(last_refresh) {
        let age_secs = now_secs.saturating_sub(dt.timestamp() as u64);
        age_secs > 8 * 24 * 3600
    } else {
        false
    }
}

// ── Codex API query ──────────────────────────────────────

#[derive(Deserialize)]
struct CodexRateLimitWindow {
    used_percent: Option<f64>,
    limit_window_seconds: Option<i64>,
    reset_at: Option<i64>,
}

#[derive(Deserialize)]
struct CodexRateLimit {
    primary_window: Option<CodexRateLimitWindow>,
    secondary_window: Option<CodexRateLimitWindow>,
}

/// A model-specific limit (e.g. `GPT-5.3-Codex-Spark`) with its own windows.
#[derive(Deserialize)]
struct CodexAdditionalRateLimit {
    limit_name: Option<String>,
    rate_limit: Option<CodexRateLimit>,
}

#[derive(Deserialize)]
struct CodexUsageResponse {
    rate_limit: Option<CodexRateLimit>,
    additional_rate_limits: Option<Vec<CodexAdditionalRateLimit>>,
}

/// Badge label for a model-specific limit: the last hyphenated token of the
/// limit name (`GPT-5.3-Codex-Spark` → `Spark`), so the lane reads like
/// Claude's per-model lane rather than repeating the family name.
fn codex_limit_short_name(limit_name: &str) -> String {
    limit_name
        .rsplit('-')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(limit_name)
        .to_string()
}

fn window_short_label(secs: i64) -> String {
    match secs {
        18000 => "5h".to_string(),
        604800 => "7d".to_string(),
        s if s >= 86400 => format!("{}d", s / 86400),
        s => format!("{}h", s / 3600),
    }
}

/// Tiers for one rate limit's windows. `lane` prefixes the tier name for a
/// model-specific limit; the account-wide limit uses the shared tier names.
fn codex_rate_limit_tiers(rate_limit: CodexRateLimit, lane: Option<&str>) -> Vec<QuotaTier> {
    let mut tiers = Vec::new();
    for window in [rate_limit.primary_window, rate_limit.secondary_window]
        .into_iter()
        .flatten()
    {
        let Some(used) = window.used_percent else {
            continue;
        };
        let name = match (lane, window.limit_window_seconds) {
            (Some(lane), Some(secs)) => format!("{lane} {}", window_short_label(secs)),
            (Some(lane), None) => lane.to_string(),
            (None, Some(secs)) => window_seconds_to_tier_name(secs),
            (None, None) => "unknown".to_string(),
        };
        tiers.push(QuotaTier {
            name,
            utilization: used,
            resets_at: window.reset_at.and_then(unix_ts_to_iso),
        });
    }
    tiers
}

#[cfg(test)]
mod codex_tier_tests {
    use super::*;

    fn window(secs: i64, used: f64) -> CodexRateLimitWindow {
        CodexRateLimitWindow {
            used_percent: Some(used),
            limit_window_seconds: Some(secs),
            reset_at: Some(1_789_809_936),
        }
    }

    #[test]
    fn account_limit_uses_shared_tier_names() {
        let tiers = codex_rate_limit_tiers(
            CodexRateLimit {
                primary_window: Some(window(604_800, 12.0)),
                secondary_window: None,
            },
            None,
        );
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].name, "seven_day");
        assert!(tiers[0].resets_at.is_some());
    }

    #[test]
    fn model_limit_becomes_a_named_lane_per_window() {
        let tiers = codex_rate_limit_tiers(
            CodexRateLimit {
                primary_window: Some(window(18_000, 0.0)),
                secondary_window: Some(window(604_800, 3.0)),
            },
            Some(&codex_limit_short_name("GPT-5.3-Codex-Spark")),
        );
        let names: Vec<_> = tiers.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Spark 5h", "Spark 7d"]);
    }

    #[test]
    fn empty_windows_are_skipped() {
        let tiers = codex_rate_limit_tiers(
            CodexRateLimit {
                primary_window: Some(window(604_800, 9.0)),
                secondary_window: Some(CodexRateLimitWindow {
                    used_percent: None,
                    limit_window_seconds: None,
                    reset_at: None,
                }),
            },
            None,
        );
        assert_eq!(tiers.len(), 1);
    }
}

/// Map a window length in seconds to a tier name (Claude-compatible naming so the frontend i18n can be reused)
fn window_seconds_to_tier_name(secs: i64) -> String {
    match secs {
        18000 => "five_hour".to_string(),
        604800 => "seven_day".to_string(),
        s => {
            let hours = s / 3600;
            if hours >= 24 {
                format!("{}_day", hours / 24)
            } else {
                format!("{}_hour", hours)
            }
        }
    }
}

/// Convert a Unix timestamp (seconds) to an ISO 8601 string
fn unix_ts_to_iso(ts: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.to_rfc3339())
}

/// Query the official Codex subscription quota
async fn query_codex_quota(access_token: &str, account_id: Option<&str>) -> SubscriptionQuota {
    let client = crate::proxy::http_client::get();

    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", "codex-cli")
        .header("Accept", "application/json");

    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }

    let resp = match req.timeout(std::time::Duration::from_secs(10)).send().await {
        Ok(r) => r,
        Err(e) => {
            return SubscriptionQuota::error(
                "codex",
                CredentialStatus::Valid,
                format!("Network error: {e}"),
            );
        }
    };

    let status = resp.status();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return SubscriptionQuota::error(
            "codex",
            CredentialStatus::Expired,
            format!("Authentication failed (HTTP {status}). Please re-login with Codex CLI."),
        );
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return SubscriptionQuota::error(
            "codex",
            CredentialStatus::Valid,
            format!("API error (HTTP {status}): {body}"),
        );
    }

    let body: CodexUsageResponse = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return SubscriptionQuota::error(
                "codex",
                CredentialStatus::Valid,
                format!("Failed to parse API response: {e}"),
            );
        }
    };

    let mut tiers = Vec::new();

    if let Some(rate_limit) = body.rate_limit {
        tiers.extend(codex_rate_limit_tiers(rate_limit, None));
    }
    // Model-specific limits carry their own 5h/7d windows — for some accounts
    // the only 5-hour window there is. Shown as extra lanes, like Claude's.
    for extra in body.additional_rate_limits.into_iter().flatten() {
        let Some(rate_limit) = extra.rate_limit else {
            continue;
        };
        let lane = extra
            .limit_name
            .as_deref()
            .map(codex_limit_short_name)
            .unwrap_or_else(|| "model".to_string());
        tiers.extend(codex_rate_limit_tiers(rate_limit, Some(&lane)));
    }

    SubscriptionQuota {
        tool: "codex".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    }
}

// ── Gemini credential reading ────────────────────────────

/// Gemini OAuth credentials file format (~/.gemini/oauth_creds.json)
#[derive(Deserialize)]
struct GeminiOAuthCredsFile {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expiry_date: Option<i64>, // Millisecond timestamp
}

/// (access_token, refresh_token, status, message)
type GeminiCredentials = (
    Option<String>,
    Option<String>,
    CredentialStatus,
    Option<String>,
);

/// Read the Gemini OAuth credentials
///
/// Tries these sources in order:
/// 1. macOS Keychain (service: "gemini-cli-oauth", account: "main-account")
/// 2. Credentials file ~/.gemini/oauth_creds.json (legacy format)
///
/// Valid only in OAuth auth mode (`oauth-personal`); API key mode cannot query official usage.
fn read_gemini_credentials() -> GeminiCredentials {
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = read_gemini_credentials_from_keychain() {
            return result;
        }
    }

    read_gemini_credentials_from_file()
}

/// Read the Gemini credentials from the macOS Keychain
#[cfg(target_os = "macos")]
fn read_gemini_credentials_from_keychain() -> Option<GeminiCredentials> {
    let output = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "gemini-cli-oauth",
            "-a",
            "main-account",
            "-w",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8(output.stdout).ok()?;
    let json_str = json_str.trim();
    if json_str.is_empty() {
        return None;
    }

    Some(parse_gemini_keychain_json(json_str))
}

/// Parse Gemini credentials in the Keychain format
///
/// Keychain format (keytar):
/// ```json
/// { "token": { "accessToken": "...", "refreshToken": "...", "expiresAt": 1234 }, "updatedAt": ... }
/// ```
#[cfg(target_os = "macos")]
fn parse_gemini_keychain_json(content: &str) -> GeminiCredentials {
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Gemini keychain JSON: {e}")),
            )
        }
    };

    let token = match parsed.get("token") {
        Some(t) => t,
        None => {
            // The Keychain entry may be flat; try the file format parser
            return parse_gemini_file_json(content);
        }
    };

    let access_token = token
        .get("accessToken")
        .and_then(|v| v.as_str())
        .map(String::from);
    let refresh_token = token
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .map(String::from);
    let expires_at = token.get("expiresAt").and_then(|v| v.as_i64());

    match access_token {
        Some(at) if !at.is_empty() => {
            // expiresAt is a millisecond timestamp
            if let Some(exp_ms) = expires_at {
                if exp_ms < now_millis() {
                    return (
                        Some(at),
                        refresh_token,
                        CredentialStatus::Expired,
                        Some("Gemini access token has expired".to_string()),
                    );
                }
            }
            (Some(at), refresh_token, CredentialStatus::Valid, None)
        }
        _ => (
            None,
            refresh_token,
            CredentialStatus::ParseError,
            Some("accessToken is empty or missing".to_string()),
        ),
    }
}

/// Read the Gemini credentials from the file
fn read_gemini_credentials_from_file() -> GeminiCredentials {
    let cred_path = crate::gemini_config::get_gemini_dir().join("oauth_creds.json");
    if !cred_path.exists() {
        return (None, None, CredentialStatus::NotFound, None);
    }

    let content = match std::fs::read_to_string(&cred_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to read Gemini credentials: {e}")),
            )
        }
    };

    parse_gemini_file_json(&content)
}

/// Parse Gemini credentials in the file format
///
/// File format (oauth_creds.json):
/// ```json
/// { "access_token": "...", "refresh_token": "...", "expiry_date": 1234 }
/// ```
fn parse_gemini_file_json(content: &str) -> GeminiCredentials {
    let creds: GeminiOAuthCredsFile = match serde_json::from_str(content) {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                None,
                CredentialStatus::ParseError,
                Some(format!("Failed to parse Gemini credentials: {e}")),
            )
        }
    };

    let access_token = match creds.access_token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                None,
                creds.refresh_token,
                CredentialStatus::ParseError,
                Some("access_token is empty or missing".to_string()),
            )
        }
    };

    // expiry_date is a millisecond timestamp
    if let Some(exp_ms) = creds.expiry_date {
        if exp_ms < now_millis() {
            return (
                Some(access_token),
                creds.refresh_token,
                CredentialStatus::Expired,
                Some("Gemini access token has expired".to_string()),
            );
        }
    }

    (
        Some(access_token),
        creds.refresh_token,
        CredentialStatus::Valid,
        None,
    )
}

// ── Gemini token refresh ────────────────────────────────────

/// Gemini OAuth client credentials (public values from the Gemini CLI source, google-gemini/gemini-cli)
const GEMINI_OAUTH_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GEMINI_OAUTH_CLIENT_SECRET: &str = "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";

/// Refresh the Gemini access token with the refresh_token
///
/// A Google OAuth access_token lasts only about 1h and must be refreshed regularly with the refresh_token.
/// The refresh_token itself does not expire (unless the user revokes access).
async fn refresh_gemini_token(refresh_token: &str) -> Option<String> {
    let client = crate::proxy::http_client::get();

    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", GEMINI_OAUTH_CLIENT_ID),
            ("client_secret", GEMINI_OAUTH_CLIENT_SECRET),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("access_token")?.as_str().map(String::from)
}

// ── Gemini API query ─────────────────────────────────────

/// loadCodeAssist response
#[derive(Deserialize)]
struct GeminiLoadCodeAssistResponse {
    #[serde(rename = "cloudaicompanionProject")]
    cloudaicompanion_project: Option<serde_json::Value>,
}

/// Quota bucket
#[derive(Deserialize)]
struct GeminiBucketInfo {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
}

/// retrieveUserQuota response
#[derive(Deserialize)]
struct GeminiQuotaResponse {
    buckets: Option<Vec<GeminiBucketInfo>>,
}

/// Extract the project ID from the loadCodeAssist response
fn extract_project_id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(obj) => obj
            .get("id")
            .or_else(|| obj.get("projectId"))
            .and_then(|v| v.as_str())
            .map(String::from),
        _ => None,
    }
}

/// Classify a Gemini model ID as Pro / Flash / Flash Lite
fn classify_gemini_model(model_id: &str) -> &str {
    if model_id.contains("flash-lite") {
        "gemini_flash_lite"
    } else if model_id.contains("flash") {
        "gemini_flash"
    } else if model_id.contains("pro") {
        "gemini_pro"
    } else {
        model_id
    }
}

/// Query the official Gemini subscription quota
///
/// Two API calls:
/// 1. loadCodeAssist -> get cloudaicompanionProject
/// 2. retrieveUserQuota -> get the quota data bucketed by model
async fn query_gemini_quota(access_token: &str) -> SubscriptionQuota {
    let client = crate::proxy::http_client::get();

    // ── Step 1: loadCodeAssist gets the project ID ──
    let load_resp = client
        .post("https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "metadata": {
                "ideType": "GEMINI_CLI",
                "pluginType": "GEMINI"
            }
        }))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let load_resp = match load_resp {
        Ok(r) => r,
        Err(e) => {
            return SubscriptionQuota::error(
                "gemini",
                CredentialStatus::Valid,
                format!("Network error (loadCodeAssist): {e}"),
            );
        }
    };

    let load_status = load_resp.status();
    if load_status == reqwest::StatusCode::UNAUTHORIZED
        || load_status == reqwest::StatusCode::FORBIDDEN
    {
        return SubscriptionQuota::error(
            "gemini",
            CredentialStatus::Expired,
            format!("Authentication failed (HTTP {load_status}). Please re-login with Gemini CLI."),
        );
    }
    if !load_status.is_success() {
        let body = load_resp.text().await.unwrap_or_default();
        return SubscriptionQuota::error(
            "gemini",
            CredentialStatus::Valid,
            format!("loadCodeAssist failed (HTTP {load_status}): {body}"),
        );
    }

    let load_body: GeminiLoadCodeAssistResponse = match load_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return SubscriptionQuota::error(
                "gemini",
                CredentialStatus::Valid,
                format!("Failed to parse loadCodeAssist response: {e}"),
            );
        }
    };

    let project_id = load_body
        .cloudaicompanion_project
        .as_ref()
        .and_then(extract_project_id);

    // ── Step 2: retrieveUserQuota gets the quota ──
    let mut quota_body = serde_json::json!({});
    if let Some(ref pid) = project_id {
        quota_body["project"] = serde_json::Value::String(pid.clone());
    }

    let quota_resp = client
        .post("https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .json(&quota_body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let quota_resp = match quota_resp {
        Ok(r) => r,
        Err(e) => {
            return SubscriptionQuota::error(
                "gemini",
                CredentialStatus::Valid,
                format!("Network error (retrieveUserQuota): {e}"),
            );
        }
    };

    let quota_status = quota_resp.status();
    if quota_status == reqwest::StatusCode::UNAUTHORIZED
        || quota_status == reqwest::StatusCode::FORBIDDEN
    {
        return SubscriptionQuota::error(
            "gemini",
            CredentialStatus::Expired,
            format!("Authentication failed (HTTP {quota_status})."),
        );
    }
    if !quota_status.is_success() {
        let body = quota_resp.text().await.unwrap_or_default();
        return SubscriptionQuota::error(
            "gemini",
            CredentialStatus::Valid,
            format!("retrieveUserQuota failed (HTTP {quota_status}): {body}"),
        );
    }

    let quota_data: GeminiQuotaResponse = match quota_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return SubscriptionQuota::error(
                "gemini",
                CredentialStatus::Valid,
                format!("Failed to parse quota response: {e}"),
            );
        }
    };

    // ── Aggregate by model class, taking the lowest remainingFraction in each ──
    let mut category_map: HashMap<String, (f64, Option<String>)> = HashMap::new();

    if let Some(buckets) = quota_data.buckets {
        for bucket in buckets {
            let model_id = bucket.model_id.as_deref().unwrap_or("unknown");
            let category = classify_gemini_model(model_id).to_string();
            let remaining = bucket.remaining_fraction.unwrap_or(1.0).clamp(0.0, 1.0);

            let entry = category_map
                .entry(category)
                .or_insert((remaining, bucket.reset_time.clone()));
            if remaining < entry.0 {
                entry.0 = remaining;
                if bucket.reset_time.is_some() {
                    entry.1.clone_from(&bucket.reset_time);
                }
            }
        }
    }

    // Convert to tiers (remainingFraction -> utilization: percentage used)
    let sort_order = |name: &str| -> usize {
        match name {
            "gemini_pro" => 0,
            "gemini_flash" => 1,
            "gemini_flash_lite" => 2,
            _ => 3,
        }
    };

    let mut tiers: Vec<QuotaTier> = category_map
        .into_iter()
        .map(|(name, (remaining, reset_time))| QuotaTier {
            name,
            utilization: (1.0 - remaining) * 100.0,
            resets_at: reset_time,
        })
        .collect();

    tiers.sort_by_key(|t| sort_order(&t.name));

    SubscriptionQuota {
        tool: "gemini".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    }
}

// ── Entry point ───────────────────────────────────────────

/// Query the official subscription quota of a CLI tool
async fn get_subscription_quota_uncached(tool: &str) -> Result<SubscriptionQuota, String> {
    match tool {
        "claude" => {
            let (token, status, message) = read_claude_credentials();

            match status {
                CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("claude")),
                CredentialStatus::SignedOut => Ok(SubscriptionQuota::signed_out("claude")),
                CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
                    "claude",
                    CredentialStatus::ParseError,
                    message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
                )),
                CredentialStatus::Expired => {
                    // Call the API even if expired (the token may still work)
                    if let Some(token) = token {
                        let result = query_claude_quota(&token).await;
                        if result.success {
                            return Ok(result);
                        }
                    }
                    Ok(SubscriptionQuota::error(
                        "claude",
                        CredentialStatus::Expired,
                        message.unwrap_or_else(|| "OAuth token has expired".to_string()),
                    ))
                }
                CredentialStatus::Valid => {
                    let token = token.expect("token must be Some when status is Valid");
                    Ok(query_claude_quota(&token).await)
                }
            }
        }
        "codex" => Ok(codex_quota_from_credentials(read_codex_credentials()).await),
        "gemini" => {
            let (token, refresh_token, status, message) = read_gemini_credentials();

            match status {
                CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("gemini")),
                CredentialStatus::SignedOut => Ok(SubscriptionQuota::signed_out("gemini")),
                CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
                    "gemini",
                    CredentialStatus::ParseError,
                    message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
                )),
                CredentialStatus::Expired => {
                    // The Gemini access_token lasts only about 1h; try refreshing with the refresh_token
                    if let Some(ref rt) = refresh_token {
                        if let Some(new_token) = refresh_gemini_token(rt).await {
                            return Ok(query_gemini_quota(&new_token).await);
                        }
                    }
                    // Refresh failed; try the old token
                    if let Some(ref token) = token {
                        let result = query_gemini_quota(token).await;
                        if result.success {
                            return Ok(result);
                        }
                    }
                    Ok(SubscriptionQuota::error(
                        "gemini",
                        CredentialStatus::Expired,
                        message.unwrap_or_else(|| "Gemini OAuth token has expired".to_string()),
                    ))
                }
                CredentialStatus::Valid => {
                    let token = token.expect("token must be Some when status is Valid");
                    Ok(query_gemini_quota(&token).await)
                }
            }
        }
        _ => Ok(SubscriptionQuota::not_found(tool)),
    }
}

/// Turns parsed Codex credentials into a quota reading, shared by the live
/// `~/.codex/auth.json` path and the per-provider stored-`auth` path.
async fn codex_quota_from_credentials(creds: CodexCredentials) -> SubscriptionQuota {
    let (token, account_id, status, message) = creds;
    match status {
        CredentialStatus::NotFound => SubscriptionQuota::not_found("codex"),
        CredentialStatus::SignedOut => SubscriptionQuota::signed_out("codex"),
        CredentialStatus::ParseError => SubscriptionQuota::error(
            "codex",
            CredentialStatus::ParseError,
            message.unwrap_or_else(|| "Failed to parse credentials".to_string()),
        ),
        CredentialStatus::Expired => {
            // Call the API even if it may have expired
            if let Some(token) = token {
                let result = query_codex_quota(&token, account_id.as_deref()).await;
                if result.success {
                    return result;
                }
            }
            SubscriptionQuota::error(
                "codex",
                CredentialStatus::Expired,
                message.unwrap_or_else(|| "Codex OAuth token may be stale".to_string()),
            )
        }
        CredentialStatus::Valid => {
            let token = token.expect("token must be Some when status is Valid");
            query_codex_quota(&token, account_id.as_deref()).await
        }
    }
}

/// Codex subscription quota for a specific provider's stored login.
///
/// A Codex provider carries its whole `auth.json` as `settings_config.auth`,
/// so a non-current Official Codex card can be read from that instead of the
/// live file — which belongs to whichever provider is current and would
/// otherwise show the same number on every card.
async fn get_codex_quota_for_provider_uncached(
    state: &crate::store::AppState,
    provider_id: &str,
) -> Result<SubscriptionQuota, String> {
    let provider = state
        .db
        .get_provider_by_id(provider_id, "codex")
        .map_err(|e| e.to_string())?;
    let Some(provider) = provider else {
        return Ok(SubscriptionQuota::not_found("codex"));
    };
    let Some(auth) = provider.settings_config.get("auth") else {
        return Ok(SubscriptionQuota::not_found("codex"));
    };
    if auth.get("tokens").is_none() {
        return Ok(SubscriptionQuota::not_found("codex"));
    }
    if crate::proxy::codex_pool::needs_sign_in(&provider) {
        return Ok(SubscriptionQuota::signed_out("codex"));
    }
    let content = serde_json::to_string(auth).map_err(|e| e.to_string())?;
    let quota = codex_quota_from_credentials(parse_codex_credentials_json(&content)).await;
    if quota.success || !matches!(quota.credential_status, CredentialStatus::Expired) {
        return Ok(quota);
    }
    // Renew the login the way the proxy does after a refusal (the access
    // token can be revoked before it expires): a login OpenAI refuses to
    // renew shows as signed out, a renewed one shows its usage.
    let renewed = crate::proxy::codex_pool::credentials_for(&state.db, &provider, true).await;
    if crate::proxy::codex_pool::needs_sign_in(&provider) {
        return Ok(SubscriptionQuota::signed_out("codex"));
    }
    match renewed {
        Ok(creds) => Ok(query_codex_quota(&creds.access_token, creds.account_id.as_deref()).await),
        Err(_) => Ok(quota),
    }
}

/// Claude subscription quota for a specific provider's captured snapshot.
///
/// Reads `~/.switchy/accounts/{provider_id}/credentials.json` (the snapshot
/// captured when the Official Claude provider was bound to an account) and
/// queries Anthropic's OAuth usage API with those credentials. This is the
/// per-account counterpart to `get_subscription_quota("claude")`, which
/// always reads live `~/.claude/.credentials.json` and therefore shows the
/// same number on every Official card.
async fn get_claude_quota_for_provider_uncached(
    state: &crate::store::AppState,
    provider_id: &str,
) -> Result<SubscriptionQuota, String> {
    let provider = state
        .db
        .get_provider_by_id(provider_id, "claude")
        .ok()
        .flatten();
    if let Some(provider) = provider.as_ref() {
        if crate::proxy::claude_pool::needs_sign_in(provider) {
            return Ok(SubscriptionQuota::signed_out("claude"));
        }
    }
    let quota = read_claude_snapshot_quota(provider_id).await?;
    if quota.success || !matches!(quota.credential_status, CredentialStatus::Expired) {
        return Ok(quota);
    }
    // Renew the captured login the way the proxy does after a refusal.
    let Some(provider) = provider.filter(crate::proxy::claude_pool::is_oauth_provider) else {
        return Ok(quota);
    };
    let renewed = crate::proxy::claude_pool::access_token_for(&provider, true).await;
    if crate::proxy::claude_pool::needs_sign_in(&provider) {
        return Ok(SubscriptionQuota::signed_out("claude"));
    }
    match renewed {
        Ok(token) => Ok(query_claude_quota(&token).await),
        Err(_) => Ok(quota),
    }
}

/// The captured snapshot's usage, as read from its stored access token.
async fn read_claude_snapshot_quota(provider_id: &str) -> Result<SubscriptionQuota, String> {
    let cred_path = crate::services::claude_account::paths::snapshot_credentials_path(provider_id);

    if !cred_path.exists() {
        return Ok(SubscriptionQuota::not_found("claude"));
    }

    let content = match std::fs::read_to_string(&cred_path) {
        Ok(c) => c,
        Err(e) => {
            return Ok(SubscriptionQuota::error(
                "claude",
                CredentialStatus::ParseError,
                format!("Failed to read snapshot credentials: {e}"),
            ));
        }
    };

    let (token, status, message) = parse_claude_credentials_json(&content);

    match status {
        CredentialStatus::NotFound => Ok(SubscriptionQuota::not_found("claude")),
        CredentialStatus::SignedOut => Ok(SubscriptionQuota::signed_out("claude")),
        CredentialStatus::ParseError => Ok(SubscriptionQuota::error(
            "claude",
            CredentialStatus::ParseError,
            message.unwrap_or_else(|| "Failed to parse snapshot credentials".to_string()),
        )),
        CredentialStatus::Expired => {
            if let Some(token) = token {
                let result = query_claude_quota(&token).await;
                if result.success {
                    return Ok(result);
                }
            }
            Ok(SubscriptionQuota::error(
                "claude",
                CredentialStatus::Expired,
                message.unwrap_or_else(|| "OAuth token has expired".to_string()),
            ))
        }
        CredentialStatus::Valid => {
            let token = token.expect("token must be Some when status is Valid");
            Ok(query_claude_quota(&token).await)
        }
    }
}

// ── Keeping a reading when the usage API refuses ─────────────────────────

/// An account's usage is asked for at most this often; a card that asks
/// sooner gets the last reading back.
const MIN_QUERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// The last successful reading per account (`tool:provider` or `tool:live`).
static LAST_GOOD: once_cell::sync::Lazy<
    std::sync::Mutex<HashMap<String, (std::time::Instant, SubscriptionQuota)>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

/// Runs `query` unless the account was read within [`MIN_QUERY_INTERVAL`].
/// When the query fails for a reason other than the login (the usage API's
/// own rate limit, a network error), the last good reading is returned
/// instead, with its original `queried_at`.
async fn with_last_good<F>(key: String, query: F) -> Result<SubscriptionQuota, String>
where
    F: std::future::Future<Output = Result<SubscriptionQuota, String>>,
{
    let cached = LAST_GOOD
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned();
    if let Some((at, quota)) = &cached {
        if at.elapsed() < MIN_QUERY_INTERVAL {
            return Ok(quota.clone());
        }
    }
    let result = query.await?;
    if result.success {
        LAST_GOOD
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, (std::time::Instant::now(), result.clone()));
        return Ok(result);
    }
    let login_is_fine = matches!(result.credential_status, CredentialStatus::Valid);
    match cached {
        Some((_, quota)) if login_is_fine => Ok(quota),
        _ => Ok(result),
    }
}

pub async fn get_subscription_quota(tool: &str) -> Result<SubscriptionQuota, String> {
    with_last_good(
        format!("{tool}:live"),
        get_subscription_quota_uncached(tool),
    )
    .await
}

pub async fn get_codex_quota_for_provider(
    state: &crate::store::AppState,
    provider_id: &str,
) -> Result<SubscriptionQuota, String> {
    with_last_good(
        format!("codex:{provider_id}"),
        get_codex_quota_for_provider_uncached(state, provider_id),
    )
    .await
}

pub async fn get_claude_quota_for_provider(
    state: &crate::store::AppState,
    provider_id: &str,
) -> Result<SubscriptionQuota, String> {
    with_last_good(
        format!("claude:{provider_id}"),
        get_claude_quota_for_provider_uncached(state, provider_id),
    )
    .await
}

// ── Helpers ───────────────────────────────────────────────

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod last_good_tests {
    use super::*;

    fn reading(tool: &str) -> SubscriptionQuota {
        SubscriptionQuota {
            tool: tool.to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: true,
            tiers: vec![],
            extra_usage: None,
            error: None,
            queried_at: Some(1),
        }
    }

    fn seed(key: &str, age_secs: u64) {
        let at = std::time::Instant::now() - std::time::Duration::from_secs(age_secs);
        LAST_GOOD
            .lock()
            .unwrap()
            .insert(key.to_string(), (at, reading("claude")));
    }

    #[tokio::test]
    async fn a_rate_limited_query_keeps_the_last_reading() {
        seed("claude:rate-limited", 120);
        let refused = SubscriptionQuota::error(
            "claude",
            CredentialStatus::Valid,
            "API error (HTTP 429 Too Many Requests)".to_string(),
        );
        let got = with_last_good("claude:rate-limited".into(), async { Ok(refused) })
            .await
            .unwrap();
        assert!(got.success);
        assert_eq!(got.queried_at, Some(1));
    }

    #[tokio::test]
    async fn a_refused_login_is_shown_even_with_an_old_reading() {
        seed("claude:signed-out", 120);
        let got = with_last_good("claude:signed-out".into(), async {
            Ok(SubscriptionQuota::signed_out("claude"))
        })
        .await
        .unwrap();
        assert!(!got.success);
        assert!(matches!(got.credential_status, CredentialStatus::SignedOut));
    }

    #[tokio::test]
    async fn a_recent_reading_is_reused_without_asking_again() {
        seed("claude:recent", 5);
        let got = with_last_good("claude:recent".into(), async {
            panic!("the usage API must not be asked again within the interval")
        })
        .await
        .unwrap();
        assert!(got.success);
    }
}
