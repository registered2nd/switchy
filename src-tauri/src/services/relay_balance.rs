//! A relay card's balance, read with the card's own key.
//!
//! Relays run on a few open panels, and each panel has a balance call: sub2api
//! answers `/v1/usage` (the call cc-switch reads), new-api and one-api answer
//! the OpenAI-style `/v1/dashboard/billing/subscription` and `/usage` for the
//! key, and `/api/user/self` for the whole account to its access token. A card
//! holding the account's access token reads `/api/user/self`; any other card
//! tries the key's calls in turn on its relay's host and shows the first
//! balance that answers, and a relay none of them answers shows nothing. A
//! relay whose units are not dollars (KuaiAPI sells 6.5 to the dollar) is
//! converted by the card's units per dollar, or by the relay's known rate.

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::{Provider, RelayBalanceConfig};
use crate::store::AppState;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

/// Relays whose units are not dollars: (host, units per dollar).
const KNOWN_RATES: &[(&str, f64)] = &[("kuaiapi.net", 6.5)];

/// new-api and one-api report a key without a limit of its own as this many
/// dollars.
const UNLIMITED_USD: f64 = 100_000_000.0;

/// new-api's and one-api's quota per dollar, unless the relay's status says
/// otherwise.
const DEFAULT_QUOTA_PER_UNIT: f64 = 500_000.0;

const CATEGORIES_WITHOUT_RELAY: &[&str] = &["official", "cn_official", "cloud_provider"];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelayBalance {
    pub success: bool,
    pub error: Option<String>,
    /// Dollars left.
    pub balance: Option<f64>,
    /// The relay's own figure, in its units.
    pub units: Option<f64>,
    /// How many of the relay's units make a dollar.
    pub units_per_dollar: f64,
    /// The call that answered: `usage` (`/v1/usage`), `billing`
    /// (`/v1/dashboard/billing`) or `account` (`/api/user/self`).
    pub source: Option<String>,
    pub queried_at: i64,
}

/// What one balance call made of the relay's answer.
#[derive(Debug, PartialEq)]
enum Answer {
    /// The balance, in the relay's units, and the call that gave it.
    Units(f64, &'static str),
    /// The relay answered the call but shows no balance through it.
    Refused(String),
}

/// The relay's address without its path: the panels answer at the root.
fn origin(base_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(base_url.trim()).ok()?;
    url.host_str()?;
    Some(url.origin().ascii_serialization())
}

fn host_matches(base_url: &str, host: &str) -> bool {
    reqwest::Url::parse(base_url.trim())
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .is_some_and(|h| h == host || h.ends_with(&format!(".{host}")))
}

/// The card's units per dollar, else the relay's known rate, else 1.
fn units_per_dollar(base_url: &str, config: Option<&RelayBalanceConfig>) -> f64 {
    config
        .and_then(|c| c.units_per_dollar)
        .filter(|r| r.is_finite() && *r > 0.0)
        .or_else(|| {
            KNOWN_RATES
                .iter()
                .find(|(host, _)| host_matches(base_url, host))
                .map(|(_, rate)| *rate)
        })
        .unwrap_or(1.0)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .filter(|n| n.is_finite())
}

fn message(body: &Value) -> Option<String> {
    ["invalidMessage", "message", "error"]
        .iter()
        .find_map(|key| match body.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Some(Value::Object(o)) => o.get("message").and_then(Value::as_str).map(str::to_string),
            _ => None,
        })
}

/// sub2api's `/v1/usage`: the wallet's `balance`, or for a key on a
/// subscription group what it has `remaining`.
fn parse_usage(body: &Value) -> Option<Answer> {
    if body.get("isValid").and_then(Value::as_bool) == Some(false) {
        return Some(Answer::Refused(message(body).unwrap_or_else(|| {
            "The relay says this key is not valid".to_string()
        })));
    }
    body.get("balance")
        .and_then(number)
        .or_else(|| body.get("remaining").and_then(number))
        .map(|units| Answer::Units(units, "usage"))
}

/// new-api's and one-api's `/v1/dashboard/billing/subscription` (`hard_limit_usd`,
/// what the key may spend in all) and `/usage` (`total_usage`, in cents).
fn parse_billing(subscription: &Value, usage: Option<&Value>) -> Option<Answer> {
    let limit = subscription.get("hard_limit_usd").and_then(number)?;
    if limit >= UNLIMITED_USD {
        return Some(Answer::Refused(
            "This key has no limit of its own, and the relay shows the account's balance only to \
             the account: add its access token and user ID under Balance in the card's Edit dialog"
                .to_string(),
        ));
    }
    let used_cents = usage?.get("total_usage").and_then(number)?;
    Some(Answer::Units(limit - used_cents / 100.0, "billing"))
}

/// new-api's and one-api's `/api/user/self`: the account's `quota`, in quota
/// units, `quota_per_unit` to the dollar.
fn parse_account(body: &Value, quota_per_unit: f64) -> Answer {
    match body.pointer("/data/quota").and_then(number) {
        Some(quota) if body.get("success").and_then(Value::as_bool) != Some(false) => {
            Answer::Units(quota / quota_per_unit, "account")
        }
        _ => Answer::Refused(format!(
            "The relay refused the account's access token: {}",
            message(body).unwrap_or_else(|| "no balance in its answer".to_string())
        )),
    }
}

/// A GET answered with JSON; `Ok(None)` when the relay has no such call (any
/// other status, or a body that is not JSON); `Err` when it could not be
/// reached.
async fn get_json(
    client: &Client,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<Option<Value>, String> {
    let mut request = client
        .get(url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(response.json::<Value>().await.ok())
}

async fn read_account(
    client: &Client,
    origin: &str,
    token: &str,
    user_id: &str,
) -> Result<Answer, String> {
    let bearer = format!("Bearer {token}");
    let headers = [
        ("Authorization", bearer.as_str()),
        ("New-Api-User", user_id),
    ];
    let Some(body) = get_json(client, &format!("{origin}/api/user/self"), &headers).await? else {
        return Ok(Answer::Refused(
            "The relay refused the account's access token".to_string(),
        ));
    };
    let quota_per_unit = get_json(client, &format!("{origin}/api/status"), &[])
        .await
        .ok()
        .flatten()
        .and_then(|s| s.pointer("/data/quota_per_unit").and_then(number))
        .filter(|q| *q > 0.0)
        .unwrap_or(DEFAULT_QUOTA_PER_UNIT);
    Ok(parse_account(&body, quota_per_unit))
}

/// Tries each balance call on the relay; `None` when none of them answers.
async fn read_units(
    client: &Client,
    origin: &str,
    key: &str,
    config: Option<&RelayBalanceConfig>,
) -> Result<Option<Answer>, String> {
    let account = config.and_then(|c| {
        let token = c
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())?;
        let user = c
            .user_id
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())?;
        Some((token, user))
    });
    if let Some((token, user_id)) = account {
        return read_account(client, origin, token, user_id).await.map(Some);
    }

    let bearer = format!("Bearer {key}");
    let auth = [("Authorization", bearer.as_str())];
    if let Some(answer) = get_json(client, &format!("{origin}/v1/usage"), &auth)
        .await?
        .as_ref()
        .and_then(parse_usage)
    {
        return Ok(Some(answer));
    }
    let Some(subscription) = get_json(
        client,
        &format!("{origin}/v1/dashboard/billing/subscription"),
        &auth,
    )
    .await?
    else {
        return Ok(None);
    };
    let usage = match subscription.get("hard_limit_usd").and_then(number) {
        None => return Ok(None),
        Some(limit) if limit >= UNLIMITED_USD => None,
        Some(_) => {
            get_json(
                client,
                &format!("{origin}/v1/dashboard/billing/usage"),
                &auth,
            )
            .await?
        }
    };
    Ok(parse_billing(&subscription, usage.as_ref()))
}

/// The relay's balance for this endpoint and key; `None` when the relay has
/// none of the balance calls.
pub(crate) async fn read_relay_balance(
    client: &Client,
    base_url: &str,
    key: &str,
    config: Option<&RelayBalanceConfig>,
) -> Option<RelayBalance> {
    let origin = origin(base_url)?;
    let rate = units_per_dollar(base_url, config);
    let report = |success, error, units: Option<f64>, source: Option<&str>| RelayBalance {
        success,
        error,
        balance: units.map(|u| u / rate),
        units,
        units_per_dollar: rate,
        source: source.map(str::to_string),
        queried_at: chrono::Utc::now().timestamp_millis(),
    };
    match read_units(client, &origin, key, config).await {
        Ok(None) => None,
        Ok(Some(Answer::Units(units, source))) => {
            Some(report(true, None, Some(units), Some(source)))
        }
        Ok(Some(Answer::Refused(error))) => Some(report(false, Some(error), None, None)),
        Err(error) => Some(report(false, Some(error), None, None)),
    }
}

/// The endpoint and key of a card that may be a relay: not an official login
/// or a company's own API, and not a card a plan account made.
fn relay_endpoint(app: &AppType, provider: &Provider) -> Option<(String, String)> {
    if provider
        .category
        .as_deref()
        .is_some_and(|c| CATEGORIES_WITHOUT_RELAY.contains(&c) || c.starts_with("omo"))
    {
        return None;
    }
    if provider
        .meta
        .as_ref()
        .is_some_and(|m| m.account_id.is_some())
    {
        return None;
    }
    let (url, key) =
        crate::services::accounts::endpoint_and_key(app.as_str(), &provider.settings_config)?;
    if crate::services::accounts::is_company_endpoint(&url) {
        return None;
    }
    Some((url, key))
}

/// A relay card's balance; `None` for a card that is no relay, or whose relay
/// has none of the balance calls.
pub async fn card_balance(
    state: &AppState,
    app: AppType,
    id: &str,
) -> Result<Option<RelayBalance>, AppError> {
    let provider = state
        .db
        .get_provider_by_id(id, app.as_str())?
        .ok_or_else(|| AppError::Message(format!("Provider {id} not found")))?;
    let Some((url, key)) = relay_endpoint(&app, &provider) else {
        return Ok(None);
    };
    let meta = provider.meta.as_ref();
    let client =
        crate::proxy::http_client::get_for_provider(meta.and_then(|m| m.proxy_config.as_ref()))
            .map_err(AppError::Message)?;
    Ok(read_relay_balance(
        &client,
        &url,
        &key,
        meta.and_then(|m| m.relay_balance.as_ref()),
    )
    .await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_relay_is_reached_at_its_root_and_its_known_rate_applies() {
        assert_eq!(
            origin("https://kuaiapi.net/v1/").as_deref(),
            Some("https://kuaiapi.net")
        );
        assert_eq!(
            origin("http://127.0.0.1:8080/api/anthropic").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(origin("not a url"), None);
        assert_eq!(units_per_dollar("https://kuaiapi.net", None), 6.5);
        assert_eq!(units_per_dollar("https://api.kuaiapi.net/v1", None), 6.5);
        assert_eq!(units_per_dollar("https://notkuaiapi.net", None), 1.0);
        let own = RelayBalanceConfig {
            units_per_dollar: Some(7.0),
            ..Default::default()
        };
        assert_eq!(units_per_dollar("https://kuaiapi.net", Some(&own)), 7.0);
        let zero = RelayBalanceConfig {
            units_per_dollar: Some(0.0),
            ..Default::default()
        };
        assert_eq!(units_per_dollar("https://aiberm.com", Some(&zero)), 1.0);
    }

    #[test]
    fn each_panels_answer_is_read() {
        // KuaiAPI's /v1/usage, 2026-11-18.
        let kuai = json!({"balance": 295.92229654, "isValid": true, "mode": "unrestricted",
            "planName": "钱包余额", "remaining": 295.92229654, "unit": "USD", "usage": {}});
        assert_eq!(
            parse_usage(&kuai),
            Some(Answer::Units(295.92229654, "usage"))
        );
        assert_eq!(
            parse_usage(&json!({"isValid": true, "remaining": "12.5"})),
            Some(Answer::Units(12.5, "usage"))
        );
        assert_eq!(
            parse_usage(&json!({"isValid": false, "invalidMessage": "key disabled"})),
            Some(Answer::Refused("key disabled".to_string()))
        );
        assert_eq!(parse_usage(&json!({"object": "list", "data": []})), None);

        // Aiberm's unlimited key, 2026-11-18.
        let unlimited = json!({"object": "billing_subscription", "hard_limit_usd": 100000000});
        assert!(matches!(
            parse_billing(&unlimited, None),
            Some(Answer::Refused(_))
        ));
        let limited = json!({"hard_limit_usd": 50.0});
        assert_eq!(
            parse_billing(&limited, Some(&json!({"total_usage": 1250.0}))),
            Some(Answer::Units(37.5, "billing"))
        );
        assert_eq!(parse_billing(&limited, None), None);

        assert_eq!(
            parse_account(
                &json!({"success": true, "data": {"quota": 25_000_000, "used_quota": 1}}),
                500_000.0
            ),
            Answer::Units(50.0, "account")
        );
        assert!(matches!(
            parse_account(&json!({"success": false, "message": "access token 无效"}), 500_000.0),
            Answer::Refused(m) if m.contains("access token 无效")
        ));
    }

    /// A relay on 127.0.0.1 answering the given paths with JSON; every other
    /// path is a 404.
    async fn relay(routes: Vec<(&'static str, Value)>) -> String {
        let mut app = axum::Router::new();
        for (path, body) in routes {
            app = app.route(
                path,
                axum::routing::get(move |headers: axum::http::HeaderMap| {
                    let body = body.clone();
                    async move {
                        let auth = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string();
                        if auth.starts_with("Bearer ") {
                            (axum::http::StatusCode::OK, axum::Json(body))
                        } else {
                            (axum::http::StatusCode::UNAUTHORIZED, axum::Json(json!({})))
                        }
                    }
                }),
            );
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        url
    }

    fn client() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn a_sub2api_relay_shows_its_wallet_in_dollars() {
        let url = relay(vec![(
            "/v1/usage",
            json!({"balance": 325.0, "remaining": 325.0, "isValid": true, "unit": "USD"}),
        )])
        .await;
        let own = RelayBalanceConfig {
            units_per_dollar: Some(6.5),
            ..Default::default()
        };
        let report = read_relay_balance(&client(), &format!("{url}/v1"), "sk-test", Some(&own))
            .await
            .unwrap();
        assert!(report.success);
        assert_eq!(report.units, Some(325.0));
        assert_eq!(report.balance, Some(50.0));
        assert_eq!(report.source.as_deref(), Some("usage"));
    }

    #[tokio::test]
    async fn a_new_api_relay_shows_a_limited_key_and_explains_an_unlimited_one() {
        let limited = relay(vec![
            (
                "/v1/dashboard/billing/subscription",
                json!({"hard_limit_usd": 20.0}),
            ),
            (
                "/v1/dashboard/billing/usage",
                json!({"object": "list", "total_usage": 450.0}),
            ),
        ])
        .await;
        let report = read_relay_balance(&client(), &limited, "sk-test", None)
            .await
            .unwrap();
        assert_eq!(report.balance, Some(15.5));
        assert_eq!(report.source.as_deref(), Some("billing"));

        let unlimited = relay(vec![(
            "/v1/dashboard/billing/subscription",
            json!({"hard_limit_usd": 100000000}),
        )])
        .await;
        let report = read_relay_balance(&client(), &unlimited, "sk-test", None)
            .await
            .unwrap();
        assert!(!report.success);
        assert!(report.error.unwrap().contains("access token"));
    }

    #[tokio::test]
    async fn an_account_token_reads_the_accounts_balance() {
        let url = relay(vec![
            (
                "/api/user/self",
                json!({"success": true, "data": {"quota": 50_000_000, "used_quota": 7}}),
            ),
            ("/api/status", json!({"data": {"quota_per_unit": 500000}})),
            (
                "/v1/dashboard/billing/subscription",
                json!({"hard_limit_usd": 100000000}),
            ),
        ])
        .await;
        let config = RelayBalanceConfig {
            access_token: Some("token".into()),
            user_id: Some("42".into()),
            ..Default::default()
        };
        let report = read_relay_balance(&client(), &url, "sk-test", Some(&config))
            .await
            .unwrap();
        assert!(report.success);
        assert_eq!(report.balance, Some(100.0));
        assert_eq!(report.source.as_deref(), Some("account"));
    }

    #[tokio::test]
    async fn a_relay_without_a_balance_call_shows_nothing() {
        let url = relay(vec![("/v1/models", json!({"data": []}))]).await;
        assert_eq!(
            read_relay_balance(&client(), &url, "sk-test", None).await,
            None
        );
    }

    #[test]
    fn only_relay_cards_are_read() {
        let card = |category: Option<&str>, base: &str| Provider {
            id: "p".into(),
            name: "p".into(),
            settings_config: json!({"env": {"ANTHROPIC_BASE_URL": base, "ANTHROPIC_AUTH_TOKEN": "k"}}),
            website_url: None,
            category: category.map(str::to_string),
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        };
        assert_eq!(
            relay_endpoint(
                &AppType::Claude,
                &card(Some("third_party"), "https://kuaiapi.net")
            ),
            Some(("https://kuaiapi.net".to_string(), "k".to_string()))
        );
        assert_eq!(
            relay_endpoint(&AppType::Claude, &card(None, "https://aiberm.com")),
            Some(("https://aiberm.com".to_string(), "k".to_string()))
        );
        assert_eq!(
            relay_endpoint(
                &AppType::Claude,
                &card(Some("cn_official"), "https://x.example")
            ),
            None
        );
        assert_eq!(
            relay_endpoint(
                &AppType::Claude,
                &card(Some("third_party"), "https://api.deepseek.com/anthropic")
            ),
            None
        );
        assert_eq!(
            relay_endpoint(
                &AppType::Claude,
                &card(Some("custom"), "https://api.z.ai/api/anthropic")
            ),
            None
        );
    }
}
