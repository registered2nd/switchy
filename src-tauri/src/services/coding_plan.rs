//! Plan quota queries for API-key coding plans
//!
//! Supports the quota of Kimi Code (kimi.ai, and kimi.com in mainland China),
//! the Z.ai / Zhipu GLM Coding Plan, the MiniMax plans and Command Code, each
//! read with the plan's own API key.
//! Reuses the SubscriptionQuota / QuotaTier types from the subscription module.

use super::subscription::{CredentialStatus, QuotaTier, SubscriptionQuota};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Provider detection ──────────────────────────────────────

#[derive(Debug, PartialEq)]
enum CodingPlanProvider {
    KimiCn,
    KimiGlobal,
    ZhipuCn,
    ZhipuEn,
    MiniMaxCn,
    MiniMaxEn,
    CommandCode,
}

fn detect_provider(base_url: &str) -> Option<CodingPlanProvider> {
    let url = base_url.to_lowercase();
    if url.contains("api.kimi.ai/coding") {
        Some(CodingPlanProvider::KimiGlobal)
    } else if url.contains("api.kimi.com/coding") {
        Some(CodingPlanProvider::KimiCn)
    } else if url.contains("open.bigmodel.cn") || url.contains("bigmodel.cn") {
        Some(CodingPlanProvider::ZhipuCn)
    } else if url.contains("api.z.ai") {
        Some(CodingPlanProvider::ZhipuEn)
    } else if url.contains("api.minimaxi.com") {
        Some(CodingPlanProvider::MiniMaxCn)
    } else if url.contains("api.minimax.io") {
        Some(CodingPlanProvider::MiniMaxEn)
    } else if url.contains("api.commandcode.ai") {
        Some(CodingPlanProvider::CommandCode)
    } else {
        None
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn millis_to_iso8601(ms: i64) -> Option<String> {
    let secs = ms / 1000;
    let nsecs = ((ms % 1000) * 1_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nsecs).map(|dt| dt.to_rfc3339())
}

/// Extract the reset time from a JSON value, accepting string and number formats
/// - String: returned as is (ISO 8601)
/// - Number: seconds or milliseconds are detected and converted to ISO 8601
fn extract_reset_time(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = value.as_i64() {
        // Seconds vs milliseconds: second timestamps are < 1e12, millisecond ones >= 1e12
        let ms = if n < 1_000_000_000_000 { n * 1000 } else { n };
        return millis_to_iso8601(ms);
    }
    None
}

/// Parse a JSON value as f64, accepting numbers and strings (e.g. `100` and `"100"`)
fn parse_f64(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn make_error(msg: String) -> SubscriptionQuota {
    SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message: None,
        success: false,
        tiers: vec![],
        extra_usage: None,
        error: Some(msg),
        queried_at: Some(now_millis()),
    }
}

/// The plan refused the key.
fn auth_failed(detail: String) -> SubscriptionQuota {
    SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Expired,
        credential_message: Some("Invalid API key".to_string()),
        success: false,
        tiers: vec![],
        extra_usage: None,
        error: Some(format!("Authentication failed ({detail})")),
        queried_at: Some(now_millis()),
    }
}

fn quota_ok(tiers: Vec<QuotaTier>, credential_message: Option<String>) -> SubscriptionQuota {
    SubscriptionQuota {
        tool: "coding_plan".to_string(),
        credential_status: CredentialStatus::Valid,
        credential_message,
        success: true,
        tiers,
        extra_usage: None,
        error: None,
        queried_at: Some(now_millis()),
    }
}

/// Sends `request` and returns its JSON body, or the quota to show when the
/// request fails, the key is refused or the body is not JSON.
async fn fetch_json(request: reqwest::RequestBuilder) -> Result<serde_json::Value, SubscriptionQuota> {
    let resp = request
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| make_error(format!("Network error: {e}")))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(auth_failed(format!("HTTP {status}")));
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(make_error(format!("API error (HTTP {status}): {body}")));
    }

    resp.json()
        .await
        .map_err(|e| make_error(format!("Failed to parse response: {e}")))
}

fn used_percent(limit: f64, remaining: f64) -> f64 {
    if limit > 0.0 {
        ((limit - remaining).max(0.0) / limit) * 100.0
    } else {
        0.0
    }
}

// ── Kimi Code ───────────────────────────────────────────────

/// `/coding/v1/usages`: the per-window used ratios Kimi Code serves since
/// 2026-09 (`usages.limit_5h` and the rest, `used_ratio` 0-1), or the older
/// `limits[].detail` / `usage` counts.
fn parse_kimi(body: &serde_json::Value) -> Vec<QuotaTier> {
    let mut tiers = Vec::new();

    if let Some(usages) = body.get("usages").filter(|v| v.is_object()) {
        for (key, name) in [
            ("limit_5h", "five_hour"),
            ("limit_7d", "weekly_limit"),
            ("limit_month_total", "monthly_limit"),
        ] {
            let Some(entry) = usages.get(key) else {
                continue;
            };
            let Some(ratio) = entry.get("used_ratio").and_then(parse_f64) else {
                continue;
            };
            tiers.push(QuotaTier {
                name: name.to_string(),
                utilization: ratio * 100.0,
                resets_at: entry.get("reset_time").and_then(extract_reset_time),
            });
        }
        return tiers;
    }

    // 5-hour window limit (shown first)
    if let Some(limits) = body.get("limits").and_then(|v| v.as_array()) {
        for limit_item in limits {
            if let Some(detail) = limit_item.get("detail") {
                let limit = detail.get("limit").and_then(parse_f64).unwrap_or(1.0);
                let remaining = detail.get("remaining").and_then(parse_f64).unwrap_or(0.0);
                tiers.push(QuotaTier {
                    name: "five_hour".to_string(),
                    utilization: used_percent(limit, remaining),
                    resets_at: detail.get("resetTime").and_then(extract_reset_time),
                });
            }
        }
    }

    // Overall usage (weekly limit)
    if let Some(usage) = body.get("usage") {
        let limit = usage.get("limit").and_then(parse_f64).unwrap_or(1.0);
        let remaining = usage.get("remaining").and_then(parse_f64).unwrap_or(0.0);
        tiers.push(QuotaTier {
            name: "weekly_limit".to_string(),
            utilization: used_percent(limit, remaining),
            resets_at: usage.get("resetTime").and_then(extract_reset_time),
        });
    }

    tiers
}

async fn query_kimi(api_key: &str, is_cn: bool) -> SubscriptionQuota {
    let api_domain = if is_cn { "api.kimi.com" } else { "api.kimi.ai" };
    let request = crate::proxy::http_client::get()
        .get(format!("https://{api_domain}/coding/v1/usages"))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");

    match fetch_json(request).await {
        Ok(body) => quota_ok(parse_kimi(&body), None),
        Err(quota) => quota,
    }
}

// ── Zhipu GLM ───────────────────────────────────────────────

/// `monitor/usage/quota/limit`: `data.limits[]` items of type `TOKENS_LIMIT`
/// are the 5-hour pool (`unit` 3, `number` 5) and the weekly one (`unit` 6);
/// `percentage` is the share used. The plan tier is `data.level`. Every answer
/// is HTTP 200, so a refused key shows only in `success` and `code`.
fn parse_zhipu(body: &serde_json::Value) -> Result<(Vec<QuotaTier>, Option<String>), SubscriptionQuota> {
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        let msg = body
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Err(match code {
            401 | 1001 => auth_failed(msg.to_string()),
            _ => make_error(format!("API error (code {code}): {msg}")),
        });
    }

    let data = body
        .get("data")
        .ok_or_else(|| make_error("Missing 'data' field in response".to_string()))?;

    let mut tiers = Vec::new();

    if let Some(limits) = data.get("limits").and_then(|v| v.as_array()) {
        for limit_item in limits {
            if limit_item.get("type").and_then(|v| v.as_str()) != Some("TOKENS_LIMIT") {
                continue;
            }
            let unit = limit_item.get("unit").and_then(|v| v.as_i64());
            // An item without `unit` is the 5-hour pool of an older plan.
            let name = match unit {
                Some(6) => "weekly_limit",
                Some(3) | None => "five_hour",
                Some(_) => continue,
            };
            tiers.push(QuotaTier {
                name: name.to_string(),
                utilization: limit_item
                    .get("percentage")
                    .and_then(parse_f64)
                    .unwrap_or(0.0),
                resets_at: limit_item
                    .get("nextResetTime")
                    .and_then(|v| v.as_i64())
                    .and_then(millis_to_iso8601),
            });
        }
    }

    // The plan tier goes into credential_message
    let level = data
        .get("level")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok((tiers, level))
}

async fn query_zhipu(api_key: &str) -> SubscriptionQuota {
    // Always use the international api.z.ai (the China site bigmodel.cn has anti-scraping measures)
    let request = crate::proxy::http_client::get()
        .get("https://api.z.ai/api/monitor/usage/quota/limit")
        .header("Authorization", api_key) // Note: Zhipu takes no Bearer prefix
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en");

    let body = match fetch_json(request).await {
        Ok(body) => body,
        Err(quota) => return quota,
    };
    match parse_zhipu(&body) {
        Ok((tiers, level)) => quota_ok(tiers, level),
        Err(quota) => quota,
    }
}

// ── MiniMax ─────────────────────────────────────────────────

/// One window of a `model_remains` entry. `*_remaining_percent`, when sent, is
/// authoritative; otherwise `*_usage_count` is read as the remaining count, as
/// MiniMax's own CLI does. Status 3 is an unlimited window.
fn minimax_window(
    item: &serde_json::Value,
    total_key: &str,
    count_key: &str,
    percent_key: &str,
    status_key: &str,
    end_key: &str,
    name: &str,
) -> Option<QuotaTier> {
    if item.get(status_key).and_then(|v| v.as_i64()) == Some(3) {
        return None;
    }
    let utilization = match item.get(percent_key).and_then(parse_f64) {
        Some(remaining_percent) => (100.0 - remaining_percent).clamp(0.0, 100.0),
        None => {
            let total = item.get(total_key).and_then(parse_f64).unwrap_or(0.0);
            if total <= 0.0 {
                return None;
            }
            let remaining = item.get(count_key).and_then(parse_f64).unwrap_or(0.0);
            used_percent(total, remaining)
        }
    };
    Some(QuotaTier {
        name: name.to_string(),
        utilization,
        resets_at: item
            .get(end_key)
            .and_then(|v| v.as_i64())
            .and_then(millis_to_iso8601),
    })
}

/// `token_plan/remains` (and the older `coding_plan/remains`): the text-model
/// entry of `model_remains` (`general` or a `MiniMax-M*` model; the video and
/// speech entries have their own quota).
fn parse_minimax(body: &serde_json::Value) -> Result<Vec<QuotaTier>, SubscriptionQuota> {
    if let Some(base_resp) = body.get("base_resp") {
        let status_code = base_resp
            .get("status_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if status_code != 0 {
            let msg = base_resp
                .get("status_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(match status_code {
                1004 => auth_failed(msg.to_string()),
                _ => make_error(format!("API error (code {status_code}): {msg}")),
            });
        }
    }

    let Some(model_remains) = body.get("model_remains").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };
    let is_text_model = |item: &&serde_json::Value| {
        let name = item
            .get("model_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        name == "general" || name.starts_with("minimax-m")
    };
    let Some(item) = model_remains
        .iter()
        .find(is_text_model)
        .or_else(|| model_remains.first())
    else {
        return Ok(vec![]);
    };

    Ok([
        minimax_window(
            item,
            "current_interval_total_count",
            "current_interval_usage_count",
            "current_interval_remaining_percent",
            "current_interval_status",
            "end_time",
            "five_hour",
        ),
        minimax_window(
            item,
            "current_weekly_total_count",
            "current_weekly_usage_count",
            "current_weekly_remaining_percent",
            "current_weekly_status",
            "weekly_end_time",
            "weekly_limit",
        ),
    ]
    .into_iter()
    .flatten()
    .collect())
}

async fn query_minimax(api_key: &str, is_cn: bool) -> SubscriptionQuota {
    // minimax.io documents token_plan/remains for plan keys; the mainland
    // site keeps the coding_plan path.
    let url = if is_cn {
        "https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains"
    } else {
        "https://api.minimax.io/v1/token_plan/remains"
    };
    let request = crate::proxy::http_client::get()
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json");

    let body = match fetch_json(request).await {
        Ok(body) => body,
        Err(quota) => return quota,
    };
    match parse_minimax(&body) {
        Ok(tiers) => quota_ok(tiers, None),
        Err(quota) => quota,
    }
}

// ── Command Code ────────────────────────────────────────────

const COMMAND_CODE_API: &str = "https://api.commandcode.ai";

/// `alpha/billing/credits`: `credits.windowLimits.fiveHour` and `.weekly`,
/// each `{ used, cap, resetAt }` in dollars, as Command Code's CLI shows them
/// under `/usage`. A plan without windows (the Provider plan) has none.
fn parse_command_code_credits(body: &serde_json::Value) -> Vec<QuotaTier> {
    let Some(windows) = body
        .get("credits")
        .and_then(|c| c.get("windowLimits"))
        .filter(|v| v.is_object())
    else {
        return vec![];
    };
    [("fiveHour", "five_hour"), ("weekly", "weekly_limit")]
        .into_iter()
        .filter_map(|(key, name)| {
            let window = windows.get(key)?;
            let cap = window.get("cap").and_then(parse_f64)?;
            let used = window.get("used").and_then(parse_f64).unwrap_or(0.0);
            Some(QuotaTier {
                name: name.to_string(),
                utilization: if cap > 0.0 {
                    (used / cap * 100.0).min(100.0)
                } else {
                    0.0
                },
                resets_at: window.get("resetAt").and_then(extract_reset_time),
            })
        })
        .collect()
}

async fn query_command_code(api_key: &str) -> SubscriptionQuota {
    let client = crate::proxy::http_client::get();
    let auth = format!("Bearer {api_key}");

    // The credits are per organization; the key's own is in whoami.
    let whoami = match fetch_json(
        client
            .get(format!("{COMMAND_CODE_API}/alpha/whoami?limits=1"))
            .header("Authorization", &auth),
    )
    .await
    {
        Ok(body) => body,
        Err(quota) => return quota,
    };
    let Some(org_id) = whoami
        .get("org")
        .and_then(|o| o.get("id"))
        .and_then(|v| v.as_str())
    else {
        return make_error("Command Code returned no organization for this key".to_string());
    };

    match fetch_json(
        client
            .get(format!("{COMMAND_CODE_API}/alpha/billing/credits"))
            .query(&[("orgId", org_id)])
            .header("Authorization", &auth),
    )
    .await
    {
        Ok(body) => quota_ok(parse_command_code_credits(&body), None),
        Err(quota) => quota,
    }
}

// ── Public entry point ──────────────────────────────────────

pub async fn get_coding_plan_quota(
    base_url: &str,
    api_key: &str,
) -> Result<SubscriptionQuota, String> {
    if api_key.trim().is_empty() {
        return Ok(SubscriptionQuota {
            tool: "coding_plan".to_string(),
            credential_status: CredentialStatus::NotFound,
            credential_message: None,
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some("No API key".to_string()),
            queried_at: None,
        });
    }

    let provider = match detect_provider(base_url) {
        Some(p) => p,
        None => {
            return Ok(SubscriptionQuota {
                tool: "coding_plan".to_string(),
                credential_status: CredentialStatus::NotFound,
                credential_message: None,
                success: false,
                tiers: vec![],
                extra_usage: None,
                error: None,
                queried_at: None,
            })
        }
    };

    let quota = match provider {
        CodingPlanProvider::KimiCn => query_kimi(api_key, true).await,
        CodingPlanProvider::KimiGlobal => query_kimi(api_key, false).await,
        CodingPlanProvider::ZhipuCn | CodingPlanProvider::ZhipuEn => query_zhipu(api_key).await,
        CodingPlanProvider::MiniMaxCn => query_minimax(api_key, true).await,
        CodingPlanProvider::MiniMaxEn => query_minimax(api_key, false).await,
        CodingPlanProvider::CommandCode => query_command_code(api_key).await,
    };

    Ok(quota)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn international_hosts_are_told_from_mainland_ones() {
        assert_eq!(
            detect_provider("https://api.kimi.ai/coding/"),
            Some(CodingPlanProvider::KimiGlobal)
        );
        assert_eq!(
            detect_provider("https://api.kimi.com/coding/"),
            Some(CodingPlanProvider::KimiCn)
        );
        assert_eq!(
            detect_provider("https://api.z.ai/api/coding/paas/v4"),
            Some(CodingPlanProvider::ZhipuEn)
        );
        assert_eq!(
            detect_provider("https://api.minimax.io/anthropic"),
            Some(CodingPlanProvider::MiniMaxEn)
        );
        assert_eq!(
            detect_provider("https://api.commandcode.ai/provider"),
            Some(CodingPlanProvider::CommandCode)
        );
        assert_eq!(detect_provider("https://opencode.ai/inference/go/anthropic"), None);
    }

    #[test]
    fn kimi_reads_used_ratios_and_falls_back_to_counts() {
        let tiers = parse_kimi(&json!({
            "usages": {
                "limit_5h": { "used_ratio": 0.3, "reset_time": "2026-09-11T18:00:00Z" },
                "limit_7d": { "used_ratio": "0.2", "reset_time": "2026-09-17T00:00:00Z" },
                "limit_month_total": { "used_ratio": 0.4, "reset_time": "2026-10-01T00:00:00Z" },
                "limit_month_code": { "used_ratio": 0.25 }
            }
        }));
        let names: Vec<_> = tiers.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["five_hour", "weekly_limit", "monthly_limit"]);
        assert!((tiers[0].utilization - 30.0).abs() < 1e-9);
        assert!((tiers[1].utilization - 20.0).abs() < 1e-9);
        assert_eq!(tiers[0].resets_at.as_deref(), Some("2026-09-11T18:00:00Z"));

        let old = parse_kimi(&json!({
            "limits": [{ "detail": { "limit": 100, "remaining": 75, "resetTime": "2026-09-11T18:00:00Z" } }],
            "usage": { "limit": "200", "remaining": "50" }
        }));
        assert_eq!(old.len(), 2);
        assert!((old[0].utilization - 25.0).abs() < 1e-9);
        assert!((old[1].utilization - 75.0).abs() < 1e-9);
    }

    #[test]
    fn zhipu_separates_the_five_hour_and_weekly_pools() {
        let (tiers, level) = parse_zhipu(&json!({
            "code": 200, "success": true,
            "data": {
                "level": "pro",
                "limits": [
                    { "type": "TIME_LIMIT", "unit": 5, "number": 1, "percentage": 10 },
                    { "type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 42, "nextResetTime": 1790000000000_i64 },
                    { "type": "TOKENS_LIMIT", "unit": 6, "number": 1, "percentage": 17 }
                ]
            }
        }))
        .unwrap();
        assert_eq!(level.as_deref(), Some("pro"));
        let names: Vec<_> = tiers.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["five_hour", "weekly_limit"]);
        assert_eq!(tiers[0].utilization, 42.0);
        assert!(tiers[0].resets_at.is_some());
    }

    #[test]
    fn zhipu_refused_key_is_an_auth_failure_despite_http_200() {
        let quota = parse_zhipu(&json!({
            "code": 401, "msg": "token expired or incorrect", "success": false
        }))
        .unwrap_err();
        assert!(!quota.success);
        assert!(matches!(quota.credential_status, CredentialStatus::Expired));
        assert!(quota.error.unwrap().contains("token expired or incorrect"));
    }

    #[test]
    fn minimax_prefers_remaining_percent_and_the_text_model_entry() {
        let tiers = parse_minimax(&json!({
            "base_resp": { "status_code": 0, "status_msg": "success" },
            "model_remains": [
                { "model_name": "video", "current_interval_total_count": 10, "current_interval_usage_count": 0 },
                {
                    "model_name": "general",
                    "current_interval_total_count": 1000,
                    "current_interval_usage_count": 900,
                    "current_interval_remaining_percent": 90,
                    "end_time": 1790000000000_i64,
                    "current_weekly_total_count": 5000,
                    "current_weekly_usage_count": 4000,
                    "weekly_end_time": 1790500000000_i64
                }
            ]
        }))
        .unwrap();
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 10.0).abs() < 1e-9);
        // No percent: the count is the remaining count.
        assert!((tiers[1].utilization - 20.0).abs() < 1e-9);
    }

    #[test]
    fn minimax_missing_key_is_an_auth_failure() {
        let quota = parse_minimax(&json!({
            "base_resp": { "status_code": 1004, "status_msg": "login fail: Please carry the API secret key in the 'Authorization' field of the request header" }
        }))
        .unwrap_err();
        assert!(matches!(quota.credential_status, CredentialStatus::Expired));
    }

    #[test]
    fn command_code_reads_its_window_limits() {
        let tiers = parse_command_code_credits(&json!({
            "credits": {
                "monthlyCredits": 80, "planId": "pro",
                "windowLimits": {
                    "fiveHour": { "used": 4, "cap": 16, "resetAt": 1790000000000_i64 },
                    "weekly": { "used": 30, "cap": 40, "resetAt": 1790500000000_i64 }
                }
            }
        }));
        assert_eq!(tiers.len(), 2);
        assert!((tiers[0].utilization - 25.0).abs() < 1e-9);
        assert!((tiers[1].utilization - 75.0).abs() < 1e-9);
        assert!(parse_command_code_credits(&json!({ "credits": { "windowLimits": null } })).is_empty());
    }

    #[tokio::test]
    async fn an_empty_key_fails_with_a_reason_and_sends_nothing() {
        let quota = get_coding_plan_quota("https://api.z.ai/api/anthropic", "  ")
            .await
            .unwrap();
        assert!(!quota.success);
        assert!(matches!(quota.credential_status, CredentialStatus::NotFound));
        assert_eq!(quota.error.as_deref(), Some("No API key"));
    }
}
