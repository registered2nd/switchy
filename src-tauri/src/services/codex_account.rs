//! Codex ChatGPT-login identity helpers and the switch-away backfill guard.
//!
//! A Codex provider stores the whole of `~/.codex/auth.json` as its `auth`
//! field, so the ChatGPT OAuth tokens travel with the provider on every
//! switch. That makes the switch-away backfill the only place where an
//! account can be filed under the wrong provider, and this module holds the
//! rules that stop it — the same rules the Claude switch-away sync applies:
//!
//!   * a blanked login never overwrites a usable stored one,
//!   * an older login never overwrites a newer stored one,
//!   * a login belonging to a *different* account is never filed under the
//!     outgoing provider; it is filed under the provider that already owns
//!     that account, if there is one.

use base64::Engine;
use serde_json::Value;

use crate::provider::Provider;

/// What `auth.json` says about the ChatGPT login it carries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodexLogin {
    /// `tokens.account_id`, falling back to the `chatgpt_account_id` claim of
    /// the id token.
    pub account_id: Option<String>,
    /// `email` claim of the id token.
    pub email: Option<String>,
    /// `chatgpt_plan_type` claim of the id token (`plus`, `pro`, `team`, …).
    pub plan_type: Option<String>,
    /// `last_refresh` as unix seconds; 0 when absent or unparseable.
    pub last_refresh: i64,
    /// Non-empty access AND refresh token present.
    pub alive: bool,
}

impl CodexLogin {
    /// The key two logins are compared on: account id first, email second.
    pub fn account_key(&self) -> Option<&str> {
        self.account_id.as_deref().or(self.email.as_deref())
    }
}

/// Reads the ChatGPT login out of an `auth.json` value. `None` when the file
/// carries no `tokens` block at all (API-key mode, or a fresh preset).
pub fn inspect(auth: &Value) -> Option<CodexLogin> {
    let tokens = auth.get("tokens")?.as_object()?;
    let claims = tokens
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(decode_jwt_payload);
    let auth_claims = claims
        .as_ref()
        .and_then(|c| c.get("https://api.openai.com/auth"));

    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            auth_claims
                .and_then(|a| a.get("chatgpt_account_id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let email = claims
        .as_ref()
        .and_then(|c| c.get("email"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let plan_type = auth_claims
        .and_then(|a| a.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let last_refresh = auth
        .get("last_refresh")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
        .unwrap_or(0);
    let alive = nonempty(tokens, "access_token") && nonempty(tokens, "refresh_token");

    Some(CodexLogin {
        account_id,
        email,
        plan_type,
        last_refresh,
        alive,
    })
}

fn nonempty(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// Decodes the payload segment of a JWT without verifying it. The id token
/// is only used for display and for matching an account, never for auth.
pub fn decode_jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Copies the login-bearing fields (`tokens`, `last_refresh`) of `from` onto
/// `onto`, leaving every other key of `onto` (notably `OPENAI_API_KEY`) as it
/// was. Returns `onto` unchanged when `from` carries no `tokens`.
pub fn transplant_login(onto: &Value, from: &Value) -> Value {
    let mut result = onto.clone();
    if !result.is_object() {
        result = Value::Object(serde_json::Map::new());
    }
    let Some(tokens) = from.get("tokens") else {
        return result;
    };
    let obj = result.as_object_mut().expect("result is object");
    obj.insert("tokens".to_string(), tokens.clone());
    match from.get("last_refresh") {
        Some(v) => {
            obj.insert("last_refresh".to_string(), v.clone());
        }
        None => {
            obj.remove("last_refresh");
        }
    }
    result
}

/// What the switch-away backfill should do with the live `auth` for the
/// outgoing provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackfillVerdict {
    /// Store the live auth as-is (the usual refresh pickup, or a first login).
    Accept,
    /// Keep the provider's stored auth; the live one is blank or older.
    KeepStored(&'static str),
    /// Keep the provider's stored auth; the live login is a different account.
    /// Carries the live account key so the caller can re-home the tokens.
    ForeignAccount(String),
}

/// Pure decision over the outgoing provider's stored `auth` and the live one.
pub fn judge_backfill(stored: &Value, live: &Value) -> BackfillVerdict {
    let Some(stored_login) = inspect(stored) else {
        // Nothing stored to protect: a fresh preset, or an API-key provider.
        // Whatever is live is the user's doing while this provider was current.
        return BackfillVerdict::Accept;
    };
    if !stored_login.alive {
        return BackfillVerdict::Accept;
    }

    let Some(live_login) = inspect(live) else {
        return BackfillVerdict::KeepStored("live login removed");
    };
    if !live_login.alive {
        return BackfillVerdict::KeepStored("live login blanked");
    }

    if let (Some(a), Some(b)) = (stored_login.account_key(), live_login.account_key()) {
        if a != b {
            return BackfillVerdict::ForeignAccount(b.to_string());
        }
    }

    if live_login.last_refresh < stored_login.last_refresh {
        return BackfillVerdict::KeepStored("live login older than stored");
    }
    BackfillVerdict::Accept
}

/// Outcome of `reconcile_switch_away`, for the switch result's warnings.
#[derive(Debug, Default)]
pub struct SwitchAwayOutcome {
    pub warnings: Vec<String>,
    /// Another provider whose stored auth was updated because the live login
    /// belonged to it rather than to the outgoing provider.
    pub rehomed: Option<Provider>,
}

/// Applies the backfill rules to the outgoing Codex provider.
///
/// `outgoing` already carries the live settings (auth + config) that the
/// generic backfill wants to store; `stored` is what the provider held before.
/// On a `KeepStored`/`ForeignAccount` verdict the stored auth is put back into
/// `outgoing` (config.toml is still backfilled). On `ForeignAccount`, the live
/// login is offered to whichever other Codex provider owns that account, and
/// that provider is returned in `rehomed` so the caller can persist it.
pub fn reconcile_switch_away(
    stored: &Provider,
    outgoing: &mut Provider,
    others: &indexmap::IndexMap<String, Provider>,
) -> SwitchAwayOutcome {
    let mut outcome = SwitchAwayOutcome::default();
    let stored_auth = stored
        .settings_config
        .get("auth")
        .cloned()
        .unwrap_or(Value::Null);
    let live_auth = outgoing
        .settings_config
        .get("auth")
        .cloned()
        .unwrap_or(Value::Null);

    let put_back_stored = |outgoing: &mut Provider| {
        if let Some(obj) = outgoing.settings_config.as_object_mut() {
            obj.insert("auth".to_string(), stored_auth.clone());
        }
    };

    match judge_backfill(&stored_auth, &live_auth) {
        BackfillVerdict::Accept => {}
        BackfillVerdict::KeepStored(reason) => {
            log::info!(
                "[codex_account] backfill keeps stored login for provider={} ({reason})",
                outgoing.id
            );
            put_back_stored(outgoing);
        }
        BackfillVerdict::ForeignAccount(live_key) => {
            log::warn!(
                "[codex_account] live login ({live_key}) is not the account provider={} holds — refusing to file it there",
                outgoing.id
            );
            put_back_stored(outgoing);
            outcome
                .warnings
                .push(format!("backfill_account_mismatch:{}", outgoing.id));

            let live_login = inspect(&live_auth).unwrap_or_default();
            let owner = others.values().find(|p| {
                p.id != outgoing.id
                    && p.settings_config
                        .get("auth")
                        .and_then(inspect)
                        .and_then(|l| l.account_key().map(str::to_string))
                        .as_deref()
                        == Some(live_key.as_str())
            });
            let Some(owner) = owner else {
                log::warn!(
                    "[codex_account] no Codex provider holds account {live_key}; its refreshed login is not kept"
                );
                return outcome;
            };
            let owner_auth = owner
                .settings_config
                .get("auth")
                .cloned()
                .unwrap_or(Value::Null);
            let owner_login = inspect(&owner_auth).unwrap_or_default();
            if owner_login.alive && owner_login.last_refresh > live_login.last_refresh {
                log::info!(
                    "[codex_account] provider={} already holds a newer login for {live_key}; not re-homing",
                    owner.id
                );
                return outcome;
            }
            let mut rehomed = owner.clone();
            if let Some(obj) = rehomed.settings_config.as_object_mut() {
                obj.insert(
                    "auth".to_string(),
                    transplant_login(&owner_auth, &live_auth),
                );
            }
            log::info!(
                "[codex_account] re-homed live login for {live_key} to provider={}",
                rehomed.id
            );
            outcome.rehomed = Some(rehomed);
        }
    }
    outcome
}

/// Display-friendly identity returned to the renderer for a provider card.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexIdentity {
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    /// `last_refresh` as unix seconds; 0 when unknown.
    pub last_refresh: i64,
}

impl From<CodexLogin> for CodexIdentity {
    fn from(l: CodexLogin) -> Self {
        Self {
            account_id: l.account_id,
            email: l.email,
            plan_type: l.plan_type,
            last_refresh: l.last_refresh,
        }
    }
}

/// The account a Codex provider card should name.
///
/// The stored `auth` is the source for every provider except the current
/// one, whose live `~/.codex/auth.json` is fresher — and, for a provider that
/// has never been switched away from, the only place its login exists. In
/// that case the live login is also filed into the provider now, under the
/// same rules the switch-away backfill applies, so the record stops being
/// empty.
pub fn read_identity(
    state: &crate::store::AppState,
    provider_id: &str,
) -> Result<Option<CodexIdentity>, crate::error::AppError> {
    let Some(provider) = state.db.get_provider_by_id(provider_id, "codex")? else {
        return Ok(None);
    };
    let stored_auth = provider
        .settings_config
        .get("auth")
        .cloned()
        .unwrap_or(Value::Null);

    let is_current = crate::settings::get_effective_current_provider(
        &state.db,
        &crate::app_config::AppType::Codex,
    )?
    .as_deref()
        == Some(provider_id);
    if !is_current {
        return Ok(inspect(&stored_auth).map(CodexIdentity::from));
    }

    let live_path = crate::codex_config::get_codex_auth_path();
    let live_auth = match std::fs::read(&live_path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };
    let Some(live_login) = inspect(&live_auth) else {
        return Ok(inspect(&stored_auth).map(CodexIdentity::from));
    };

    if live_login.alive
        && inspect(&stored_auth).map(|l| l.alive) != Some(true)
        && judge_backfill(&stored_auth, &live_auth) == BackfillVerdict::Accept
    {
        let mut updated = provider.clone();
        if let Some(obj) = updated.settings_config.as_object_mut() {
            obj.insert("auth".to_string(), live_auth.clone());
        }
        if let Err(e) = state.db.save_provider("codex", &updated) {
            log::warn!(
                "[codex_account] could not file live login into provider={provider_id}: {e}"
            );
        } else {
            log::info!("[codex_account] filed live login into provider={provider_id}");
        }
    }
    Ok(Some(CodexIdentity::from(live_login)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(email: &str, account: &str, plan: &str) -> String {
        let payload = json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account,
                "chatgpt_plan_type": plan,
            }
        });
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.{}",
            b64.encode(r#"{"alg":"RS256"}"#),
            b64.encode(serde_json::to_vec(&payload).unwrap()),
            b64.encode("sig")
        )
    }

    fn auth(email: &str, account: &str, refresh: &str, last_refresh: &str) -> Value {
        json!({
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": jwt(email, account, "plus"),
                "access_token": "AAA",
                "refresh_token": refresh,
                "account_id": account,
            },
            "last_refresh": last_refresh,
        })
    }

    fn provider(id: &str, auth: Value) -> Provider {
        Provider::with_id(
            id.into(),
            id.into(),
            json!({ "auth": auth, "config": "" }),
            None,
        )
    }

    #[test]
    fn inspect_reads_identity_from_id_token() {
        let login = inspect(&auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z")).unwrap();
        assert_eq!(login.email.as_deref(), Some("a@x.io"));
        assert_eq!(login.account_id.as_deref(), Some("acct-a"));
        assert_eq!(login.plan_type.as_deref(), Some("plus"));
        assert!(login.alive);
        assert!(login.last_refresh > 0);
    }

    #[test]
    fn inspect_falls_back_to_claim_for_account_id() {
        let mut a = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        a["tokens"].as_object_mut().unwrap().remove("account_id");
        let login = inspect(&a).unwrap();
        assert_eq!(login.account_id.as_deref(), Some("acct-a"));
    }

    #[test]
    fn inspect_is_none_without_tokens() {
        assert!(inspect(&json!({})).is_none());
        assert!(inspect(&json!({ "OPENAI_API_KEY": "sk" })).is_none());
    }

    #[test]
    fn blank_refresh_token_is_not_alive() {
        let login = inspect(&auth("a@x.io", "acct-a", "", "2026-09-01T00:00:00Z")).unwrap();
        assert!(!login.alive);
    }

    #[test]
    fn first_login_into_fresh_preset_is_accepted() {
        let live = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        assert_eq!(judge_backfill(&json!({}), &live), BackfillVerdict::Accept);
    }

    #[test]
    fn same_account_refresh_is_accepted() {
        let stored = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        let live = auth("a@x.io", "acct-a", "SSS", "2026-09-02T00:00:00Z");
        assert_eq!(judge_backfill(&stored, &live), BackfillVerdict::Accept);
    }

    #[test]
    fn blanked_live_keeps_stored() {
        let stored = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        let live = auth("a@x.io", "acct-a", "", "2026-09-02T00:00:00Z");
        assert!(matches!(
            judge_backfill(&stored, &live),
            BackfillVerdict::KeepStored(_)
        ));
    }

    #[test]
    fn logged_out_live_keeps_stored() {
        let stored = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        assert!(matches!(
            judge_backfill(&stored, &json!({})),
            BackfillVerdict::KeepStored(_)
        ));
    }

    #[test]
    fn older_live_keeps_stored() {
        let stored = auth("a@x.io", "acct-a", "RRR", "2026-09-05T00:00:00Z");
        let live = auth("a@x.io", "acct-a", "SSS", "2026-09-01T00:00:00Z");
        assert!(matches!(
            judge_backfill(&stored, &live),
            BackfillVerdict::KeepStored(_)
        ));
    }

    #[test]
    fn different_account_is_foreign() {
        let stored = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        let live = auth("b@x.io", "acct-b", "SSS", "2026-09-02T00:00:00Z");
        assert_eq!(
            judge_backfill(&stored, &live),
            BackfillVerdict::ForeignAccount("acct-b".into())
        );
    }

    #[test]
    fn reconcile_rehomes_foreign_login_to_its_owner() {
        let stored_a = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        let stored_b = auth("b@x.io", "acct-b", "OLD", "2026-08-01T00:00:00Z");
        let live_b = auth("b@x.io", "acct-b", "NEW", "2026-09-02T00:00:00Z");

        let stored = provider("A", stored_a.clone());
        let mut outgoing = provider("A", live_b.clone());
        let mut others = indexmap::IndexMap::new();
        others.insert("A".to_string(), stored.clone());
        others.insert("B".to_string(), provider("B", stored_b));

        let outcome = reconcile_switch_away(&stored, &mut outgoing, &others);

        assert_eq!(outgoing.settings_config["auth"], stored_a);
        assert_eq!(outcome.warnings, vec!["backfill_account_mismatch:A"]);
        let rehomed = outcome.rehomed.expect("B receives the login");
        assert_eq!(rehomed.id, "B");
        assert_eq!(
            rehomed.settings_config["auth"]["tokens"]["refresh_token"],
            "NEW"
        );
    }

    #[test]
    fn reconcile_does_not_downgrade_owner_with_older_login() {
        let stored_a = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        let stored_b = auth("b@x.io", "acct-b", "NEWER", "2026-09-09T00:00:00Z");
        let live_b = auth("b@x.io", "acct-b", "OLDER", "2026-09-02T00:00:00Z");

        let stored = provider("A", stored_a);
        let mut outgoing = provider("A", live_b);
        let mut others = indexmap::IndexMap::new();
        others.insert("B".to_string(), provider("B", stored_b));

        let outcome = reconcile_switch_away(&stored, &mut outgoing, &others);
        assert!(outcome.rehomed.is_none());
    }

    #[test]
    fn reconcile_with_no_owner_keeps_stored_and_warns() {
        let stored_a = auth("a@x.io", "acct-a", "RRR", "2026-09-01T00:00:00Z");
        let live_c = auth("c@x.io", "acct-c", "SSS", "2026-09-02T00:00:00Z");
        let stored = provider("A", stored_a.clone());
        let mut outgoing = provider("A", live_c);
        let others = indexmap::IndexMap::new();

        let outcome = reconcile_switch_away(&stored, &mut outgoing, &others);
        assert!(outcome.rehomed.is_none());
        assert_eq!(outgoing.settings_config["auth"], stored_a);
        assert_eq!(outcome.warnings.len(), 1);
    }

    #[test]
    fn transplant_keeps_other_keys() {
        let onto = json!({ "OPENAI_API_KEY": "sk", "tokens": { "refresh_token": "old" } });
        let from = json!({ "tokens": { "refresh_token": "new" }, "last_refresh": "x" });
        let out = transplant_login(&onto, &from);
        assert_eq!(out["OPENAI_API_KEY"], "sk");
        assert_eq!(out["tokens"]["refresh_token"], "new");
        assert_eq!(out["last_refresh"], "x");
    }
}
