//! Single-key merge primitive for the `oauthAccount` block.
//!
//! Decision D-4: whole-object replace, no field-level merging — we refuse to
//! Frankenstein two accounts' fields together. Sibling top-level keys in the
//! target object (`projects`, `userID`, etc.) are preserved.

use serde_json::Value;

use crate::error::AppError;

/// Sets `target["oauthAccount"] = oauth_account`, leaving all other keys
/// untouched. Returns an error if `target` is not a JSON object.
pub fn replace_oauth_account(target: &mut Value, oauth_account: Value) -> Result<(), AppError> {
    let obj = target.as_object_mut().ok_or_else(|| {
        AppError::Message("Cannot merge oauthAccount: target JSON is not an object".to_string())
    })?;
    obj.insert("oauthAccount".to_string(), oauth_account);
    Ok(())
}

/// Root-level keys of `.claude.json` that describe the *logged-in account*
/// rather than the machine, and that carry no account identifier of their own.
/// Left behind on a swap they keep reporting the previous account's plan,
/// entitlement and usage — the account line reads correctly while everything
/// around it does not.
///
/// Deliberately excluded:
/// * `machineID`, `userID` — both identify the config instance, not the
///   account. The same account logged in on two machines carries a different
///   `userID` on each, so carrying it across a swap would stamp one install
///   with another's identity.
/// * `groveConfigCache`, `passesEligibilityCache` — keyed by account /
///   organization UUID, so a leftover entry belongs to a key nobody looks up.
/// * `cachedGrowthBookFeatures`, `cachedExperimentData`, `clientDataCacheSlots`
///   — flag caches on their own refetch timer.
/// * `projects`, `mcpServers`, onboarding and UI counters — machine state.
pub const ACCOUNT_STATE_KEYS: &[&str] = &[
    "cachedUsageUtilization",
    "hasAvailableSubscription",
    "cachedExtraUsageDisabledReason",
    "subscriptionNoticeCount",
    "passesLastSeenRemaining",
    "modelAccessCache",
    "orgModelDefaultCache",
    "penguinModeOrgEnabled",
];

/// Collects the account-scoped root keys present in `root` into a flat object.
/// Keys missing from `root` stay missing, so a restore can tell "this account
/// had no value" apart from "this account had `null`".
pub fn extract_account_state(root: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(obj) = root.as_object() {
        for key in ACCOUNT_STATE_KEYS {
            if let Some(v) = obj.get(*key) {
                out.insert((*key).to_string(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Applies a captured account-state object to `target`: every allowlisted key
/// present in `state` is written, and every allowlisted key *absent* from
/// `state` is removed. The removal half is the point — an account that never
/// had a key must not inherit the previous account's value for it.
///
/// Keys outside the allowlist are ignored in both directions, so a snapshot
/// written by a future version cannot inject arbitrary root keys.
pub fn apply_account_state(target: &mut Value, state: &Value) -> Result<(), AppError> {
    let incoming = state.as_object().ok_or_else(|| {
        AppError::Message("Cannot apply account state: snapshot JSON is not an object".to_string())
    })?;
    let obj = target.as_object_mut().ok_or_else(|| {
        AppError::Message("Cannot apply account state: target JSON is not an object".to_string())
    })?;
    for key in ACCOUNT_STATE_KEYS {
        match incoming.get(*key) {
            Some(v) => {
                obj.insert((*key).to_string(), v.clone());
            }
            None => {
                obj.remove(*key);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replaces_oauth_account_preserving_siblings() {
        let mut target = json!({
            "oauthAccount": { "emailAddress": "old@example.com" },
            "projects": { "p1": {} },
            "userID": "abc"
        });
        let incoming = json!({ "emailAddress": "new@example.com", "accountUuid": "u" });
        replace_oauth_account(&mut target, incoming).unwrap();

        assert_eq!(
            target
                .get("oauthAccount")
                .and_then(|v| v.get("emailAddress"))
                .and_then(|v| v.as_str()),
            Some("new@example.com")
        );
        assert!(target.get("projects").is_some(), "projects preserved");
        assert_eq!(
            target.get("userID").and_then(|v| v.as_str()),
            Some("abc"),
            "sibling scalars preserved"
        );
    }

    #[test]
    fn errors_when_target_is_not_an_object() {
        let mut target = json!(["a", "b"]);
        let err = replace_oauth_account(&mut target, json!({})).unwrap_err();
        assert!(err.to_string().contains("not an object"));
    }

    #[test]
    fn adds_oauth_account_when_missing() {
        let mut target = json!({ "projects": {} });
        replace_oauth_account(&mut target, json!({ "emailAddress": "x@y" })).unwrap();
        assert!(target.get("oauthAccount").is_some());
    }

    #[test]
    fn extracts_only_account_scoped_keys() {
        let root = json!({
            "hasAvailableSubscription": false,
            "machineID": "m-1",
            "userID": "u-1",
            "projects": { "p": {} },
            "oauthAccount": { "emailAddress": "a@b" }
        });
        let state = extract_account_state(&root);
        assert_eq!(
            state.get("hasAvailableSubscription").unwrap(),
            &json!(false)
        );
        assert!(state.get("machineID").is_none(), "machine state excluded");
        assert!(
            state.get("userID").is_none(),
            "userID identifies the install, not the account"
        );
        assert!(state.get("projects").is_none());
        assert!(
            state.get("oauthAccount").is_none(),
            "identity travels separately"
        );
    }

    #[test]
    fn apply_removes_keys_the_incoming_account_does_not_have() {
        let mut target = json!({
            "userID": "install-identity",
            "hasAvailableSubscription": true,
            "subscriptionNoticeCount": 4,
            "penguinModeOrgEnabled": true,
            "projects": { "p": {} },
            "machineID": "m-1"
        });
        // Incoming account has none of the entitlement fields.
        apply_account_state(&mut target, &json!({ "penguinModeOrgEnabled": false })).unwrap();

        assert_eq!(target.get("penguinModeOrgEnabled").unwrap(), &json!(false));
        assert!(
            target.get("hasAvailableSubscription").is_none(),
            "stale entitlement must not survive the swap"
        );
        assert!(target.get("subscriptionNoticeCount").is_none());
        assert!(target.get("projects").is_some(), "machine state preserved");
        assert_eq!(target.get("machineID").unwrap(), "m-1");
        assert_eq!(
            target.get("userID").unwrap(),
            "install-identity",
            "the install keeps its own userID across a swap"
        );
    }

    #[test]
    fn apply_ignores_keys_outside_the_allowlist() {
        let mut target = json!({ "projects": {} });
        apply_account_state(
            &mut target,
            &json!({ "projects": "clobbered", "subscriptionNoticeCount": 2 }),
        )
        .unwrap();
        assert_eq!(target.get("projects").unwrap(), &json!({}));
        assert_eq!(target.get("subscriptionNoticeCount").unwrap(), &json!(2));
    }

    #[test]
    fn extract_then_apply_round_trips() {
        let source = json!({
            "userID": "u-1",
            "cachedUsageUtilization": { "utilization": 25 },
            "penguinModeOrgEnabled": false,
            "machineID": "m-source"
        });
        let state = extract_account_state(&source);
        let mut target = json!({ "userID": "u-target", "machineID": "m-target" });
        apply_account_state(&mut target, &state).unwrap();

        assert_eq!(
            target.get("cachedUsageUtilization").unwrap(),
            &json!({ "utilization": 25 })
        );
        assert_eq!(target.get("penguinModeOrgEnabled").unwrap(), &json!(false));
        assert_eq!(
            target.get("machineID").unwrap(),
            "m-target",
            "machine identity stays with the machine"
        );
        assert_eq!(
            target.get("userID").unwrap(),
            "u-target",
            "so does the install's own userID"
        );
    }
}
