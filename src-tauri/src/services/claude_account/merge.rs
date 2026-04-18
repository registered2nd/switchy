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
        AppError::Message(
            "Cannot merge oauthAccount: target JSON is not an object".to_string(),
        )
    })?;
    obj.insert("oauthAccount".to_string(), oauth_account);
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
            target.get("oauthAccount").and_then(|v| v.get("emailAddress")).and_then(|v| v.as_str()),
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
}
