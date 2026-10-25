//! Antigravity CLI (`agy`) login.
//!
//! `agy` keeps its Google sign-in in the system credential store: on Windows a
//! generic Credential Manager entry, target `gemini:antigravity`, user
//! `antigravity`, whose blob is JSON
//! `{ "token": { access_token, refresh_token, expiry, … }, "auth_method", "id_token_email" }`.
//! Its settings and state under `~/.gemini/antigravity-cli/` are not
//! per-account. A Switchy Antigravity provider stores
//! `{ "credential": <that JSON | null> }`.
//!
//! macOS and Linux keep the entry in the Keychain / Secret Service, which is
//! not handled. Under tests the entry is a file in the test home instead.

use serde_json::{json, Value};

use crate::error::AppError;

const TARGET: &str = "gemini:antigravity";
const USER: &str = "antigravity";

/// `~/.gemini/antigravity-cli`, `agy`'s state and settings.
pub fn get_antigravity_dir() -> std::path::PathBuf {
    crate::config::get_home_dir()
        .join(".gemini")
        .join("antigravity-cli")
}

fn sandboxed() -> bool {
    cfg!(test) || std::env::var_os(crate::paths::ENV_TEST_HOME).is_some()
}

fn sandbox_path() -> std::path::PathBuf {
    // Unit tests run in parallel with tests that move the test home.
    if cfg!(test) {
        return std::env::temp_dir().join(format!(
            "switchy-unit-test-agy-credential-{}.json",
            std::process::id()
        ));
    }
    get_antigravity_dir().join("test-credential.json")
}

/// The stored blob, `None` when `agy` is signed out.
fn read_blob() -> Result<Option<Vec<u8>>, AppError> {
    if sandboxed() {
        let path = sandbox_path();
        return match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::io(&path, e)),
        };
    }
    platform::read()
}

fn write_blob(blob: Option<&[u8]>) -> Result<(), AppError> {
    if sandboxed() {
        let path = sandbox_path();
        return match blob {
            Some(bytes) => crate::config::atomic_write(&path, bytes),
            None if path.exists() => crate::config::delete_file(&path),
            None => Ok(()),
        };
    }
    platform::write(blob)
}

/// The live login as a provider stores it.
pub fn read_antigravity_live() -> Result<Value, AppError> {
    let credential = match read_blob()? {
        Some(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(|e| {
            AppError::Message(format!("The Antigravity credential is not JSON: {e}"))
        })?,
        None => Value::Null,
    };
    Ok(json!({ "credential": credential }))
}

/// Writes a provider's login into the credential store. A provider with no
/// login removes the entry, so `agy` opens on its sign-in screen.
pub fn write_antigravity_live(settings: &Value) -> Result<(), AppError> {
    match settings.get("credential").filter(|v| !v.is_null()) {
        Some(credential) => {
            let text = serde_json::to_string(credential)
                .map_err(|e| AppError::JsonSerialize { source: e })?;
            write_blob(Some(text.as_bytes()))
        }
        None => write_blob(None),
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{TARGET, USER};
    use crate::error::AppError;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct CredentialW {
        flags: u32,
        kind: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: FileTime,
        blob_size: u32,
        blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut std::ffi::c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn CredReadW(target: *const u16, kind: u32, flags: u32, cred: *mut *mut CredentialW)
            -> i32;
        fn CredWriteW(cred: *const CredentialW, flags: u32) -> i32;
        fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
        fn CredFree(buffer: *mut std::ffi::c_void);
    }

    const CRED_TYPE_GENERIC: u32 = 1;
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    const ERROR_NOT_FOUND: i32 = 1168;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn error(action: &str) -> AppError {
        AppError::Message(format!(
            "Could not {action} the Antigravity login in Credential Manager: {}",
            std::io::Error::last_os_error()
        ))
    }

    pub fn read() -> Result<Option<Vec<u8>>, AppError> {
        let target = wide(TARGET);
        let mut cred: *mut CredentialW = std::ptr::null_mut();
        let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) };
        if ok == 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(ERROR_NOT_FOUND) {
                return Ok(None);
            }
            return Err(error("read"));
        }
        let blob = unsafe {
            let c = &*cred;
            let bytes = if c.blob.is_null() || c.blob_size == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.blob, c.blob_size as usize).to_vec()
            };
            CredFree(cred.cast());
            bytes
        };
        Ok(Some(blob))
    }

    pub fn write(blob: Option<&[u8]>) -> Result<(), AppError> {
        let mut target = wide(TARGET);
        let Some(blob) = blob else {
            let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
            if ok == 0 && std::io::Error::last_os_error().raw_os_error() != Some(ERROR_NOT_FOUND) {
                return Err(error("remove"));
            }
            return Ok(());
        };
        let mut user = wide(USER);
        let mut bytes = blob.to_vec();
        let cred = CredentialW {
            flags: 0,
            kind: CRED_TYPE_GENERIC,
            target_name: target.as_mut_ptr(),
            comment: std::ptr::null_mut(),
            last_written: FileTime { low: 0, high: 0 },
            blob_size: bytes.len() as u32,
            blob: bytes.as_mut_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: std::ptr::null_mut(),
            target_alias: std::ptr::null_mut(),
            user_name: user.as_mut_ptr(),
        };
        if unsafe { CredWriteW(&cred, 0) } == 0 {
            return Err(error("write"));
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use crate::error::AppError;

    fn unsupported() -> AppError {
        AppError::Message(
            "Switchy switches the Antigravity login only on Windows so far".to_string(),
        )
    }

    pub fn read() -> Result<Option<Vec<u8>>, AppError> {
        Err(unsupported())
    }

    pub fn write(_blob: Option<&[u8]>) -> Result<(), AppError> {
        Err(unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_round_trips_and_an_empty_provider_signs_out() {
        let credential = json!({
            "token": { "access_token": "ya29.A", "refresh_token": "1//R", "expiry": "2026-10-24T17:22:33-07:00" },
            "auth_method": "consumer",
            "id_token_email": "a@x.io"
        });
        write_antigravity_live(&json!({ "credential": credential })).unwrap();
        assert_eq!(read_antigravity_live().unwrap()["credential"], credential);

        write_antigravity_live(&json!({ "credential": null })).unwrap();
        assert!(read_antigravity_live().unwrap()["credential"].is_null());
    }
}
