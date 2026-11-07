//! Plan accounts and balance monitors.
//!
//! A plan account runs in a program through a provider of that program made
//! for it (its meta `accountId` names the account), built from what the plan
//! puts into each program (`src/config/planPrograms.json`, which the window
//! reads too). The v8 migration files existing providers under accounts by
//! their endpoint; a balance monitor reads a pay-as-you-go balance with its
//! key and works out the spend from the drops between its readings.

use crate::app_config::AppType;
use crate::database::upsert_account_on_conn;
use crate::database::{Account, BalanceReading, Database};
use crate::error::AppError;
use crate::provider::{Provider, ProviderMeta};
use crate::services::subscription::{CredentialStatus, SubscriptionQuota};
use crate::services::ProviderService;
use crate::store::AppState;
use chrono::{Datelike, Local, TimeZone};
use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;

/// Coding-plan endpoints: (URL fragment, company, plan, region). The first
/// fragment the base URL contains wins.
const PLAN_ENDPOINTS: &[(&str, &str, &str, &str)] = &[
    ("api.z.ai/api/anthropic", "zai", "coding", "intl"),
    ("api.z.ai/api/coding", "zai", "coding", "intl"),
    ("open.bigmodel.cn/api/anthropic", "zai", "coding", "cn"),
    ("open.bigmodel.cn/api/coding", "zai", "coding", "cn"),
    ("api.minimax.io/anthropic", "minimax", "m-plan", "intl"),
    ("api.minimaxi.com/anthropic", "minimax", "m-plan", "cn"),
    ("api.kimi.ai/coding", "kimi", "kimi-code", "intl"),
    ("api.kimi.com/coding", "kimi", "kimi-code", "cn"),
    (
        "token-plan.ap-southeast-1.maas.aliyuncs.com",
        "alibaba",
        "token-plan",
        "intl",
    ),
    (
        "coding-intl.dashscope.aliyuncs.com",
        "alibaba",
        "coding-plan",
        "intl",
    ),
    (
        "coding.dashscope.aliyuncs.com",
        "alibaba",
        "coding-plan",
        "cn",
    ),
    (
        "tokenhub-intl.tencentcloudmaas.com/plan",
        "tencent",
        "token-plan",
        "intl",
    ),
    (
        "ark.ap-southeast.bytepluses.com/api/coding",
        "byteplus",
        "coding-plan",
        "intl",
    ),
    (
        "ark.cn-beijing.volces.com/api/coding",
        "byteplus",
        "coding-plan",
        "cn",
    ),
    ("opencode.ai/inference/go", "opencode", "go", "intl"),
    ("opencode.ai/zen/go", "opencode", "go", "intl"),
    ("api.commandcode.ai/provider", "commandcode", "plan", "intl"),
    ("api.stepfun.com/step_plan", "stepfun", "step-plan", "cn"),
];

/// Pay-as-you-go services with a balance call: (host, company, region).
const MONITOR_ENDPOINTS: &[(&str, &str, &str)] = &[
    ("api.deepseek.com", "deepseek", "intl"),
    ("api.moonshot.ai", "kimi", "intl"),
    ("api.moonshot.cn", "kimi", "cn"),
    ("api.stepfun.ai", "stepfun", "intl"),
    ("api.stepfun.com", "stepfun", "cn"),
];

fn plan_for_url(base_url: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let url = base_url.to_lowercase();
    PLAN_ENDPOINTS
        .iter()
        .find(|(fragment, ..)| url.contains(fragment))
        .map(|(_, company, plan, region)| (*company, *plan, *region))
}

fn monitor_for_url(base_url: &str) -> Option<(&'static str, &'static str)> {
    let url = base_url.to_lowercase();
    MONITOR_ENDPOINTS
        .iter()
        .find(|(host, ..)| url.contains(host))
        .map(|(_, company, region)| (*company, *region))
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut v = value;
    for key in path {
        v = v.get(key)?;
    }
    v.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn toml_value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    config.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.trim().trim_matches('"');
        (!v.is_empty()).then_some(v)
    })
}

/// A provider's endpoint and key, wherever its app keeps them.
fn endpoint_and_key(app_type: &str, settings: &Value) -> Option<(String, String)> {
    let (url, key) = match app_type {
        "claude" => (
            str_at(settings, &["env", "ANTHROPIC_BASE_URL"]),
            str_at(settings, &["env", "ANTHROPIC_AUTH_TOKEN"])
                .or_else(|| str_at(settings, &["env", "ANTHROPIC_API_KEY"])),
        ),
        "opencode" => (
            str_at(settings, &["options", "baseURL"]),
            str_at(settings, &["options", "apiKey"]),
        ),
        "openclaw" => (
            str_at(settings, &["baseUrl"]),
            str_at(settings, &["apiKey"]),
        ),
        "gemini" => (
            str_at(settings, &["env", "GOOGLE_GEMINI_BASE_URL"]),
            str_at(settings, &["env", "GEMINI_API_KEY"]),
        ),
        "codex" => (
            settings
                .get("config")
                .and_then(Value::as_str)
                .and_then(|c| toml_value(c, "base_url")),
            str_at(settings, &["auth", "OPENAI_API_KEY"]),
        ),
        "kimi" => {
            let config = settings.get("config").and_then(Value::as_str).unwrap_or("");
            (
                toml_value(config, "base_url"),
                toml_value(config, "api_key"),
            )
        }
        _ => (None, None),
    };
    Some((url?.to_string(), key?.to_string()))
}

/// v8 migration: links each provider made from a plan endpoint to a plan
/// account (one per company, plan, region and key) and gives each key of a
/// service with a balance call a monitor. Providers keep their configs; ones
/// already linked are left alone, so running it again changes nothing.
pub(crate) fn migrate_providers_to_accounts(conn: &rusqlite::Connection) -> Result<(), AppError> {
    let db_err = |e: rusqlite::Error| AppError::Database(e.to_string());
    // Databases from before the category column still reach this step.
    let category = if Database::has_column(conn, "providers", "category")? {
        "COALESCE(category, '')"
    } else {
        "''"
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, app_type, name, settings_config, meta, {category} FROM providers"
        ))
        .map_err(db_err)?;
    let rows: Vec<(String, String, String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<_, _>>()
        .map_err(db_err)?;
    drop(stmt);

    let mut accounts: Vec<Account> = {
        let mut stmt = conn
            .prepare("SELECT id, company, kind, plan, region, secret FROM accounts")
            .map_err(db_err)?;
        let existing = stmt
            .query_map([], |row| {
                Ok(Account {
                    id: row.get(0)?,
                    company: row.get(1)?,
                    kind: row.get(2)?,
                    plan: row.get(3)?,
                    region: row.get(4)?,
                    secret: row.get(5)?,
                    name: String::new(),
                    login_provider_id: None,
                    sort_index: None,
                    created_at: 0,
                    meta: Value::Null,
                })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        existing
    };
    let now = chrono::Utc::now().timestamp_millis();
    let (mut linked, mut created) = (0, 0);

    for (id, app_type, name, settings, meta, category) in rows {
        if category.starts_with("omo") || category == "official" {
            continue;
        }
        let Ok(settings) = serde_json::from_str::<Value>(&settings) else {
            continue;
        };
        let mut meta: Value =
            serde_json::from_str(&meta).unwrap_or(Value::Object(Default::default()));
        if meta.get("accountId").and_then(Value::as_str).is_some() {
            continue;
        }
        let Some((url, key)) = endpoint_and_key(&app_type, &settings) else {
            continue;
        };

        if let Some((company, plan, region)) = plan_for_url(&url) {
            let account_id = match accounts.iter().find(|a| {
                a.kind == "plan"
                    && a.company == company
                    && a.plan.as_deref() == Some(plan)
                    && a.region.as_deref() == Some(region)
                    && a.secret == key
            }) {
                Some(a) => a.id.clone(),
                None => {
                    let account = Account {
                        id: uuid::Uuid::new_v4().to_string(),
                        company: company.to_string(),
                        kind: "plan".to_string(),
                        plan: Some(plan.to_string()),
                        region: Some(region.to_string()),
                        name: name.clone(),
                        secret: key.clone(),
                        login_provider_id: None,
                        sort_index: None,
                        created_at: now,
                        meta: Value::Object(Default::default()),
                    };
                    upsert_account_on_conn(conn, &account)?;
                    created += 1;
                    let id = account.id.clone();
                    accounts.push(account);
                    id
                }
            };
            if let Some(object) = meta.as_object_mut() {
                object.insert("accountId".to_string(), Value::String(account_id));
            }
            conn.execute(
                "UPDATE providers SET meta = ?1 WHERE id = ?2 AND app_type = ?3",
                rusqlite::params![meta.to_string(), id, app_type],
            )
            .map_err(db_err)?;
            linked += 1;
        } else if let Some((company, region)) = monitor_for_url(&url) {
            let exists = accounts.iter().any(|a| {
                a.kind == "monitor"
                    && a.company == company
                    && a.region.as_deref() == Some(region)
                    && a.secret == key
            });
            if !exists {
                let account = Account {
                    id: uuid::Uuid::new_v4().to_string(),
                    company: company.to_string(),
                    kind: "monitor".to_string(),
                    plan: None,
                    region: Some(region.to_string()),
                    name: name.clone(),
                    secret: key.clone(),
                    login_provider_id: None,
                    sort_index: None,
                    created_at: now,
                    meta: Value::Object(Default::default()),
                };
                upsert_account_on_conn(conn, &account)?;
                created += 1;
                accounts.push(account);
            }
        }
    }
    log::info!("[accounts] migration: {created} accounts created, {linked} providers linked");
    Ok(())
}

// ── Plan accounts in programs ───────────────────────────────

/// What each plan puts into each program, keyed `company/plan/region`.
fn plan_programs() -> &'static Value {
    static PLANS: OnceLock<Value> = OnceLock::new();
    PLANS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../src/config/planPrograms.json"))
            .unwrap_or(Value::Null)
    })
}

fn plan_entry(account: &Account) -> Option<&'static Value> {
    let key = format!(
        "{}/{}/{}",
        account.company,
        account.plan.as_deref().unwrap_or_default(),
        account.region.as_deref().unwrap_or("intl")
    );
    plan_programs().get(key)
}

fn program_name(app: &AppType) -> &'static str {
    match app {
        AppType::Claude => "Claude Code",
        AppType::Codex => "Codex",
        AppType::Gemini => "Gemini CLI",
        AppType::Kimi => "Kimi Code",
        AppType::Cursor => "Cursor CLI",
        AppType::Antigravity => "Antigravity",
        AppType::OpenCode => "OpenCode",
        AppType::OpenClaw => "OpenClaw",
    }
}

/// The programs the account's plan can run in.
pub fn plan_apps(account: &Account) -> Vec<AppType> {
    let Some(programs) = plan_entry(account).and_then(|e| e.get("programs")) else {
        return vec![];
    };
    [AppType::Claude, AppType::OpenCode]
        .into_iter()
        .filter(|app| programs.get(app.as_str()).is_some())
        .collect()
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The OpenCode provider key of an account's provider: the company and the
/// first eight letters or digits of the account id.
fn opencode_key(account: &Account) -> String {
    let short: String = account
        .id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect::<String>()
        .to_lowercase();
    format!("{}-{short}", account.company)
}

/// The provider that puts the account into `app`: the plan's settings for that
/// program with the account's key. A provider made earlier keeps its own
/// settings and gets the account's name, key and the plan's host.
pub fn build_plan_provider(
    account: &Account,
    app: &AppType,
    existing: Option<&Provider>,
) -> Result<Provider, AppError> {
    let entry = plan_entry(account).ok_or_else(|| {
        AppError::Message(format!(
            "{} has no plan '{}' for that service",
            account.company,
            account.plan.as_deref().unwrap_or_default()
        ))
    })?;
    let program = entry
        .get("programs")
        .and_then(|p| p.get(app.as_str()))
        .ok_or_else(|| {
            AppError::Message(format!("This plan does not run in {}", program_name(app)))
        })?;
    let template = program
        .get("settingsConfig")
        .cloned()
        .unwrap_or(Value::Null);
    let key_field = opt_str(program, "keyField").unwrap_or_else(|| "ANTHROPIC_AUTH_TOKEN".into());
    let name = format!(
        "{} · {}",
        opt_str(entry, "companyName").unwrap_or_else(|| account.company.clone()),
        account.name
    );
    let mut settings = existing
        .map(|p| p.settings_config.clone())
        .unwrap_or_else(|| template.clone());
    if !settings.is_object() {
        settings = Value::Object(Default::default());
    }

    match app {
        AppType::Claude => {
            let host = template
                .pointer("/env/ANTHROPIC_BASE_URL")
                .cloned()
                .unwrap_or(Value::Null);
            let root = settings.as_object_mut().expect("object");
            let env = root
                .entry("env")
                .or_insert_with(|| Value::Object(Default::default()));
            if !env.is_object() {
                *env = Value::Object(Default::default());
            }
            let env = env.as_object_mut().expect("object");
            env.remove("ANTHROPIC_AUTH_TOKEN");
            env.remove("ANTHROPIC_API_KEY");
            env.insert(key_field.clone(), Value::String(account.secret.clone()));
            if !host.is_null() {
                env.insert("ANTHROPIC_BASE_URL".into(), host);
            }
        }
        AppType::OpenCode => {
            let host = template
                .pointer("/options/baseURL")
                .cloned()
                .unwrap_or(Value::Null);
            let root = settings.as_object_mut().expect("object");
            if existing.is_none() {
                root.insert("name".into(), Value::String(name.clone()));
            }
            let options = root
                .entry("options")
                .or_insert_with(|| Value::Object(Default::default()));
            if !options.is_object() {
                *options = Value::Object(Default::default());
            }
            let options = options.as_object_mut().expect("object");
            options.insert("apiKey".into(), Value::String(account.secret.clone()));
            if !host.is_null() {
                options.insert("baseURL".into(), host);
            }
        }
        other => {
            return Err(AppError::Message(format!(
                "A plan account cannot run in {}",
                program_name(other)
            )))
        }
    }

    let mut meta = existing
        .and_then(|p| p.meta.clone())
        .unwrap_or_else(ProviderMeta::default);
    meta.account_id = Some(account.id.clone());
    if matches!(app, AppType::Claude) {
        meta.api_key_field = (key_field != "ANTHROPIC_AUTH_TOKEN").then(|| key_field.clone());
        if existing.is_none() {
            meta.api_format = opt_str(program, "apiFormat");
        }
    }
    let id = match existing {
        Some(p) => p.id.clone(),
        None if matches!(app, AppType::OpenCode) => opencode_key(account),
        None => uuid::Uuid::new_v4().to_string(),
    };
    Ok(Provider {
        id,
        name,
        settings_config: settings,
        website_url: existing
            .and_then(|p| p.website_url.clone())
            .or_else(|| opt_str(program, "websiteUrl")),
        category: existing
            .and_then(|p| p.category.clone())
            .or_else(|| opt_str(program, "category")),
        created_at: existing
            .and_then(|p| p.created_at)
            .or_else(|| Some(chrono::Utc::now().timestamp_millis())),
        sort_index: existing.and_then(|p| p.sort_index),
        notes: existing.and_then(|p| p.notes.clone()),
        meta: Some(meta),
        icon: existing
            .and_then(|p| p.icon.clone())
            .or_else(|| opt_str(program, "icon")),
        icon_color: existing
            .and_then(|p| p.icon_color.clone())
            .or_else(|| opt_str(program, "iconColor")),
        in_failover_queue: existing.map(|p| p.in_failover_queue).unwrap_or(false),
    })
}

/// A new, empty sign-in card of each program whose accounts are its own
/// logins (`src/config/signInTemplates.json`, written from the window's
/// catalog).
fn sign_in_templates() -> &'static Value {
    static TEMPLATES: OnceLock<Value> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        serde_json::from_str(include_str!("../../../src/config/signInTemplates.json"))
            .unwrap_or(Value::Null)
    })
}

/// Adds an empty sign-in card to `app` (`claude`, `codex`, `gemini`, `kimi`,
/// `cursor`, `antigravity`). Use it in the program and sign in there: the
/// login is filed with the card in use. Answers the card's provider id.
pub fn add_sign_in_account(
    state: &AppState,
    app: AppType,
    name: Option<String>,
) -> Result<String, AppError> {
    let template = sign_in_templates().get(app.as_str()).ok_or_else(|| {
        AppError::Message(format!("{} has no sign-in of its own", program_name(&app)))
    })?;
    let provider = Provider {
        id: uuid::Uuid::new_v4().to_string(),
        name: name
            .filter(|n| !n.trim().is_empty())
            .or_else(|| opt_str(template, "name"))
            .unwrap_or_else(|| program_name(&app).to_string()),
        settings_config: template
            .get("settingsConfig")
            .cloned()
            .unwrap_or(Value::Object(Default::default())),
        website_url: None,
        category: Some("official".to_string()),
        created_at: Some(chrono::Utc::now().timestamp_millis()),
        sort_index: None,
        notes: None,
        meta: None,
        icon: opt_str(template, "icon"),
        icon_color: None,
        in_failover_queue: false,
    };
    let id = provider.id.clone();
    ProviderService::add(state, app, provider, false)?;
    Ok(id)
}

/// The provider an account put into `app`, if it has one.
pub fn linked_provider(
    state: &AppState,
    account_id: &str,
    app: &AppType,
) -> Result<Option<Provider>, AppError> {
    Ok(state
        .db
        .get_all_providers(app.as_str())?
        .into_values()
        .find(|p| p.meta.as_ref().and_then(|m| m.account_id.as_deref()) == Some(account_id)))
}

fn plan_account(state: &AppState, id: &str) -> Result<Account, AppError> {
    let account = state
        .db
        .get_account(id)?
        .ok_or_else(|| AppError::Message(format!("Account {id} not found")))?;
    if account.kind != "plan" {
        return Err(AppError::Message(
            "A balance monitor runs in no program".to_string(),
        ));
    }
    Ok(account)
}

/// Puts the account into `app`: its provider there is made, or given the
/// account's key and host, and the program switched to it (OpenCode, which
/// holds several, adds it to its config). Answers the provider's id.
pub fn use_account_in(state: &AppState, id: &str, app: AppType) -> Result<String, AppError> {
    let account = plan_account(state, id)?;
    let existing = linked_provider(state, id, &app)?;
    let provider = build_plan_provider(&account, &app, existing.as_ref())?;
    let provider_id = provider.id.clone();
    if existing.is_some() {
        ProviderService::update(state, app.clone(), None, provider)?;
    } else {
        ProviderService::add(state, app.clone(), provider, false)?;
    }
    ProviderService::switch(state, app.clone(), &provider_id)?;
    log::info!("[accounts] account {id} now used in {}", program_name(&app));
    Ok(provider_id)
}

/// Takes the account out of a program that holds several accounts at once.
pub fn remove_account_from(state: &AppState, id: &str, app: AppType) -> Result<(), AppError> {
    if !app.is_additive_mode() {
        return Err(AppError::Message(format!(
            "{} always runs on one account: use another account in it instead",
            program_name(&app)
        )));
    }
    if let Some(provider) = linked_provider(state, id, &app)? {
        ProviderService::remove_from_live_config(state, app, &provider.id)?;
    }
    Ok(())
}

/// Saves the account and gives the providers it made its new name, key and
/// host, so a program it is in use in runs on the new key at once.
pub fn save_account(state: &AppState, account: &Account) -> Result<(), AppError> {
    state.db.save_account(account)?;
    if account.kind != "plan" {
        return Ok(());
    }
    for app in [AppType::Claude, AppType::OpenCode] {
        if let Some(existing) = linked_provider(state, &account.id, &app)? {
            let provider = build_plan_provider(account, &app, Some(&existing))?;
            ProviderService::update(state, app, None, provider)?;
        }
    }
    Ok(())
}

/// Deletes the account and the providers it made; refuses while a program
/// that runs on one account at a time is using it.
pub fn delete_account(state: &AppState, id: &str) -> Result<(), AppError> {
    let mut linked = Vec::new();
    for app in [AppType::Claude, AppType::OpenCode] {
        if let Some(provider) = linked_provider(state, id, &app)? {
            if !app.is_additive_mode() {
                let current = crate::settings::get_effective_current_provider(&state.db, &app)?;
                if current.as_deref() == Some(provider.id.as_str()) {
                    return Err(AppError::Message(format!(
                        "Switch {} to another account before deleting this one",
                        program_name(&app)
                    )));
                }
            }
            linked.push((app, provider.id));
        }
    }
    for (app, provider_id) in linked {
        ProviderService::delete(state, app, &provider_id)?;
    }
    state.db.delete_account(id)
}

/// A plan account's quota windows, read with its key the way the company
/// records them; a plan without a usage call says so.
pub async fn account_quota(db: &Database, id: &str) -> Result<SubscriptionQuota, AppError> {
    let account = db
        .get_account(id)?
        .ok_or_else(|| AppError::Message(format!("Account {id} not found")))?;
    let quota_url = plan_entry(&account)
        .and_then(|e| e.get("quotaUrl"))
        .and_then(Value::as_str)
        .map(str::to_string);
    match quota_url {
        Some(url) => crate::services::coding_plan::get_coding_plan_quota(&url, &account.secret)
            .await
            .map_err(AppError::Message),
        None => Ok(SubscriptionQuota {
            tool: "coding_plan".to_string(),
            credential_status: CredentialStatus::NotFound,
            credential_message: None,
            success: false,
            tiers: vec![],
            extra_usage: None,
            error: Some("This plan shows its usage only in its console".to_string()),
            queried_at: None,
        }),
    }
}

// ── Balance monitors ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BalancePart {
    /// `granted`, `toppedUp`, `cash` or `voucher`.
    pub name: String,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BalanceReport {
    pub success: bool,
    pub error: Option<String>,
    pub currency: Option<String>,
    pub balance: Option<f64>,
    pub parts: Vec<BalancePart>,
    /// Spent since local midnight and since the first of the month, counted
    /// from the drops between readings; `None` before two readings exist.
    pub spent_today: Option<f64>,
    pub spent_month: Option<f64>,
    /// When the first reading was taken (Unix milliseconds).
    pub tracking_since: Option<i64>,
    pub queried_at: i64,
}

impl BalanceReport {
    fn failed(error: String) -> Self {
        Self {
            success: false,
            error: Some(error),
            currency: None,
            balance: None,
            parts: vec![],
            spent_today: None,
            spent_month: None,
            tracking_since: None,
            queried_at: chrono::Utc::now().timestamp_millis(),
        }
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// `user/balance`: `balance_infos[]` per currency, amounts as strings. The
/// entry with money in it is shown; with none, the first.
fn parse_deepseek(body: &Value) -> Option<(String, f64, Vec<BalancePart>)> {
    let infos = body.get("balance_infos")?.as_array()?;
    let info = infos
        .iter()
        .find(|i| i.get("total_balance").and_then(number).unwrap_or(0.0) > 0.0)
        .or_else(|| infos.first())?;
    let currency = info.get("currency")?.as_str()?.to_string();
    let total = info.get("total_balance").and_then(number)?;
    let parts = [
        ("granted", "granted_balance"),
        ("toppedUp", "topped_up_balance"),
    ]
    .into_iter()
    .filter_map(|(name, key)| {
        Some(BalancePart {
            name: name.to_string(),
            amount: info.get(key).and_then(number)?,
        })
    })
    .collect();
    Some((currency, total, parts))
}

/// `users/me/balance`: `data.available_balance` (cash plus vouchers), with
/// `cash_balance` and `voucher_balance`.
fn parse_moonshot(body: &Value) -> Option<(f64, Vec<BalancePart>)> {
    let data = body.get("data")?;
    let available = data.get("available_balance").and_then(number)?;
    let parts = [("cash", "cash_balance"), ("voucher", "voucher_balance")]
        .into_iter()
        .filter_map(|(name, key)| {
            Some(BalancePart {
                name: name.to_string(),
                amount: data.get(key).and_then(number)?,
            })
        })
        .collect();
    Some((available, parts))
}

/// `accounts`: `balance`, at the top level or under `data`.
fn parse_stepfun(body: &Value) -> Option<f64> {
    body.get("balance")
        .or_else(|| body.get("data").and_then(|d| d.get("balance")))
        .and_then(number)
}

async fn fetch(url: &str, key: &str) -> Result<Value, String> {
    let resp = crate::proxy::http_client::get()
        .get(url)
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!("Authentication failed (HTTP {status})"));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("API error (HTTP {status}): {body}"));
    }
    resp.json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))
}

/// The company's balance for this key: (currency, balance, parts).
async fn query_balance(
    company: &str,
    region: &str,
    key: &str,
) -> Result<(String, f64, Vec<BalancePart>), String> {
    if key.trim().is_empty() {
        return Err("No key".to_string());
    }
    let cn = region == "cn";
    match company {
        "deepseek" => {
            let body = fetch("https://api.deepseek.com/user/balance", key).await?;
            parse_deepseek(&body).ok_or_else(|| "Unexpected balance response".to_string())
        }
        "kimi" => {
            let host = if cn {
                "api.moonshot.cn"
            } else {
                "api.moonshot.ai"
            };
            let body = fetch(&format!("https://{host}/v1/users/me/balance"), key).await?;
            let (balance, parts) =
                parse_moonshot(&body).ok_or_else(|| "Unexpected balance response".to_string())?;
            Ok((if cn { "CNY" } else { "USD" }.to_string(), balance, parts))
        }
        "stepfun" => {
            let host = if cn {
                "api.stepfun.com"
            } else {
                "api.stepfun.ai"
            };
            let body = fetch(&format!("https://{host}/v1/accounts"), key).await?;
            let balance =
                parse_stepfun(&body).ok_or_else(|| "Unexpected balance response".to_string())?;
            Ok((if cn { "CNY" } else { "USD" }.to_string(), balance, vec![]))
        }
        other => Err(format!("{other} has no balance call")),
    }
}

/// Money that left the balance: the sum of the drops between consecutive
/// readings in the same currency. A rise is a top-up, not spend.
fn spent(readings: &[BalanceReading], currency: &str) -> f64 {
    readings
        .windows(2)
        .filter(|w| w[0].currency == currency && w[1].currency == currency)
        .map(|w| (w[0].balance - w[1].balance).max(0.0))
        .sum()
}

fn local_midnight_millis(now: chrono::DateTime<Local>) -> i64 {
    Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .earliest()
        .map(|t| t.timestamp_millis())
        .unwrap_or_else(|| now.timestamp_millis())
}

fn local_month_start_millis(now: chrono::DateTime<Local>) -> i64 {
    Local
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .earliest()
        .map(|t| t.timestamp_millis())
        .unwrap_or_else(|| now.timestamp_millis())
}

/// Reads a monitor's balance, records it and adds what was spent today and
/// this month.
pub async fn account_balance(db: &Database, account_id: &str) -> Result<BalanceReport, AppError> {
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| AppError::Message(format!("Account {account_id} not found")))?;
    let region = account.region.clone().unwrap_or_else(|| "intl".to_string());
    let (currency, balance, parts) =
        match query_balance(&account.company, &region, &account.secret).await {
            Ok(result) => result,
            Err(error) => return Ok(BalanceReport::failed(error)),
        };

    let now = Local::now();
    let observed_at = now.timestamp_millis();
    db.record_balance_reading(
        &account.id,
        &BalanceReading {
            currency: currency.clone(),
            balance,
            observed_at,
        },
    )?;
    let month = db.get_balance_readings(&account.id, local_month_start_millis(now))?;
    let today = db.get_balance_readings(&account.id, local_midnight_millis(now))?;
    let tracking_since = db.first_balance_reading_at(&account.id)?;
    let enough = month.len() >= 2;
    Ok(BalanceReport {
        success: true,
        error: None,
        spent_today: enough.then(|| spent(&today, &currency)),
        spent_month: enough.then(|| spent(&month, &currency)),
        currency: Some(currency),
        balance: Some(balance),
        parts,
        tracking_since,
        queried_at: observed_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_endpoints_are_told_by_region() {
        assert_eq!(
            plan_for_url("https://api.z.ai/api/anthropic"),
            Some(("zai", "coding", "intl"))
        );
        assert_eq!(
            plan_for_url("https://open.bigmodel.cn/api/anthropic"),
            Some(("zai", "coding", "cn"))
        );
        assert_eq!(
            plan_for_url("https://coding.dashscope.aliyuncs.com/apps/anthropic"),
            Some(("alibaba", "coding-plan", "cn"))
        );
        assert_eq!(
            plan_for_url("https://coding-intl.dashscope.aliyuncs.com/apps/anthropic"),
            Some(("alibaba", "coding-plan", "intl"))
        );
        assert_eq!(
            plan_for_url("https://opencode.ai/inference/go/openai/v1"),
            Some(("opencode", "go", "intl"))
        );
        assert_eq!(plan_for_url("https://api.deepseek.com/anthropic"), None);
        assert_eq!(
            monitor_for_url("https://api.deepseek.com/anthropic"),
            Some(("deepseek", "intl"))
        );
        assert_eq!(monitor_for_url("https://www.packyapi.com"), None);
    }

    #[test]
    fn keys_are_read_where_each_app_keeps_them() {
        assert_eq!(
            endpoint_and_key(
                "claude",
                &json!({"env": {"ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic", "ANTHROPIC_AUTH_TOKEN": "k1"}})
            ),
            Some(("https://api.z.ai/api/anthropic".into(), "k1".into()))
        );
        assert_eq!(
            endpoint_and_key(
                "claude",
                &json!({"env": {"ANTHROPIC_BASE_URL": "https://api.kimi.ai/coding/", "ANTHROPIC_API_KEY": "k2"}})
            ),
            Some(("https://api.kimi.ai/coding/".into(), "k2".into()))
        );
        assert_eq!(
            endpoint_and_key(
                "opencode",
                &json!({"options": {"baseURL": "https://api.minimax.io/anthropic/v1", "apiKey": "k3"}})
            ),
            Some(("https://api.minimax.io/anthropic/v1".into(), "k3".into()))
        );
        assert_eq!(
            endpoint_and_key(
                "codex",
                &json!({"auth": {"OPENAI_API_KEY": "k4"}, "config": "model = \"x\"\nbase_url = \"https://api.deepseek.com\"\n"})
            ),
            Some(("https://api.deepseek.com".into(), "k4".into()))
        );
        assert_eq!(endpoint_and_key("claude", &json!({"env": {}})), None);
    }

    fn migrate(rows: &[(&str, &str, Value)]) -> (rusqlite::Connection, Vec<Account>) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        for (id, app, settings) in rows {
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta) VALUES (?1, ?2, ?3, ?4, '{}')",
                rusqlite::params![id, app, format!("{id} card"), settings.to_string()],
            )
            .unwrap();
        }
        migrate_providers_to_accounts(&conn).unwrap();
        let db = Database {
            conn: std::sync::Mutex::new(conn),
        };
        let accounts = db.get_accounts().unwrap();
        (db.conn.into_inner().unwrap(), accounts)
    }

    fn account_of(conn: &rusqlite::Connection, id: &str, app: &str) -> Option<String> {
        let meta: String = conn
            .query_row(
                "SELECT meta FROM providers WHERE id = ?1 AND app_type = ?2",
                [id, app],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str::<Value>(&meta)
            .unwrap()
            .get("accountId")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    #[test]
    fn migration_files_plan_cards_under_one_account_per_key() {
        let (conn, accounts) = migrate(&[
            (
                "zai-claude",
                "claude",
                json!({"env": {"ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic", "ANTHROPIC_AUTH_TOKEN": "zk"}}),
            ),
            (
                "zai-opencode",
                "opencode",
                json!({"options": {"baseURL": "https://api.z.ai/api/coding/paas/v4", "apiKey": "zk"}}),
            ),
            (
                "deepseek",
                "claude",
                json!({"env": {"ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic", "ANTHROPIC_AUTH_TOKEN": "dk"}}),
            ),
            (
                "packy",
                "claude",
                json!({"env": {"ANTHROPIC_BASE_URL": "https://www.packyapi.com", "ANTHROPIC_AUTH_TOKEN": "pk"}}),
            ),
        ]);
        let plans: Vec<_> = accounts.iter().filter(|a| a.kind == "plan").collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].company, "zai");
        assert_eq!(plans[0].secret, "zk");
        assert_eq!(
            account_of(&conn, "zai-claude", "claude").as_deref(),
            Some(plans[0].id.as_str())
        );
        assert_eq!(
            account_of(&conn, "zai-opencode", "opencode").as_deref(),
            Some(plans[0].id.as_str())
        );

        let monitors: Vec<_> = accounts.iter().filter(|a| a.kind == "monitor").collect();
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].company, "deepseek");
        // The DeepSeek card itself is kept, unlinked; a relay is untouched.
        assert_eq!(account_of(&conn, "deepseek", "claude"), None);
        assert_eq!(account_of(&conn, "packy", "claude"), None);

        // Running it again changes nothing.
        migrate_providers_to_accounts(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    fn plan(company: &str, plan: &str, region: &str) -> Account {
        Account {
            id: "sw309-acc-1".into(),
            company: company.into(),
            kind: "plan".into(),
            plan: Some(plan.into()),
            region: Some(region.into()),
            name: "work".into(),
            secret: "k1".into(),
            login_provider_id: None,
            sort_index: None,
            created_at: 0,
            meta: Value::Null,
        }
    }

    #[test]
    fn a_plan_provider_carries_the_accounts_key_on_the_plans_host() {
        let zai = plan("zai", "coding", "intl");
        assert_eq!(plan_apps(&zai), vec![AppType::Claude, AppType::OpenCode]);
        let claude = build_plan_provider(&zai, &AppType::Claude, None).unwrap();
        assert_eq!(claude.name, "Z.ai GLM · work");
        assert_eq!(
            claude.settings_config.pointer("/env/ANTHROPIC_BASE_URL"),
            Some(&json!("https://api.z.ai/api/anthropic"))
        );
        assert_eq!(
            claude.settings_config.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
            Some(&json!("k1"))
        );
        assert_eq!(
            claude.meta.as_ref().unwrap().account_id.as_deref(),
            Some("sw309-acc-1")
        );

        let kimi_cn =
            build_plan_provider(&plan("kimi", "kimi-code", "cn"), &AppType::Claude, None).unwrap();
        assert_eq!(
            kimi_cn.settings_config.pointer("/env/ANTHROPIC_API_KEY"),
            Some(&json!("k1"))
        );
        assert_eq!(
            kimi_cn.settings_config.pointer("/env/ANTHROPIC_BASE_URL"),
            Some(&json!("https://api.kimi.com/coding/"))
        );
        assert_eq!(
            kimi_cn.meta.as_ref().unwrap().api_key_field.as_deref(),
            Some("ANTHROPIC_API_KEY")
        );

        let opencode =
            build_plan_provider(&plan("minimax", "m-plan", "cn"), &AppType::OpenCode, None)
                .unwrap();
        assert_eq!(opencode.id, "minimax-sw309acc");
        assert_eq!(
            opencode.settings_config.pointer("/options/baseURL"),
            Some(&json!("https://api.minimaxi.com/anthropic/v1"))
        );
        assert_eq!(
            opencode.settings_config.pointer("/options/apiKey"),
            Some(&json!("k1"))
        );

        let alibaba = plan("alibaba", "token-plan", "intl");
        assert_eq!(plan_apps(&alibaba), vec![AppType::Claude]);
        assert!(build_plan_provider(&alibaba, &AppType::OpenCode, None).is_err());
        assert!(build_plan_provider(&zai, &AppType::Codex, None).is_err());
    }

    #[test]
    fn every_sign_in_program_has_an_empty_card() {
        for app in ["claude", "codex", "gemini", "kimi", "cursor", "antigravity"] {
            let template = sign_in_templates().get(app).expect(app);
            assert!(template.get("settingsConfig").is_some(), "{app}");
        }
        assert!(sign_in_templates()
            .pointer("/kimi/settingsConfig/config")
            .and_then(Value::as_str)
            .is_some_and(|c| c.contains("kimi-code-env-0e4f99c69cc27850")));
        assert!(sign_in_templates().get("opencode").is_none());
    }

    #[test]
    fn a_made_provider_keeps_its_settings_and_takes_the_new_key() {
        let mut zai = plan("zai", "coding", "intl");
        let first = build_plan_provider(&zai, &AppType::Claude, None).unwrap();
        let mut existing = first.clone();
        existing
            .settings_config
            .pointer_mut("/env")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("ANTHROPIC_MODEL".into(), json!("mine"));
        zai.secret = "k2".into();
        let updated = build_plan_provider(&zai, &AppType::Claude, Some(&existing)).unwrap();
        assert_eq!(updated.id, first.id);
        assert_eq!(
            updated.settings_config.pointer("/env/ANTHROPIC_AUTH_TOKEN"),
            Some(&json!("k2"))
        );
        assert_eq!(
            updated.settings_config.pointer("/env/ANTHROPIC_MODEL"),
            Some(&json!("mine"))
        );
    }

    #[test]
    fn balances_are_read_from_each_format() {
        let (currency, total, parts) = parse_deepseek(&json!({
            "is_available": true,
            "balance_infos": [
                {"currency": "USD", "total_balance": "0.00", "granted_balance": "0.00", "topped_up_balance": "0.00"},
                {"currency": "CNY", "total_balance": "110.00", "granted_balance": "10.00", "topped_up_balance": "100.00"}
            ]
        }))
        .unwrap();
        assert_eq!(currency, "CNY");
        assert!((total - 110.0).abs() < 1e-9);
        assert_eq!(parts.len(), 2);

        let (available, parts) = parse_moonshot(&json!({
            "code": 0,
            "data": {"available_balance": 49.58, "voucher_balance": 46.58, "cash_balance": 3.0},
            "status": true
        }))
        .unwrap();
        assert!((available - 49.58).abs() < 1e-9);
        assert_eq!(parts[0].name, "cash");

        assert_eq!(
            parse_stepfun(&json!({"type": "prepaid", "balance": 12.5})),
            Some(12.5)
        );
        assert_eq!(parse_stepfun(&json!({"data": {"balance": "3"}})), Some(3.0));
    }

    #[test]
    fn spend_counts_drops_and_ignores_top_ups() {
        let r = |balance: f64, at: i64| BalanceReading {
            currency: "USD".into(),
            balance,
            observed_at: at,
        };
        let readings = [r(10.0, 1), r(8.0, 2), r(20.0, 3), r(19.5, 4)];
        assert!((spent(&readings, "USD") - 2.5).abs() < 1e-9);
        assert_eq!(spent(&readings[..1], "USD"), 0.0);
    }
}
