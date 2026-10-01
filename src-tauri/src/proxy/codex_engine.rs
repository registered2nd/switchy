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
//! shell function Switchy installs in WSL (`CODEX_SHELL`) or PowerShell on
//! Windows, so the hooks Codex
//! runs keep that terminal's environment (Orca's pane variables). A shared
//! server would run them with its own. The function records each server's
//! endpoint in `~/.codex/switchy-sessions/<name>.session`; Switchy connects
//! through a Unix socket in WSL or authenticated loopback WebSocket on Windows,
//! signs each server in to the current account whenever the account or its token changes, and answers its
//! requests for new tokens, so Switchy stays the only holder renewing the
//! login. The server keeps the tokens in memory and leaves `auth.json` alone.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use futures::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::Manager;
use tokio::process::Command;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::database::Database;
use crate::provider::Provider;

/// How often the windows' servers are looked for, and each connection checks
/// the current account, when nothing wakes them sooner.
const TICK: Duration = Duration::from_secs(2);
/// How long a server that could not be served is left alone.
const RETRY: Duration = Duration::from_secs(60);
/// How long a request to a window's server may take to answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

type WsError = tokio_tungstenite::tungstenite::Error;
type WsTx = Pin<Box<dyn futures::Sink<Message, Error = WsError> + Send>>;
type WsRx = Pin<Box<dyn futures::Stream<Item = Result<Message, WsError>> + Send>>;

const SESSIONS_DIR: &str = "switchy-sessions";
const ENABLED_FILE: &str = "enabled";
const SHELL_FILE: &str = "switchy/codex.sh";
const BASHRC_LINE: &str =
    "[ -r \"$HOME/.codex/switchy/codex.sh\" ] && . \"$HOME/.codex/switchy/codex.sh\"  # Switchy";
const WINDOWS_SHELL_FILE: &str = "switchy/codex.ps1";
const WINDOWS_PROFILE_TAG: &str = "# Switchy Codex Windows";
const CODEX_WINDOWS_SHELL: &str = include_str!("codex_windows.ps1");

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
    printf '%s\nrefresh-on-exit-v1\n%s\n%s\n%s\n' \
      "$_sw_sock" "$PWD" "$(command codex --version 2>/dev/null)" "${CODEX_HOME:-$HOME/.codex}" > "$_sw_sock.session"
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

/// A native Windows or WSL Codex install.
#[derive(Clone, Debug)]
struct Install {
    kind: InstallKind,
    /// `~/.codex` as seen from Windows.
    codex_dir: PathBuf,
}

#[derive(Clone, Debug)]
enum InstallKind {
    Windows,
    Wsl(String),
}

#[derive(Clone, Debug)]
struct Session {
    marker: PathBuf,
    socket: String,
    cwd: Option<String>,
    cli_version: Option<String>,
    config_dir: Option<PathBuf>,
    executable: Option<PathBuf>,
    refresh_on_exit: bool,
}

impl Install {
    fn name(&self) -> String {
        match &self.kind {
            InstallKind::Windows => "Windows".into(),
            InstallKind::Wsl(distro) => format!("WSL: {distro}"),
        }
    }

    fn sessions_dir(&self) -> PathBuf {
        self.codex_dir.join(SESSIONS_DIR)
    }

    fn shell(&self, script: &str) -> Command {
        let mut command = Command::new("wsl.exe");
        let InstallKind::Wsl(distro) = &self.kind else {
            unreachable!()
        };
        command.args(["-d", distro, "--", "bash", "-lc", script]);
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
        match &self.kind {
            InstallKind::Wsl(_) => self.shell(&format!("exec codex {args}")),
            InstallKind::Windows => {
                let mut command = Command::new("cmd.exe");
                command.args(["/d", "/c", "codex.cmd", args]);
                #[cfg(windows)]
                command.creation_flags(0x0800_0000);
                command.kill_on_drop(true);
                command
            }
        }
    }

    /// Writes the shell function and sources it from `~/.bashrc`.
    fn install_shell(&self) -> std::io::Result<()> {
        if matches!(self.kind, InstallKind::Windows) {
            return self.install_windows_shell();
        }
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
                self.name()
            );
        }
        Ok(())
    }

    fn install_windows_shell(&self) -> std::io::Result<()> {
        self.install_windows_shell_at(&crate::config::get_home_dir())
    }

    fn install_windows_shell_at(&self, home: &Path) -> std::io::Result<()> {
        let path = self.codex_dir.join(WINDOWS_SHELL_FILE);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(CODEX_WINDOWS_SHELL) {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, CODEX_WINDOWS_SHELL)?;
        }
        let quoted = path.to_string_lossy().replace('\'', "''");
        let line = format!(
            "if (Test-Path -LiteralPath '{quoted}') {{ . '{quoted}' }} {WINDOWS_PROFILE_TAG}"
        );
        for shell in ["WindowsPowerShell", "PowerShell"] {
            let profile = home.join("Documents").join(shell).join("profile.ps1");
            let current = std::fs::read_to_string(&profile).unwrap_or_default();
            let mut lines: Vec<&str> = current
                .lines()
                .filter(|line| !line.contains(WINDOWS_PROFILE_TAG))
                .collect();
            if lines.last().is_some_and(|line| !line.is_empty()) {
                lines.push("");
            }
            let next = format!("{}{}\n", lines.join("\n"), line);
            if current != next {
                if let Some(parent) = profile.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&profile, next)?;
            }
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
                "[codex_engine] {}: could not {} per-window Codex servers: {e}",
                self.name(),
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
                    config_dir: refresh_on_exit
                        .then(|| PathBuf::from(lines.next().unwrap_or("")))
                        .filter(|path| !path.as_os_str().is_empty()),
                    executable: refresh_on_exit
                        .then(|| PathBuf::from(lines.next().unwrap_or("")))
                        .filter(|path| !path.as_os_str().is_empty()),
                    refresh_on_exit,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowUpdate {
    pub platform: String,
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
fn selected_catalog(install: &Install, session: &Session) -> Option<PathBuf> {
    let config_dir = session.config_dir.as_ref().unwrap_or(&install.codex_dir);
    let config_dir = match &install.kind {
        InstallKind::Windows => config_dir.clone(),
        InstallKind::Wsl(distro) => wsl_path(distro, &config_dir.to_string_lossy()),
    };
    let text = std::fs::read_to_string(config_dir.join("config.toml")).ok()?;
    let config: toml::Value = text.parse().ok()?;
    let name = config.get("model_catalog_json")?.as_str()?;
    match &install.kind {
        InstallKind::Wsl(distro) if name.starts_with('/') => Some(wsl_path(distro, name)),
        _ if Path::new(name).is_absolute() => Some(PathBuf::from(name)),
        _ => Some(config_dir.join(name)),
    }
}

fn wsl_path(distro: &str, path: &str) -> PathBuf {
    if path.starts_with(r"\\wsl$\") || path.starts_with(r"\\wsl.localhost\") {
        return PathBuf::from(path);
    }
    let mut result = PathBuf::from(format!(r"\\wsl$\{distro}"));
    for part in path.split('/').filter(|part| !part.is_empty()) {
        result.push(part);
    }
    result
}

/// Open windows whose server predates the installed CLI or selected catalog.
/// The window is left running; its shell can refresh it when the user exits.
pub async fn window_updates() -> Result<Vec<WindowUpdate>, String> {
    let mut updates = Vec::new();
    for install in installs() {
        updates.extend(window_updates_for(&install).await?);
    }
    Ok(updates)
}

async fn window_updates_for(install: &Install) -> Result<Vec<WindowUpdate>, String> {
    let sessions = install.sessions();
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let wsl_version = if matches!(install.kind, InstallKind::Wsl(_)) {
        Some(cli_version(install.codex("--version")).await?)
    } else {
        None
    };
    // Entry-point mtime also covers older windows whose shell marker has no
    // version, and reinstalling a CLI build with the same version string.
    let wsl_cli_modified = match &install.kind {
        InstallKind::Wsl(_) => install
            .shell("stat -Lc %Y \"$(command -v codex)\"")
            .output()
            .await
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|text| text.trim().parse::<u64>().ok())
            .map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
        InstallKind::Windows => None,
    };
    let mut updates = Vec::new();
    for session in sessions {
        let executable = session.executable.clone().or_else(native_codex_entrypoint);
        let installed_version = if let Some(version) = &wsl_version {
            version.clone()
        } else {
            let path = executable
                .as_ref()
                .ok_or("Could not find the native Codex CLI")?;
            let mut command = if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            {
                Command::new(path)
            } else {
                let mut command = Command::new("cmd.exe");
                command.args(["/d", "/c"]).arg(path);
                command
            };
            command.arg("--version");
            #[cfg(windows)]
            command.creation_flags(0x0800_0000);
            cli_version(command).await?
        };
        let cli_modified = if wsl_version.is_some() {
            wsl_cli_modified
        } else {
            executable
                .as_ref()
                .and_then(|path| std::fs::metadata(path).ok())
                .and_then(|metadata| metadata.modified().ok())
        };
        let catalog_modified = selected_catalog(install, &session)
            .and_then(|path| std::fs::metadata(path).ok())
            .and_then(|metadata| metadata.modified().ok());
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
        if cli_changed || catalog_changed {
            updates.push(WindowUpdate {
                platform: install.name(),
                queued: session.marker.with_extension("refresh").exists(),
                socket: session.socket,
                cwd: session.cwd.filter(|cwd| !cwd.is_empty()),
                server_version: session.cli_version.filter(|version| !version.is_empty()),
                installed_version,
                cli_changed,
                catalog_changed,
                refresh_on_exit: session.refresh_on_exit,
            });
        }
    }
    Ok(updates)
}

async fn cli_version(mut command: Command) -> Result<String, String> {
    let output = command
        .output()
        .await
        .map_err(|e| format!("Could not check the installed Codex CLI: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not check the installed Codex CLI: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The shell consumes this marker only after its Codex client exits. No
/// running client or server is stopped by this command.
pub fn set_refresh_on_exit(socket: &str, queued: bool) -> Result<(), String> {
    let session = installs()
        .into_iter()
        .flat_map(|install| install.sessions())
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
    Some(Install {
        kind: InstallKind::Wsl(distro),
        codex_dir,
    })
}

fn installs() -> Vec<Install> {
    let mut result = Vec::new();
    #[cfg(windows)]
    result.push(Install {
        kind: InstallKind::Windows,
        codex_dir: crate::codex_config::get_codex_config_dir(),
    });
    if let Some(install) = wsl_install() {
        result.push(install);
    }
    result
}

fn native_codex_entrypoint() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| [dir.join("codex.cmd"), dir.join("codex.exe")])
        .find(|path| path.is_file())
}

/// Starts the task that serves the WSL install's windows. Called once at
/// startup.
pub fn start(app: tauri::AppHandle) {
    for install in installs() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { run(app, install).await });
    }
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
            "[codex_engine] {}: could not install the Codex shell function: {e}",
            install.name()
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
/// that account or its token changes, once none of its conversations is
/// mid-turn, and answers its requests for new tokens. Returns when the
/// window's session file goes away: the relay does not end when its server
/// does.
async fn serve(
    db: &Arc<Database>,
    install: &Install,
    socket: &str,
    marker: &Path,
) -> Result<(), String> {
    let mut relay = None;
    let (mut tx, mut rx): (WsTx, WsRx) = match &install.kind {
        InstallKind::Windows => {
            let token = std::fs::read_to_string(marker.with_extension("token"))
                .map_err(|e| format!("could not read window token: {e}"))?;
            let mut request = socket.into_client_request().map_err(|e| e.to_string())?;
            request.headers_mut().insert(
                tokio_tungstenite::tungstenite::http::header::AUTHORIZATION,
                format!("Bearer {token}").parse().map_err(
                    |e: tokio_tungstenite::tungstenite::http::header::InvalidHeaderValue| {
                        e.to_string()
                    },
                )?,
            );
            let (ws, _) = tokio_tungstenite::connect_async(request)
                .await
                .map_err(|e| format!("could not connect: {e}"))?;
            let (tx, rx) = ws.split();
            (Box::pin(tx), Box::pin(rx))
        }
        InstallKind::Wsl(_) => {
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
            relay = Some(child);
            let (tx, rx) = ws.split();
            (Box::pin(tx), Box::pin(rx))
        }
    };
    let _relay = relay;

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
    // The sign-in waiting for the window to finish its turn, logged once, and
    // when the server was last asked whether it is still mid-turn.
    let mut deferred: Option<(String, String)> = None;
    let mut checked_at = Instant::now();
    loop {
        if !marker.exists() {
            return Ok(());
        }
        if let Some(provider) = current_account(db) {
            match super::codex_pool::credentials_for(db, &provider, false).await {
                Ok(creds) => {
                    let fingerprint = (provider.id.clone(), creds.access_token.clone());
                    let busy = if signed.as_ref() == Some(&fingerprint) {
                        false
                    } else if deferred.as_ref() == Some(&fingerprint)
                        && checked_at.elapsed() < TICK
                    {
                        true
                    } else {
                        checked_at = Instant::now();
                        match window_busy(&mut tx, &mut rx, db, &mut next_id).await {
                            Ok(Some(busy)) => busy,
                            Ok(None) => return Ok(()),
                            Err(e) => {
                                log::debug!("[codex_engine] {socket}: could not read its turns: {e}");
                                false
                            }
                        }
                    };
                    if busy {
                        if deferred.as_ref() != Some(&fingerprint) {
                            log::info!(
                                "[codex_engine] {socket} is mid-turn; signing it in to provider={} once it is idle",
                                provider.id
                            );
                            deferred = Some(fingerprint);
                        }
                    } else if signed.as_ref() != Some(&fingerprint) {
                        deferred = None;
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

/// Whether any conversation loaded on a window's server is mid-turn. Signing
/// the server in cancels the requests it has in flight, and a running turn
/// ends on "application network permission was revoked", so a sign-in waits
/// until this is false. `Ok(None)` when the connection closed.
async fn window_busy(
    tx: &mut WsTx,
    rx: &mut WsRx,
    db: &Arc<Database>,
    next_id: &mut i64,
) -> Result<Option<bool>, String> {
    let Some(list) = call(tx, rx, db, next_id, "thread/loaded/list", json!({})).await? else {
        return Ok(None);
    };
    let threads: Vec<String> = list
        .pointer("/result/data")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("thread/loaded/list answered {list}"))?
        .iter()
        .filter_map(|thread| thread.as_str().map(str::to_string))
        .collect();
    for thread in threads {
        let params = json!({ "threadId": thread, "includeTurns": false });
        let Some(read) = call(tx, rx, db, next_id, "thread/read", params).await? else {
            return Ok(None);
        };
        if read.pointer("/result/thread/status/type").and_then(Value::as_str) == Some("active") {
            return Ok(Some(true));
        }
    }
    Ok(Some(false))
}

/// Sends one request to a window's server and returns its response, answering
/// the server's own requests in the meantime. `Ok(None)` when the connection
/// closed.
async fn call(
    tx: &mut WsTx,
    rx: &mut WsRx,
    db: &Arc<Database>,
    next_id: &mut i64,
    method: &str,
    params: Value,
) -> Result<Option<Value>, String> {
    let id = *next_id;
    *next_id += 1;
    let request = json!({ "id": id, "method": method, "params": params });
    tx.send(Message::Text(request.to_string().into()))
        .await
        .map_err(|e| e.to_string())?;
    let response = async {
        while let Some(message) = rx.next().await {
            let text = match message {
                Ok(Message::Text(text)) => text.to_string(),
                Ok(Message::Close(_)) | Err(_) => return Ok(None),
                Ok(_) => continue,
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value.get("method").is_none() && value.get("id").and_then(Value::as_i64) == Some(id)
            {
                return Ok(Some(value));
            }
            if let Some(reply) = answer(db, &text).await {
                tx.send(Message::Text(reply.to_string().into()))
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(None)
    };
    tokio::time::timeout(CALL_TIMEOUT, response)
        .await
        .map_err(|_| format!("{method}: no answer"))?
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
            kind: InstallKind::Wsl("Test".into()),
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

    #[test]
    fn windows_profiles_keep_their_content_and_load_the_launcher_once() {
        let home = tempfile::TempDir::new().expect("temp home");
        let install = Install {
            kind: InstallKind::Windows,
            codex_dir: home.path().join(".codex"),
        };
        let profile = home.path().join("Documents/WindowsPowerShell/profile.ps1");
        std::fs::create_dir_all(profile.parent().unwrap()).expect("profile dir");
        std::fs::write(&profile, "$env:EXAMPLE = 'kept'\n").expect("profile");
        install
            .install_windows_shell_at(home.path())
            .expect("install");
        install
            .install_windows_shell_at(home.path())
            .expect("install again");
        let text = std::fs::read_to_string(&profile).expect("read profile");
        assert!(text.starts_with("$env:EXAMPLE = 'kept'\n"));
        assert_eq!(text.matches(WINDOWS_PROFILE_TAG).count(), 1);
        assert!(home
            .path()
            .join("Documents/PowerShell/profile.ps1")
            .exists());
        assert_eq!(
            std::fs::read_to_string(install.codex_dir.join(WINDOWS_SHELL_FILE)).unwrap(),
            CODEX_WINDOWS_SHELL
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn native_window_detects_new_cli_at_its_own_launch_path() {
        let home = tempfile::TempDir::new().expect("temp home");
        let codex_dir = home.path().join(".codex");
        let install = Install {
            kind: InstallKind::Windows,
            codex_dir: codex_dir.clone(),
        };
        let sessions = install.sessions_dir();
        std::fs::create_dir_all(&sessions).expect("sessions dir");
        let binary_dir = home.path().join("CLI folder");
        std::fs::create_dir_all(&binary_dir).expect("binary dir");
        let executable = binary_dir.join("codex.cmd");
        std::fs::write(&executable, "@echo off\r\necho codex-cli 0.160.0\r\n").expect("fake CLI");
        let marker = sessions.join("window.session");
        std::fs::write(
            &marker,
            format!(
            "ws://127.0.0.1:4321\nrefresh-on-exit-v1\nC:\\Projects\ncodex-cli 0.159.0\n{}\n{}\n",
            codex_dir.display(), executable.display()
        ),
        )
        .expect("session");
        let updates = window_updates_for(&install).await.expect("updates");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].platform, "Windows");
        assert_eq!(updates[0].installed_version, "codex-cli 0.160.0");
        assert!(updates[0].cli_changed);
        assert!(updates[0].refresh_on_exit);
    }

    /// A window server that answers each request with the next scripted
    /// reply, after a notification the client must skip.
    fn scripted_server(replies: Vec<Value>) -> (WsTx, WsRx) {
        let (sent, received) = futures::channel::mpsc::unbounded::<Message>();
        std::mem::forget(received);
        let tx: WsTx = Box::pin(sent.sink_map_err(|_| WsError::ConnectionClosed));
        let mut stream = Vec::new();
        for reply in replies {
            stream.push(Ok(Message::Text(
                json!({ "method": "thread/tokenUsage/updated", "params": {} }).to_string().into(),
            )));
            stream.push(Ok(Message::Text(reply.to_string().into())));
        }
        (tx, Box::pin(futures::stream::iter(stream)))
    }

    fn read(id: i64, status: &str) -> Value {
        json!({ "id": id, "result": { "thread": { "status": { "type": status } } } })
    }

    #[tokio::test]
    async fn a_window_with_a_conversation_mid_turn_is_busy() {
        let db = Arc::new(Database::memory().unwrap());
        let (mut tx, mut rx) = scripted_server(vec![
            json!({ "id": 7, "result": { "data": ["main", "agent"] } }),
            read(8, "idle"),
            read(9, "active"),
        ]);
        let mut next_id = 7;
        let busy = window_busy(&mut tx, &mut rx, &db, &mut next_id).await;
        assert_eq!(busy, Ok(Some(true)));
    }

    #[tokio::test]
    async fn a_window_whose_conversations_are_idle_or_failed_is_not_busy() {
        let db = Arc::new(Database::memory().unwrap());
        let (mut tx, mut rx) = scripted_server(vec![
            json!({ "id": 2, "result": { "data": ["main", "agent"] } }),
            read(3, "systemError"),
            read(4, "idle"),
        ]);
        let mut next_id = 2;
        let busy = window_busy(&mut tx, &mut rx, &db, &mut next_id).await;
        assert_eq!(busy, Ok(Some(false)));
        assert_eq!(next_id, 5);
    }

    #[tokio::test]
    async fn a_server_without_the_turn_queries_is_an_error_and_a_closed_one_is_none() {
        let db = Arc::new(Database::memory().unwrap());
        let (mut tx, mut rx) = scripted_server(vec![
            json!({ "id": 2, "error": { "code": -32601, "message": "unknown method" } }),
        ]);
        let mut next_id = 2;
        assert!(window_busy(&mut tx, &mut rx, &db, &mut next_id).await.is_err());

        let (mut tx, mut rx) = scripted_server(Vec::new());
        assert_eq!(window_busy(&mut tx, &mut rx, &db, &mut next_id).await, Ok(None));
    }
}
