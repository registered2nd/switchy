//! Switching the account Codex itself is signed in to, so `/status` in an
//! open Codex window shows the account the proxy serves.
//!
//! A Codex window keeps its signed-in account for its whole life. Codex's
//! shared background server (`codex app-server daemon`) is the exception:
//! every `codex` window started while it runs attaches to it, and a client of
//! that server can sign it in with an account's tokens (`account/login/start`,
//! type `chatgptAuthTokens`, the request the ChatGPT desktop app uses), which
//! every attached window follows at once. OpenAI marks that request unstable,
//! so this is an experimental setting (on by default). With it off, only the
//! proxy moves an open window's requests.
//!
//! One connection per Codex install (Windows, and WSL when its mirror is set),
//! made through `codex app-server proxy`, which relays the server's control
//! socket over stdio. The server keeps the handed-in tokens in memory, leaving
//! `auth.json` alone, and asks this connection for new ones when OpenAI refuses
//! them, so Switchy stays the only holder that renews the login.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tauri::Manager;
use tokio::process::Command;
use tokio_tungstenite::tungstenite::Message;

use crate::database::Database;
use crate::provider::Provider;

/// How often an install's state is checked when nothing wakes it sooner.
const TICK: Duration = Duration::from_secs(2);
/// How long to wait before trying an install again after it failed.
const RETRY: Duration = Duration::from_secs(60);

/// Wakes every install's task at once (a switch, a sign-in, a setting).
static WAKE: Lazy<tokio::sync::watch::Sender<u64>> = Lazy::new(|| tokio::sync::watch::channel(0).0);

/// Makes each install act on the current account now instead of at its next
/// check.
pub fn nudge() {
    WAKE.send_modify(|n| *n = n.wrapping_add(1));
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Install {
    Windows,
    Wsl { distro: String },
}

impl Install {
    fn label(&self) -> String {
        match self {
            Install::Windows => "Windows".to_string(),
            Install::Wsl { distro } => format!("WSL {distro}"),
        }
    }

    /// `codex <args>` for this install, as the user's shell runs it.
    fn codex(&self, args: &str) -> Command {
        let mut command = match self {
            Install::Windows => {
                let mut c = Command::new("cmd");
                c.args(["/C", &format!("codex {args}")]);
                c
            }
            Install::Wsl { distro } => {
                let mut c = Command::new("wsl.exe");
                c.args([
                    "-d",
                    distro,
                    "--",
                    "bash",
                    "-lc",
                    &format!("exec codex {args}"),
                ]);
                c
            }
        };
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command.kill_on_drop(true);
        command
    }
}

/// `Ubuntu-22.04` from `\\wsl$\Ubuntu-22.04\home\...` or
/// `\\wsl.localhost\Ubuntu-22.04\home\...`.
fn wsl_distro(config_dir: &Path) -> Option<String> {
    let text = config_dir.to_string_lossy().replace('/', "\\");
    let rest = text
        .strip_prefix("\\\\wsl$\\")
        .or_else(|| text.strip_prefix("\\\\wsl.localhost\\"))?;
    rest.split('\\')
        .next()
        .filter(|d| !d.is_empty())
        .map(str::to_string)
}

fn installs() -> Vec<Install> {
    let mut installs = vec![Install::Windows];
    if let Some(distro) = crate::settings::get_codex_mirror_override_dir()
        .as_deref()
        .and_then(wsl_distro)
    {
        installs.push(Install::Wsl { distro });
    }
    installs
}

/// Starts one task per Codex install. Called once at startup.
pub fn start(app: tauri::AppHandle) {
    for install in installs() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { run(app, install).await });
    }
}

/// On when the setting is on and Codex is routed through the proxy.
async fn wanted(db: &Database) -> bool {
    let on = db
        .get_account_pool_config()
        .map(|c| c.codex_shared_session)
        .unwrap_or(false);
    on && db
        .get_proxy_config_for_app("codex")
        .await
        .map(|c| c.enabled)
        .unwrap_or(false)
}

/// The account Codex should be signed in to: the current Codex provider,
/// when it carries a ChatGPT login.
fn current_account(db: &Database) -> Option<Provider> {
    let id =
        crate::settings::get_effective_current_provider(db, &crate::app_config::AppType::Codex)
            .ok()
            .flatten()?;
    db.get_provider_by_id(&id, "codex")
        .ok()
        .flatten()
        .filter(super::codex_pool::is_chatgpt_provider)
}

async fn run(app: tauri::AppHandle, install: Install) {
    let db = app.state::<crate::store::AppState>().db.clone();
    let mut wake = WAKE.subscribe();
    let mut started_here = false;
    loop {
        if !wanted(&db).await {
            if started_here {
                stop_daemon(&install).await;
                started_here = false;
            }
            wait(&mut wake, TICK).await;
            continue;
        }
        if let Err(e) = start_daemon(&install).await {
            log::warn!(
                "[codex_engine] {}: Codex's shared server did not start: {e}",
                install.label()
            );
            wait(&mut wake, RETRY).await;
            continue;
        }
        started_here = true;
        match serve(&db, &install, &mut wake).await {
            Ok(()) => {}
            Err(e) => {
                log::warn!(
                    "[codex_engine] {}: connection to Codex's shared server ended: {e}",
                    install.label()
                );
                wait(&mut wake, RETRY).await;
            }
        }
    }
}

async fn wait(wake: &mut tokio::sync::watch::Receiver<u64>, timeout: Duration) {
    let _ = tokio::time::timeout(timeout, wake.changed()).await;
}

async fn start_daemon(install: &Install) -> Result<(), String> {
    let out = install
        .codex("app-server daemon start")
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Stops the server this task started; windows attached to it close, and
/// `codex resume` reopens them.
async fn stop_daemon(install: &Install) {
    match install.codex("app-server daemon stop").output().await {
        Ok(_) => log::info!(
            "[codex_engine] {}: stopped Codex's shared server",
            install.label()
        ),
        Err(e) => log::warn!(
            "[codex_engine] {}: could not stop Codex's shared server: {e}",
            install.label()
        ),
    }
}

/// Holds one connection: signs the server in to the current account whenever
/// that account or its token changes, and answers its requests for new
/// tokens. Returns when the feature is turned off.
async fn serve(
    db: &Arc<Database>,
    install: &Install,
    wake: &mut tokio::sync::watch::Receiver<u64>,
) -> Result<(), String> {
    let mut child = install
        .codex("app-server proxy")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (ws, _) =
        tokio_tungstenite::client_async("ws://localhost/", tokio::io::join(stdout, stdin))
            .await
            .map_err(|e| format!("handshake: {e}"))?;
    let (mut tx, mut rx) = ws.split();

    let send = |value: Value| Message::Text(value.to_string().into());
    tx.send(send(json!({
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": { "name": "switchy", "title": "Switchy", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": true },
        },
    })))
    .await
    .map_err(|e| e.to_string())?;
    tx.send(send(json!({ "method": "initialized" })))
        .await
        .map_err(|e| e.to_string())?;
    log::info!(
        "[codex_engine] {}: connected to Codex's shared server",
        install.label()
    );

    let mut next_id: i64 = 2;
    // (provider id, access token) the server was last signed in with.
    let mut signed: Option<(String, String)> = None;
    loop {
        if !wanted(db).await {
            return Ok(());
        }
        if let Some(provider) = current_account(db) {
            match super::codex_pool::credentials_for(db, &provider, false).await {
                Ok(creds) => {
                    let fingerprint = (provider.id.clone(), creds.access_token.clone());
                    if signed.as_ref() != Some(&fingerprint) {
                        tx.send(send(json!({
                            "id": next_id,
                            "method": "account/login/start",
                            "params": {
                                "type": "chatgptAuthTokens",
                                "accessToken": creds.access_token,
                                "chatgptAccountId": creds.account_id.unwrap_or_default(),
                                "chatgptPlanType": null,
                            },
                        })))
                        .await
                        .map_err(|e| e.to_string())?;
                        next_id += 1;
                        log::info!(
                            "[codex_engine] {}: signed Codex in to provider={}",
                            install.label(),
                            provider.id
                        );
                        signed = Some(fingerprint);
                    }
                }
                Err(e) => log::debug!(
                    "[codex_engine] no usable login for provider={}: {e}",
                    provider.id
                ),
            }
        }

        tokio::select! {
            message = rx.next() => {
                let Some(message) = message else {
                    return Err("the server closed the connection".into());
                };
                let text = match message.map_err(|e| e.to_string())? {
                    Message::Text(text) => text.to_string(),
                    Message::Close(_) => return Err("the server closed the connection".into()),
                    _ => continue,
                };
                if let Some(reply) = answer(db, &text).await {
                    tx.send(send(reply)).await.map_err(|e| e.to_string())?;
                }
            }
            _ = wait(wake, TICK) => {}
        }
    }
}

/// The reply to a request the server sent this client; `None` for responses
/// and notifications.
async fn answer(db: &Arc<Database>, text: &str) -> Option<Value> {
    let message: Value = serde_json::from_str(text).ok()?;
    let id = message.get("id")?.clone();
    let method = message.get("method")?.as_str()?;
    if method != "account/chatgptAuthTokens/refresh" {
        return Some(json!({
            "id": id,
            "error": { "code": -32601, "message": "not handled by Switchy" },
        }));
    }
    let Some(provider) = current_account(db) else {
        return Some(json!({
            "id": id,
            "error": { "code": -32000, "message": "no ChatGPT account is current in Switchy" },
        }));
    };
    match super::codex_pool::credentials_for(db, &provider, true).await {
        Ok(creds) => Some(json!({
            "id": id,
            "result": {
                "accessToken": creds.access_token,
                "chatgptAccountId": creds.account_id.unwrap_or_default(),
                "chatgptPlanType": null,
            },
        })),
        Err(e) => Some(json!({
            "id": id,
            "error": { "code": -32000, "message": e.to_string() },
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distro_is_read_from_a_wsl_path() {
        assert_eq!(
            wsl_distro(Path::new(r"\\wsl$\Ubuntu-22.04\home\me\.codex")).as_deref(),
            Some("Ubuntu-22.04")
        );
        assert_eq!(
            wsl_distro(Path::new(r"\\wsl.localhost\Debian\home\me\.codex")).as_deref(),
            Some("Debian")
        );
        assert_eq!(wsl_distro(Path::new(r"C:\Users\me\.codex")), None);
    }
}
