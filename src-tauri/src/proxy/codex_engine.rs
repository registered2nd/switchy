//! Switching the account Codex itself is signed in to, so `/status` in an
//! open Codex window shows the account the proxy serves.
//!
//! A Codex window keeps its signed-in account for its whole life, except when
//! it runs on an app-server a client can talk to: a client can sign that
//! server in with an account's tokens (`account/login/start`, type
//! `chatgptAuthTokens`, the request the ChatGPT desktop app uses), and the
//! window follows at once. OpenAI marks that request unstable, so this is an
//! experimental setting (on by default). With it off, only the proxy moves an
//! open window's requests.
//!
//! Each window gets its own server, started from its terminal by a `codex`
//! shell function Switchy installs in WSL (`CODEX_SHELL`), so the hooks Codex
//! runs keep that terminal's environment (Orca's pane variables). A shared
//! server would run them with its own. The function records each server's
//! socket in `~/.codex/switchy-sessions/<name>.sock.session`; Switchy connects
//! to each through `codex app-server proxy --sock`, signs it in to the current
//! account whenever the account or its token changes, and answers its
//! requests for new tokens, so Switchy stays the only holder renewing the
//! login. The server keeps the tokens in memory and leaves `auth.json` alone.
//! Native Windows Codex is not covered.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use futures::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::Manager;
use tokio::process::Command;
use tokio_tungstenite::tungstenite::Message;

use crate::database::Database;
use crate::provider::Provider;

/// How often the windows' servers are looked for, and each connection checks
/// the current account, when nothing wakes them sooner.
const TICK: Duration = Duration::from_secs(2);
/// How long a server that could not be served is left alone.
const RETRY: Duration = Duration::from_secs(60);

const SESSIONS_DIR: &str = "switchy-sessions";
const ENABLED_FILE: &str = "enabled";
const SHELL_FILE: &str = "switchy/codex.sh";
const BASHRC_LINE: &str =
    "[ -r \"$HOME/.codex/switchy/codex.sh\" ] && . \"$HOME/.codex/switchy/codex.sh\"  # Switchy";

/// The `codex` shell function. Runs the window on its own app-server when
/// `enabled` exists, and plain `codex` otherwise or for anything that is not
/// an interactive window.
const CODEX_SHELL: &str = r#"# Written by Switchy; replaced on each start. Gives each interactive Codex
# window its own app-server, started from this terminal, so Switchy can switch
# the account the window is signed in to while hooks (Orca's) still run with
# this terminal's environment. Plain codex while
# ~/.codex/switchy-sessions/enabled is missing (Settings -> Pool in Switchy).
codex() {
  _sw_dir="$HOME/.codex/switchy-sessions"
  if [ ! -e "$_sw_dir/enabled" ]; then command codex "$@"; return; fi
  case "${1:-}" in
    exec|e|review|login|logout|mcp|mcp-server|app-server|app|completion|sandbox|debug|apply|a|cloud|features|help|doctor|plugin|marketplace|queue|remote-control|agents|update|-h|--help|-V|--version)
      command codex "$@"; return ;;
  esac
  for _sw_arg in "$@"; do
    case "$_sw_arg" in
      --no-daemon|--add-dir|--add-dir=*|--worktree|--worktree=*|--remote|--remote=*)
        command codex "$@"; return ;;
    esac
  done
  mkdir -p "$_sw_dir" && chmod 700 "$_sw_dir"
  # Keep launch overrides when the fresh client opens the conversation picker.
  _sw_resume_opts=()
  _sw_option_value=0
  for _sw_arg in "$@"; do
    if [ "$_sw_option_value" -eq 1 ]; then
      _sw_resume_opts+=("$_sw_arg")
      _sw_option_value=0
      continue
    fi
    case "$_sw_arg" in
      --) break ;;
      -c|--config|-m|--model|-p|--profile|-s|--sandbox|-a|--ask-for-approval|-C|--cd|--local-provider|--enable|--disable)
        _sw_resume_opts+=("$_sw_arg"); _sw_option_value=1 ;;
      --config=*|--model=*|--profile=*|--sandbox=*|--ask-for-approval=*|--cd=*|--local-provider=*|--enable=*|--disable=*)
        _sw_resume_opts+=("$_sw_arg") ;;
      --approve-for-me|--dangerously-bypass-approvals-and-sandbox|--dangerously-bypass-hook-trust|--oss|--search|--no-alt-screen|--strict-config)
        _sw_resume_opts+=("$_sw_arg") ;;
    esac
  done
  while :; do
    _sw_sock="$_sw_dir/$$-$RANDOM.sock"
    ( command codex app-server --listen "unix://$_sw_sock" </dev/null >/dev/null 2>&1 & )
    _sw_n=0
    while [ ! -S "$_sw_sock" ] && [ "$_sw_n" -lt 150 ]; do sleep 0.1; _sw_n=$((_sw_n + 1)); done
    if [ ! -S "$_sw_sock" ]; then
      pkill -f "unix://$_sw_sock" 2>/dev/null
      command codex "$@"; return
    fi
    printf '%s\nrefresh-on-exit-v1\n%s\n%s\n' \
      "$_sw_sock" "$PWD" "$(command codex --version 2>/dev/null)" > "$_sw_sock.session"
    # Stops the server when this shell goes away without the window exiting.
    ( ( _sw_shell=$$
        while kill -0 "$_sw_shell" 2>/dev/null && [ -S "$_sw_sock" ]; do sleep 5; done
        pkill -f "unix://$_sw_sock"; sleep 3; pkill -KILL -f "unix://$_sw_sock"
        rm -f "$_sw_sock" "$_sw_sock.session" "$_sw_sock.refresh" ) </dev/null >/dev/null 2>&1 & )
    sleep 1
    command codex --remote "unix://$_sw_sock" "$@"
    _sw_rc=$?
    _sw_refresh=0
    if [ -e "$_sw_sock.refresh" ]; then _sw_refresh=1; fi
    rm -f "$_sw_sock.session" "$_sw_sock.refresh"
    ( ( pkill -f "unix://$_sw_sock"; sleep 3; pkill -KILL -f "unix://$_sw_sock"; rm -f "$_sw_sock" ) </dev/null >/dev/null 2>&1 & )
    if [ "$_sw_refresh" -ne 1 ]; then return $_sw_rc; fi
    # The picker preserves the right conversation even when several threads
    # were loaded on the old server. The new server uses the installed CLI.
    set -- "${_sw_resume_opts[@]}" resume
  done
}
"#;

/// Wakes the connections at once (a switch, a sign-in, a setting).
static WAKE: Lazy<tokio::sync::watch::Sender<u64>> = Lazy::new(|| tokio::sync::watch::channel(0).0);

/// Makes every connection act on the current account now instead of at its
/// next check.
pub fn nudge() {
    WAKE.send_modify(|n| *n = n.wrapping_add(1));
}

/// A WSL Codex install, reached from Windows through its `\\wsl$` path.
#[derive(Clone, Debug)]
struct Install {
    distro: String,
    /// `~/.codex` as seen from Windows.
    codex_dir: PathBuf,
}

#[derive(Clone, Debug)]
struct Session {
    marker: PathBuf,
    socket: String,
    cwd: Option<String>,
    cli_version: Option<String>,
    refresh_on_exit: bool,
}

impl Install {
    fn sessions_dir(&self) -> PathBuf {
        self.codex_dir.join(SESSIONS_DIR)
    }

    fn shell(&self, script: &str) -> Command {
        let mut command = Command::new("wsl.exe");
        command.args(["-d", &self.distro, "--", "bash", "-lc", script]);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command.kill_on_drop(true);
        command
    }

    /// `codex <args>` in this distro, as the user's login shell runs it.
    fn codex(&self, args: &str) -> Command {
        self.shell(&format!("exec codex {args}"))
    }

    /// Writes the shell function and sources it from `~/.bashrc`.
    fn install_shell(&self) -> std::io::Result<()> {
        let path = self.codex_dir.join(SHELL_FILE);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(CODEX_SHELL) {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, CODEX_SHELL)?;
        }
        let Some(home) = self.codex_dir.parent() else {
            return Ok(());
        };
        let bashrc = home.join(".bashrc");
        let current = std::fs::read_to_string(&bashrc).unwrap_or_default();
        if !current.contains(BASHRC_LINE) {
            let mut next = current;
            if !next.is_empty() && !next.ends_with('\n') {
                next.push('\n');
            }
            next.push_str(BASHRC_LINE);
            next.push('\n');
            std::fs::write(&bashrc, next)?;
            log::info!(
                "[codex_engine] WSL {}: added the Codex shell function to ~/.bashrc",
                self.distro
            );
        }
        Ok(())
    }

    /// New windows get their own server only while this file exists.
    fn set_enabled(&self, on: bool) {
        let path = self.sessions_dir().join(ENABLED_FILE);
        if on == path.exists() {
            return;
        }
        let result = if on {
            std::fs::create_dir_all(self.sessions_dir()).and_then(|_| std::fs::write(&path, ""))
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = result {
            log::warn!(
                "[codex_engine] WSL {}: could not {} per-window Codex servers: {e}",
                self.distro,
                if on { "turn on" } else { "turn off" }
            );
        }
    }

    /// Every window's server, including the launch metadata from newer shells.
    fn sessions(&self) -> Vec<Session> {
        let Ok(entries) = std::fs::read_dir(self.sessions_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "session"))
            .filter_map(|marker| {
                let text = std::fs::read_to_string(&marker).ok()?;
                let mut lines = text.lines();
                let socket = lines.next()?.trim().to_string();
                if socket.is_empty() {
                    return None;
                }
                let refresh_on_exit = lines.next() == Some("refresh-on-exit-v1");
                Some(Session {
                    marker,
                    socket,
                    cwd: refresh_on_exit.then(|| lines.next().unwrap_or("").to_string()),
                    cli_version: refresh_on_exit.then(|| lines.next().unwrap_or("").to_string()),
                    refresh_on_exit,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowUpdate {
    pub socket: String,
    pub cwd: Option<String>,
    pub server_version: Option<String>,
    pub installed_version: String,
    pub cli_changed: bool,
    pub catalog_changed: bool,
    pub refresh_on_exit: bool,
    pub queued: bool,
}

/// A custom catalog is a startup snapshot in Codex. Compare it with the
/// session marker instead of config.toml, which Switchy also edits for proxy
/// routing while a window is open.
fn selected_catalog(install: &Install) -> Option<PathBuf> {
    let text = std::fs::read_to_string(install.codex_dir.join("config.toml")).ok()?;
    let config: toml::Value = text.parse().ok()?;
    let name = config.get("model_catalog_json")?.as_str()?;
    if name.starts_with('/') {
        let mut path = PathBuf::from(format!(r"\\wsl$\{}", install.distro));
        for part in name.split('/').filter(|part| !part.is_empty()) {
            path.push(part);
        }
        Some(path)
    } else {
        Some(install.codex_dir.join(name))
    }
}

/// Open windows whose server predates the installed CLI or selected catalog.
/// The window is left running; its shell can refresh it when the user exits.
pub async fn window_updates() -> Result<Vec<WindowUpdate>, String> {
    let Some(install) = wsl_install() else {
        return Ok(Vec::new());
    };
    let sessions = install.sessions();
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let output = install
        .codex("--version")
        .output()
        .await
        .map_err(|e| format!("Could not check the installed Codex CLI: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not check the installed Codex CLI: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let installed_version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // Entry-point mtime also covers older windows whose shell marker has no
    // version, and reinstalling a CLI build with the same version string.
    let cli_modified = install
        .shell("stat -Lc %Y \"$(command -v codex)\"")
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
    let catalog_modified = selected_catalog(&install)
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|metadata| metadata.modified().ok());
    Ok(sessions
        .into_iter()
        .filter_map(|session| {
            let started = std::fs::metadata(&session.marker)
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            let cli_changed = session
                .cli_version
                .as_ref()
                .is_some_and(|version| !version.is_empty() && version != &installed_version)
                || cli_modified
                    .zip(started)
                    .is_some_and(|(binary, started)| binary > started);
            let catalog_changed = catalog_modified
                .zip(started)
                .is_some_and(|(catalog, started)| catalog > started);
            (cli_changed || catalog_changed).then(|| WindowUpdate {
                queued: session.marker.with_extension("refresh").exists(),
                socket: session.socket,
                cwd: session.cwd.filter(|cwd| !cwd.is_empty()),
                server_version: session.cli_version.filter(|version| !version.is_empty()),
                installed_version: installed_version.clone(),
                cli_changed,
                catalog_changed,
                refresh_on_exit: session.refresh_on_exit,
            })
        })
        .collect())
}

/// The shell consumes this marker only after its Codex client exits. No
/// running client or server is stopped by this command.
pub fn set_refresh_on_exit(socket: &str, queued: bool) -> Result<(), String> {
    let install = wsl_install().ok_or("No WSL Codex mirror is configured")?;
    let session = install
        .sessions()
        .into_iter()
        .find(|session| session.socket == socket)
        .ok_or("That Codex window is no longer open")?;
    if !session.refresh_on_exit {
        return Err("This Codex window predates refresh-on-exit support".to_string());
    }
    let marker = session.marker.with_extension("refresh");
    if queued {
        std::fs::write(&marker, "").map_err(|e| format!("Could not queue refresh: {e}"))
    } else if marker.exists() {
        std::fs::remove_file(&marker).map_err(|e| format!("Could not cancel refresh: {e}"))
    } else {
        Ok(())
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

fn wsl_install() -> Option<Install> {
    let codex_dir = crate::settings::get_codex_mirror_override_dir()?;
    let distro = wsl_distro(&codex_dir)?;
    Some(Install { distro, codex_dir })
}

/// Starts the task that serves the WSL install's windows. Called once at
/// startup.
pub fn start(app: tauri::AppHandle) {
    let Some(install) = wsl_install() else {
        return;
    };
    tauri::async_runtime::spawn(async move { run(app, install).await });
}

/// New windows get their own server when the setting is on and Codex is
/// routed through the proxy.
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
    if let Err(e) = install.install_shell() {
        log::warn!(
            "[codex_engine] WSL {}: could not install the Codex shell function: {e}",
            install.distro
        );
    }
    let serving: Arc<Mutex<HashSet<String>>> = Arc::default();
    let mut failed_at: HashMap<String, Instant> = HashMap::new();
    let (failure_tx, mut failure_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut wake = WAKE.subscribe();
    loop {
        install.set_enabled(wanted(&db).await);
        while let Ok(socket) = failure_rx.try_recv() {
            failed_at.insert(socket, Instant::now());
        }
        let sessions = install.sessions();
        let live: HashSet<&String> = sessions.iter().map(|session| &session.socket).collect();
        failed_at.retain(|socket, _| live.contains(socket));
        for session in sessions {
            let Session { marker, socket, .. } = session;
            if failed_at
                .get(&socket)
                .is_some_and(|at| at.elapsed() < RETRY)
            {
                continue;
            }
            if !serving
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(socket.clone())
            {
                continue;
            }
            let (db, install, serving, failure_tx) = (
                db.clone(),
                install.clone(),
                serving.clone(),
                failure_tx.clone(),
            );
            tauri::async_runtime::spawn(async move {
                if let Err(e) = serve(&db, &install, &socket, &marker).await {
                    if marker.exists() {
                        log::warn!("[codex_engine] {socket}: {e}");
                        let _ = failure_tx.send(socket.clone());
                    }
                }
                serving
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&socket);
            });
        }
        let _ = tokio::time::timeout(TICK, wake.changed()).await;
    }
}

/// Holds one window's server: signs it in to the current account whenever
/// that account or its token changes, and answers its requests for new
/// tokens. Returns when the window's session file goes away: the relay does
/// not end when its server does.
async fn serve(
    db: &Arc<Database>,
    install: &Install,
    socket: &str,
    marker: &Path,
) -> Result<(), String> {
    let mut child = install
        .codex(&format!("app-server proxy --sock '{socket}'"))
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
            .map_err(|e| format!("could not connect: {e}"))?;
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
    log::info!("[codex_engine] connected to the Codex window server {socket}");

    let mut wake = WAKE.subscribe();
    let mut next_id: i64 = 2;
    // (provider id, access token) the server was last signed in with.
    let mut signed: Option<(String, String)> = None;
    loop {
        if !marker.exists() {
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
                            "[codex_engine] signed the Codex window server {socket} in to provider={}",
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
                    return Ok(());
                };
                let text = match message {
                    Ok(Message::Text(text)) => text.to_string(),
                    Ok(Message::Close(_)) | Err(_) => return Ok(()),
                    Ok(_) => continue,
                };
                if let Some(reply) = answer(db, &text).await {
                    tx.send(send(reply)).await.map_err(|e| e.to_string())?;
                }
            }
            _ = tokio::time::timeout(TICK, wake.changed()) => {}
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

    #[test]
    fn the_shell_function_is_installed_once_and_the_sessions_are_listed() {
        let home = tempfile::TempDir::new().expect("temp home");
        let codex_dir = home.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("codex dir");
        std::fs::write(home.path().join(".bashrc"), "export A=1").expect("bashrc");
        let install = Install {
            distro: "Test".into(),
            codex_dir: codex_dir.clone(),
        };
        install.install_shell().expect("install");
        install.install_shell().expect("install again");
        let bashrc = std::fs::read_to_string(home.path().join(".bashrc")).expect("read");
        assert_eq!(bashrc.matches(BASHRC_LINE).count(), 1);
        assert!(bashrc.starts_with("export A=1\n"));
        assert_eq!(
            std::fs::read_to_string(codex_dir.join(SHELL_FILE)).expect("shell"),
            CODEX_SHELL
        );

        install.set_enabled(true);
        assert!(install.sessions_dir().join(ENABLED_FILE).exists());
        std::fs::write(
            install.sessions_dir().join("1-2.sock.session"),
            "/home/me/.codex/switchy-sessions/1-2.sock\n",
        )
        .expect("marker");
        let sessions = install.sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].socket,
            "/home/me/.codex/switchy-sessions/1-2.sock"
        );
        assert!(!sessions[0].refresh_on_exit);
        std::fs::write(
            install.sessions_dir().join("3-4.sock.session"),
            "/home/me/.codex/switchy-sessions/3-4.sock\nrefresh-on-exit-v1\n/home/me/project\ncodex-cli 0.159.0\n",
        )
        .expect("new marker");
        let sessions = install.sessions();
        let new = sessions
            .iter()
            .find(|session| session.socket.ends_with("3-4.sock"))
            .expect("new session");
        assert_eq!(new.cwd.as_deref(), Some("/home/me/project"));
        assert_eq!(new.cli_version.as_deref(), Some("codex-cli 0.159.0"));
        assert!(new.refresh_on_exit);
        install.set_enabled(false);
        assert!(!install.sessions_dir().join(ENABLED_FILE).exists());
    }
}
