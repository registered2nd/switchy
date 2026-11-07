//! macOS login-Keychain access for Claude Code's OAuth credentials.
//!
//! On macOS, Claude Code stores its credentials blob as a generic-password
//! Keychain item (service `Claude Code-credentials`) instead of the
//! `~/.claude/.credentials.json` file it uses on Windows and Linux. Capture
//! reads that item; swap-restore writes it. We shell out to the `security`
//! CLI to stay dependency-free and consistent with `services::subscription`,
//! which already reads this same item.

use std::process::Command;

use crate::error::AppError;

/// Keychain generic-password service name Claude Code stores its blob under.
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Reads the credentials blob from the login Keychain.
///
/// Returns `Ok(None)` when the item is absent (no login yet); `Err` only when
/// the `security` binary itself can't be invoked. A non-zero exit (typically
/// 44 = item not found) maps to `Ok(None)` so callers fall back cleanly.
pub(super) fn read_credentials() -> Result<Option<Vec<u8>>, AppError> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .output()
        .map_err(|e| AppError::Message(format!("failed to invoke `security`: {e}")))?;

    if !output.status.success() {
        return Ok(None);
    }

    let mut blob = output.stdout;
    // `-w` prints the password followed by a trailing newline; strip it.
    while matches!(blob.last(), Some(b'\n') | Some(b'\r')) {
        blob.pop();
    }
    if blob.is_empty() {
        return Ok(None);
    }
    Ok(Some(blob))
}

/// Writes `blob` into the login Keychain, updating in place when an item
/// already exists so a re-capture or account swap overwrites the current
/// login rather than creating a duplicate.
///
/// The item is keyed by (service, account); we target the same account Claude
/// Code used by reading it off the existing item, falling back to the OS
/// username when no item is present yet.
pub(super) fn write_credentials(blob: &[u8]) -> Result<(), AppError> {
    let account = existing_account().unwrap_or_else(default_account);
    let password = std::str::from_utf8(blob)
        .map_err(|e| AppError::Message(format!("credentials blob is not valid UTF-8: {e}")))?;

    let output = Command::new("security")
        .args([
            "add-generic-password",
            "-U", // update the item in place if it already exists
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            &account,
            "-w",
            password,
        ])
        .output()
        .map_err(|e| AppError::Message(format!("failed to invoke `security`: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Message(format!(
            "keychain write failed (security exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )));
    }
    Ok(())
}

/// Reads the `acct` attribute of the existing Keychain item so an update
/// targets the exact record Claude Code created. Uses the attribute-only dump
/// (no `-g`/`-w`), which does not trigger a Keychain access prompt. Returns
/// `None` when the item is absent or the account can't be parsed.
fn existing_account() -> Option<String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_acct(&String::from_utf8_lossy(&output.stdout))
}

/// Extracts the account from a `security find-generic-password` attribute dump,
/// which lists it as `"acct"<blob>="<value>"`.
fn parse_acct(dump: &str) -> Option<String> {
    for line in dump.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("\"acct\"<blob>=") else {
            continue;
        };
        // Printable account → quoted form: "acct"<blob>="username".
        // Non-printable/NULL/hex forms (0x..., <NULL>) are left to the fallback.
        if let Some(inner) = rest.trim().strip_prefix('"') {
            if let Some(end) = inner.find('"') {
                let value = &inner[..end];
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Fallback account name when no existing item is present. Claude Code keys the
/// item by the OS username.
fn default_account() -> String {
    std::env::var("USER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claude".to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_acct;

    #[test]
    fn parses_quoted_account_from_attribute_dump() {
        let dump = r#"keychain: "/Users/x/Library/Keychains/login.keychain-db"
class: "genp"
attributes:
    0x00000007 <blob>="Claude Code-credentials"
    "acct"<blob>="user"
    "svce"<blob>="Claude Code-credentials"
"#;
        assert_eq!(parse_acct(dump).as_deref(), Some("user"));
    }

    #[test]
    fn returns_none_when_account_absent_or_null() {
        let dump = "class: \"genp\"\n    \"acct\"<blob>=<NULL>\n";
        assert_eq!(parse_acct(dump), None);
        assert_eq!(parse_acct("no account line here"), None);
    }
}
