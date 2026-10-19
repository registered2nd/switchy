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
const POSIX_PROFILE_TAG: &str = "# Switchy Codex shell";
const LEGACY_BASHRC_LINE: &str =
    "[ -r \"$HOME/.codex/switchy/codex.sh\" ] && . \"$HOME/.codex/switchy/codex.sh\"  # Switchy";
const MANUAL_SIGN_IN_FILE: &str = "switchy/manual_sign_in";
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
  _sw_config_dir=__SWITCHY_CODEX_CONFIG_DIR__
  if [ "$#" -eq 1 ] && [ "$1" = login ] && [ -e "$_sw_config_dir/switchy/manual_sign_in" ]; then
    command codex login --device-auth; return
  fi
  _sw_dir="$_sw_config_dir/switchy-sessions"
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
  # Where this window's Codex runs, so Switchy can end it for a refresh: it
  # types /exit into an Orca pane and ends the client in any other terminal.
  _sw_terminal=native
  if [ -n "${ORCA_PANE_KEY:-}" ] && [ -z "${TMUX:-}${STY:-}" ]; then
    _sw_terminal="orca $ORCA_PANE_KEY ${ORCA_TERMINAL_HANDLE:-}"
  fi
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
    # Codex refuses permission flags from a remote client resuming a
    # conversation, so they go to the window's server as config.
    _sw_all=("$@")
    _sw_server_opts=()
    _sw_client_args=()
    while [ $# -gt 0 ]; do
      case "$1" in
        --) _sw_client_args+=("$@"); break ;;
        -c|--config|-m|--model|-p|--profile|-C|--cd|--local-provider|--enable|--disable|-i|--image)
          _sw_client_args+=("$1"); [ $# -gt 1 ] && { shift; _sw_client_args+=("$1"); } ;;
        --dangerously-bypass-approvals-and-sandbox)
          _sw_server_opts+=(-c sandbox_mode=danger-full-access -c approval_policy=never) ;;
        --approve-for-me)
          _sw_server_opts+=(-c sandbox_mode=workspace-write -c approval_policy=on-request -c approvals_reviewer=auto_review) ;;
        -s|--sandbox) [ $# -gt 1 ] && { shift; _sw_server_opts+=(-c "sandbox_mode=$1"); } ;;
        --sandbox=*) _sw_server_opts+=(-c "sandbox_mode=${1#*=}") ;;
        -a|--ask-for-approval) [ $# -gt 1 ] && { shift; _sw_server_opts+=(-c "approval_policy=$1"); } ;;
        --ask-for-approval=*) _sw_server_opts+=(-c "approval_policy=${1#*=}") ;;
        *) _sw_client_args+=("$1") ;;
      esac
      shift
    done
    set -- "${_sw_client_args[@]}"
    _sw_sock="$_sw_dir/$$-$RANDOM.sock"
    ( command codex app-server "${_sw_server_opts[@]}" --listen "unix://$_sw_sock" </dev/null >/dev/null 2>&1 & )
    _sw_n=0
    while [ ! -S "$_sw_sock" ] && [ "$_sw_n" -lt 150 ]; do sleep 0.1; _sw_n=$((_sw_n + 1)); done
    if [ ! -S "$_sw_sock" ]; then
      pkill -f "unix://$_sw_sock" 2>/dev/null
      command codex "${_sw_all[@]}"; return
    fi
    printf '%s\nrefresh-on-exit-v1\n%s\n%s\n%s\n\nterminal=%s\n' \
      "$_sw_sock" "$PWD" "$(command codex --version 2>/dev/null)" "${CODEX_HOME:-$HOME/.codex}" \
      "$_sw_terminal" > "$_sw_sock.session"
    # Stops the server when this shell goes away without the window exiting.
    ( ( _sw_shell=$$
        while kill -0 "$_sw_shell" 2>/dev/null && [ -S "$_sw_sock" ]; do sleep 5; done
        pkill -f "unix://$_sw_sock"; sleep 3; pkill -KILL -f "unix://$_sw_sock"
        rm -f "$_sw_sock" "$_sw_sock.session" "$_sw_sock.refresh" "$_sw_sock.thread" ) </dev/null >/dev/null 2>&1 & )
    sleep 1
    _sw_tty=$(stty -g 2>/dev/null)
    command codex --remote "unix://$_sw_sock" "$@"
    _sw_rc=$?
    _sw_refresh=0
    _sw_thread=
    _sw_ended=
    if [ -e "$_sw_sock.refresh" ]; then
      _sw_refresh=1
      # "ended" when Switchy ended the client from outside.
      read -r _sw_ended < "$_sw_sock.refresh"
      # Switchy names the conversation this window was in when it is certain.
      [ -r "$_sw_sock.thread" ] && read -r _sw_thread < "$_sw_sock.thread"
    fi
    rm -f "$_sw_sock.session" "$_sw_sock.refresh" "$_sw_sock.thread"
    ( ( pkill -f "unix://$_sw_sock"; sleep 3; pkill -KILL -f "unix://$_sw_sock"; rm -f "$_sw_sock" ) </dev/null >/dev/null 2>&1 & )
    if [ "$_sw_ended" = ended ]; then
      # Undo the raw mode, keyboard and mouse modes and alternate screen the
      # ended client left.
      [ -n "$_sw_tty" ] && stty "$_sw_tty" 2>/dev/null
      printf '\033[<u\033[?2004l\033[?1004l\033[?1006l\033[?1003l\033[?1002l\033[?1000l\033[?25h\033[0m\033[?1049l'
    fi
    if [ "$_sw_refresh" -ne 1 ]; then return $_sw_rc; fi
    # The new server uses the installed CLI. Without a named conversation the
    # picker opens, so another window's conversation is never taken.
    set -- "${_sw_resume_opts[@]}" resume
    case "$_sw_thread" in
      *[!0-9a-fA-F-]*) ;;
      ????????-????-????-????-????????????) set -- "$@" "$_sw_thread" ;;
    esac
  done
}
"#;

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn posix_config_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with("//wsl$/") || text.starts_with("//wsl.localhost/") {
        return format!("/{}", text.split('/').skip(4).collect::<Vec<_>>().join("/"));
    }
    text
}

fn posix_shell(config_dir: &str) -> String {
    CODEX_SHELL.replace("__SWITCHY_CODEX_CONFIG_DIR__", &shell_quote(config_dir))
}

fn profile_text(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

fn install_posix_shell(
    codex_dir: &Path,
    home: &Path,
    config_dir: &str,
    include_zsh: bool,
) -> std::io::Result<()> {
    let path = codex_dir.join(SHELL_FILE);
    let script = posix_shell(config_dir);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, script)?;
    }
    let quoted = shell_quote(&format!("{config_dir}/{SHELL_FILE}"));
    let line = format!("[ -r {quoted} ] && . {quoted}  {POSIX_PROFILE_TAG}");
    for profile_name in if include_zsh {
        &[".bashrc", ".zshrc"][..]
    } else {
        &[".bashrc"][..]
    } {
        let profile = home.join(profile_name);
        let current = profile_text(&profile)?;
        let mut lines: Vec<&str> = current
            .lines()
            .filter(|line| !line.ends_with(POSIX_PROFILE_TAG) && *line != LEGACY_BASHRC_LINE)
            .collect();
        if lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push("");
        }
        let next = format!("{}{}\n", lines.join("\n"), line);
        if current != next {
            std::fs::create_dir_all(home)?;
            std::fs::write(&profile, next)?;
        }
    }
    Ok(())
}

/// Controls plain `codex login` in the shells installed for this machine.
pub fn configure_manual_sign_in(
    settings: &crate::settings::AppSettings,
) -> Result<(), crate::error::AppError> {
    let home = crate::config::get_home_dir();
    let native_dir = settings
        .codex_config_dir
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(crate::settings::resolve_override_path)
        .unwrap_or_else(|| home.join(".codex"));
    #[allow(unused_mut)]
    let mut directories: Vec<(PathBuf, Option<String>)> = vec![(native_dir, None)];
    #[cfg(windows)]
    {
        let mirror = settings
            .codex_mirror_config_dir
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(crate::settings::resolve_override_path)
            .or_else(|| {
                if crate::config::is_test_sandbox() {
                    None
                } else {
                    crate::commands::config::build_default_codex_mirror_dir().map(PathBuf::from)
                }
            })
            .map(crate::settings::reachable_wsl_path);
        if let Some(mirror) = mirror {
            if let Some(distro) = wsl_distro(&mirror) {
                if !directories.iter().any(|(path, _)| *path == mirror) {
                    directories.push((mirror, Some(distro)));
                }
            }
        }
    }
    for (codex_dir, distro) in directories {
        let marker = codex_dir.join(MANUAL_SIGN_IN_FILE);
        if !settings.manual_sign_in {
            if !marker.exists() {
                continue;
            }
            match std::fs::remove_file(&marker) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(crate::error::AppError::io(&marker, error)),
            }
            continue;
        }
        #[cfg(windows)]
        let installed = Install {
            kind: distro.map(InstallKind::Wsl).unwrap_or(InstallKind::Windows),
            codex_dir: codex_dir.clone(),
        }
        .install_shell();
        #[cfg(not(windows))]
        let installed = {
            let _ = distro;
            install_posix_shell(&codex_dir, &home, &posix_config_path(&codex_dir), true)
        };
        installed.map_err(|error| crate::error::AppError::io(&codex_dir, error))?;
        std::fs::write(&marker, "").map_err(|error| crate::error::AppError::io(&marker, error))?;
    }
    Ok(())
}

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
    /// Where the window's Codex runs, recorded by shells that let Switchy end
    /// it for a refresh; `None` from older shells.
    terminal: Option<Terminal>,
}

/// The terminal a window's Codex runs in.
#[derive(Clone, Debug, PartialEq)]
enum Terminal {
    /// An Orca pane: its `ORCA_PANE_KEY` and the `ORCA_TERMINAL_HANDLE` it
    /// had when Codex started.
    Orca { pane: String, handle: String },
    /// Any other terminal.
    Native,
}

impl Terminal {
    /// The `terminal=` line of a session file.
    fn parse(line: &str) -> Option<Self> {
        let mut words = line.strip_prefix("terminal=")?.split_whitespace();
        match words.next()? {
            "native" => Some(Self::Native),
            "orca" => Some(Self::Orca {
                pane: words.next()?.to_string(),
                handle: words.next().unwrap_or("").to_string(),
            }),
            _ => None,
        }
    }
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
        let home = self.posix_home()?;
        install_posix_shell(
            &self.codex_dir,
            &home,
            &posix_config_path(&self.codex_dir),
            true,
        )
    }

    fn posix_home(&self) -> std::io::Result<PathBuf> {
        let InstallKind::Wsl(distro) = &self.kind else {
            unreachable!()
        };
        // Default and custom directories inside the user's home need no subprocess.
        let path = posix_config_path(&self.codex_dir);
        if path.starts_with("/home/") {
            if let Some(user) = path.split('/').nth(2).filter(|name| !name.is_empty()) {
                return Ok(wsl_path(distro, &format!("/home/{user}")));
            }
        } else if path == "/root" || path.starts_with("/root/") {
            return Ok(wsl_path(distro, "/root"));
        } else if wsl_distro(&self.codex_dir).is_none() {
            return self
                .codex_dir
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    std::io::Error::other("Codex configuration directory has no parent")
                });
        }
        let mut command = std::process::Command::new("wsl.exe");
        command.args(["-d", distro, "--", "sh", "-c", "printf '%s' \"$HOME\""]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let output = command.output()?;
        let home = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !home.starts_with('/') {
            return Err(std::io::Error::other(format!(
                "Could not read the home directory in WSL {distro}"
            )));
        }
        Ok(wsl_path(distro, home.trim()))
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
            let current = profile_text(&profile)?;
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
                    terminal: refresh_on_exit
                        .then(|| lines.find_map(Terminal::parse))
                        .flatten(),
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
    /// Switchy ends the window's Codex itself for a refresh; otherwise a
    /// queued refresh waits for the user to exit Codex.
    pub managed: bool,
    pub queued: bool,
    /// A requested refresh waits for the window to finish its turn.
    pub waiting: bool,
}

/// Windows whose requested refresh waits for a turn to finish.
static WAITING: Lazy<Mutex<HashSet<String>>> = Lazy::new(Mutex::default);

fn set_waiting(socket: &str, waiting: bool) {
    let mut set = WAITING.lock().unwrap_or_else(|e| e.into_inner());
    if waiting {
        set.insert(socket.to_string());
    } else {
        set.remove(socket);
    }
}

fn is_waiting(socket: &str) -> bool {
    WAITING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(socket)
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
        return crate::settings::reachable_wsl_path(PathBuf::from(path));
    }
    let mut result = PathBuf::from(format!(r"\\wsl$\{distro}"));
    for part in path.split('/').filter(|part| !part.is_empty()) {
        result.push(part);
    }
    crate::settings::reachable_wsl_path(result)
}

/// Open windows whose server predates the installed CLI or selected catalog.
/// Each is left running until a refresh is requested for it.
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
                managed: session.terminal.is_some(),
                waiting: is_waiting(&session.socket),
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

/// The text Switchy writes into a window's refresh marker before it ends the
/// window's Codex from outside, so the shell restores the terminal.
const ENDED: &str = "ended";

fn open_session(socket: &str) -> Result<Session, String> {
    installs()
        .into_iter()
        .flat_map(|install| install.sessions())
        .find(|session| session.socket == socket)
        .ok_or_else(|| "That Codex terminal is no longer open".to_string())
}

/// Refreshes a window: its shell starts the installed CLI on the window's
/// conversation once its Codex exits. Where the shell recorded its terminal,
/// the window's connection ends that Codex as soon as no turn is running;
/// otherwise the refresh waits for the user to exit Codex.
pub fn request_refresh(socket: &str) -> Result<(), String> {
    let session = open_session(socket)?;
    if !session.refresh_on_exit {
        return Err("This Codex terminal predates managed refresh".to_string());
    }
    let marker = session.marker.with_extension("refresh");
    if !marker.exists() {
        std::fs::write(&marker, "").map_err(|e| format!("Could not queue refresh: {e}"))?;
    }
    // The window's connection names its conversation for the shell now.
    nudge();
    Ok(())
}

/// Cancels a refresh that has not yet ended the window's Codex.
pub fn cancel_refresh(socket: &str) -> Result<(), String> {
    let marker = open_session(socket)?.marker.with_extension("refresh");
    match std::fs::read_to_string(&marker) {
        Ok(text) if text.trim() == ENDED => {
            return Err("Codex in that terminal is already restarting".to_string())
        }
        Ok(_) => {
            std::fs::remove_file(&marker).map_err(|e| format!("Could not cancel refresh: {e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Could not cancel refresh: {e}")),
    }
    nudge();
    Ok(())
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
            let Session {
                marker,
                socket,
                terminal,
                ..
            } = session;
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
                if let Err(e) = serve(&db, &install, &socket, &marker, terminal.as_ref()).await {
                    if marker.exists() {
                        log::warn!("[codex_engine] {socket}: {e}");
                        let _ = failure_tx.send(socket.clone());
                    }
                }
                set_waiting(&socket, false);
                serving
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&socket);
            });
        }
        let _ = tokio::time::timeout(TICK, wake.changed()).await;
    }
}

/// Removes the files of a window whose server has exited, so it is not tried
/// again. The shell removes them when its window exits, but not when its
/// terminal is killed first.
fn forget(socket: &str, marker: &Path, why: &str) {
    for path in [
        marker.with_extension("token"),
        marker.with_extension("refresh"),
        marker.with_extension("thread"),
        marker.to_path_buf(),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("[codex_engine] could not remove {}: {e}", path.display()),
        }
    }
    log::info!(
        "[codex_engine] {socket}: {why}, so its window has closed; removed its session files"
    );
}

/// Whether a WSL relay that could not connect reported its window's socket
/// as removed or left with nothing listening, the two ways it is gone for
/// good.
fn wsl_socket_gone(said: &str) -> bool {
    said.contains("failed to connect to socket")
        && (said.contains("(os error 2)") || said.contains("(os error 111)"))
}

/// Holds one window's server: signs it in to the current account whenever
/// that account or its token changes, once none of its conversations is
/// mid-turn, answers its requests for new tokens, and while a refresh is
/// queued names the window's conversation for its shell to resume and, when
/// its `terminal` is known, ends its Codex once no turn is running. Returns
/// when the window's session file goes away: the relay does not end when its
/// server does.
async fn serve(
    db: &Arc<Database>,
    install: &Install,
    socket: &str,
    marker: &Path,
    terminal: Option<&Terminal>,
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
            let (ws, _) = match tokio_tungstenite::connect_async(request).await {
                Ok(connected) => connected,
                // The shell writes the session file only once its server
                // answers, so a refusal now means that server has exited.
                Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                    forget(socket, marker, "the connection was refused");
                    return Ok(());
                }
                Err(e) => return Err(format!("could not connect: {e}")),
            };
            let (tx, rx) = ws.split();
            (Box::pin(tx), Box::pin(rx))
        }
        InstallKind::Wsl(_) => {
            let mut child = install
                .codex(&format!("app-server proxy --sock '{socket}'"))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let mut stderr = child.stderr.take().ok_or("no stderr")?;
            // Read for the relay's whole life so its pipe never fills; the
            // start says why a relay that could not connect stopped.
            let said = tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let (mut start, mut chunk) = (Vec::new(), [0u8; 1024]);
                while let Ok(n) = stderr.read(&mut chunk).await {
                    if n == 0 {
                        break;
                    }
                    if start.len() < 4096 {
                        start.extend_from_slice(&chunk[..n]);
                    }
                }
                String::from_utf8_lossy(&start).into_owned()
            });
            let (ws, _) = match tokio_tungstenite::client_async(
                "ws://localhost/",
                tokio::io::join(stdout, stdin),
            )
            .await
            {
                Ok(connected) => connected,
                Err(e) => {
                    let said = tokio::time::timeout(TICK, said)
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or_default();
                    if wsl_socket_gone(&said) {
                        forget(
                            socket,
                            marker,
                            "its socket is gone or refused the connection",
                        );
                        return Ok(());
                    }
                    return Err(format!("could not connect: {e}"));
                }
            };
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
    // The conversation last named for the shell (`Some(None)`: none), when
    // that was decided, and whether the loaded conversations may have changed
    // since.
    let thread_file = marker.with_extension("thread");
    let refresh_file = marker.with_extension("refresh");
    let mut named: Option<Option<String>> = None;
    let mut named_at = Instant::now();
    let mut stale = true;
    // How far a requested refresh has got in ending the window's Codex.
    let mut ending = Ending::NotYet;
    loop {
        if !marker.exists() {
            name_conversation(&thread_file, None);
            return Ok(());
        }
        if let Some(provider) = current_account(db) {
            match super::codex_pool::credentials_for(db, &provider, false).await {
                Ok(creds) => {
                    let fingerprint = (provider.id.clone(), creds.access_token.clone());
                    let busy = if signed.as_ref() == Some(&fingerprint) {
                        false
                    } else if deferred.as_ref() == Some(&fingerprint) && checked_at.elapsed() < TICK
                    {
                        true
                    } else {
                        checked_at = Instant::now();
                        match window_busy(&mut tx, &mut rx, db, &mut next_id).await {
                            Ok(Some(busy)) => busy,
                            Ok(None) => return Ok(()),
                            Err(e) => {
                                log::debug!(
                                    "[codex_engine] {socket}: could not read its turns: {e}"
                                );
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

        if stale || named_at.elapsed() >= TICK {
            stale = false;
            named_at = Instant::now();
            let queued = refresh_file.exists();
            // The conversation to name and whether a turn is running.
            let read = if queued {
                match loaded_threads(&mut tx, &mut rx, db, &mut next_id).await {
                    Ok(Some(threads)) => Some((
                        own_conversation(&threads),
                        threads.iter().any(thread_active),
                    )),
                    Ok(None) => return Ok(()),
                    Err(e) => {
                        log::debug!(
                            "[codex_engine] {socket}: could not read its conversations: {e}"
                        );
                        None
                    }
                }
            } else {
                Some((None, false))
            };
            if let Some(id) = read
                .as_ref()
                .map(|(id, _)| id.clone())
                .filter(|id| named.as_ref() != Some(id))
            {
                name_conversation(&thread_file, id.as_deref());
                if queued {
                    match &id {
                        Some(id) => log::info!(
                            "[codex_engine] {socket}: refresh resumes conversation {id}"
                        ),
                        None => log::info!(
                            "[codex_engine] {socket}: refresh opens the conversation picker; the window has no single conversation"
                        ),
                    }
                }
                named = Some(id);
            }
            match (terminal.filter(|_| queued), read) {
                (Some(terminal), Some((_, busy))) => {
                    if busy && !is_waiting(socket) {
                        log::info!(
                            "[codex_engine] {socket}: refresh waits for the running turn to finish"
                        );
                    }
                    set_waiting(socket, busy);
                    if !busy {
                        ending = end_step(install, socket, marker, terminal, ending).await;
                    }
                }
                // Its turns could not be read; tried again at the next check.
                (Some(_), None) => {}
                (None, _) => {
                    set_waiting(socket, false);
                    ending = Ending::NotYet;
                }
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
                // A conversation was opened or got its first message.
                stale |= text.contains("\"thread/started\"")
                    || text.contains("\"thread/status/changed\"");
                if let Some(reply) = answer(db, &text).await {
                    tx.send(send(reply)).await.map_err(|e| e.to_string())?;
                }
            }
            _ = tokio::time::timeout(TICK, wake.changed()) => stale = true,
        }
    }
}

/// Names the conversation a window's shell resumes after a refresh on exit,
/// or removes the name.
fn name_conversation(path: &Path, id: Option<&str>) {
    let result = match id {
        Some(id) => std::fs::write(path, format!("{id}\n")),
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = result {
        log::warn!("[codex_engine] could not update {}: {e}", path.display());
    }
}

/// The conversation a window is in, when that is certain: the one
/// conversation with a message among the threads loaded on its server. The
/// server keeps every conversation its window opened loaded until it stops,
/// beside the empty thread the window starts with, a short-lived one that
/// titles a new conversation, and subagents' threads; those are left out.
/// `None` when no conversation or more than one remains.
fn own_conversation(threads: &[Value]) -> Option<String> {
    let mut ids = threads
        .iter()
        .filter(|thread| {
            thread.get("parentThreadId").is_none_or(Value::is_null)
                && thread.get("ephemeral").and_then(Value::as_bool) != Some(true)
                && thread
                    .get("preview")
                    .and_then(Value::as_str)
                    .is_some_and(|preview| !preview.is_empty())
        })
        .filter_map(|thread| thread.get("id").and_then(Value::as_str));
    let id = ids.next()?;
    (ids.next().is_none() && is_thread_id(id)).then(|| id.to_string())
}

/// A Codex thread id, the only form the window's shell resumes.
fn is_thread_id(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// Every thread loaded on a window's server, as `thread/read` describes it.
/// `Ok(None)` when the connection closed.
async fn loaded_threads(
    tx: &mut WsTx,
    rx: &mut WsRx,
    db: &Arc<Database>,
    next_id: &mut i64,
) -> Result<Option<Vec<Value>>, String> {
    let Some(list) = call(tx, rx, db, next_id, "thread/loaded/list", json!({})).await? else {
        return Ok(None);
    };
    let ids: Vec<String> = list
        .pointer("/result/data")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("thread/loaded/list answered {list}"))?
        .iter()
        .filter_map(|thread| thread.as_str().map(str::to_string))
        .collect();
    let mut threads = Vec::new();
    for id in ids {
        let params = json!({ "threadId": id, "includeTurns": false });
        let Some(mut read) = call(tx, rx, db, next_id, "thread/read", params).await? else {
            return Ok(None);
        };
        if let Some(thread) = read
            .pointer_mut("/result/thread")
            .map(Value::take)
            .filter(Value::is_object)
        {
            threads.push(thread);
        }
    }
    Ok(Some(threads))
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
    Ok(loaded_threads(tx, rx, db, next_id)
        .await?
        .map(|threads| threads.iter().any(thread_active)))
}

/// Whether a thread, as `thread/read` describes it, is mid-turn.
fn thread_active(thread: &Value) -> bool {
    thread.pointer("/status/type").and_then(Value::as_str) == Some("active")
}

/// How far a requested refresh has got in ending a window's Codex.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ending {
    NotYet,
    /// `/exit` was typed into the window's Orca pane.
    Typed(Instant),
    /// The window's Codex client was ended from outside.
    Ended(Instant),
    /// Its shell had still not taken the window back after that.
    GaveUp,
}

/// How long Codex may take to exit after Switchy asks it to.
const EXIT_WAIT: Duration = Duration::from_secs(15);

/// Takes the next step in ending an idle window's Codex for a refresh. In an
/// Orca pane Codex is quit as a user would, by typing `/exit`; in any other
/// terminal, or when that does not work, its client process is ended and the
/// shell restores the terminal.
async fn end_step(
    install: &Install,
    socket: &str,
    marker: &Path,
    terminal: &Terminal,
    ending: Ending,
) -> Ending {
    match ending {
        Ending::NotYet => {
            if let Terminal::Orca { pane, handle } = terminal {
                match type_exit(pane, handle).await {
                    Ok(handle) => {
                        log::info!(
                            "[codex_engine] {socket}: refreshing; typed /exit into Orca terminal {handle}"
                        );
                        return Ending::Typed(Instant::now());
                    }
                    Err(e) => log::warn!(
                        "[codex_engine] {socket}: could not type /exit into its Orca terminal ({e}); ending its Codex client instead"
                    ),
                }
            }
            end_client(install, socket, marker).await
        }
        Ending::Typed(at) if at.elapsed() >= EXIT_WAIT => {
            log::warn!(
                "[codex_engine] {socket}: Codex was still open {} s after /exit; ending its client",
                EXIT_WAIT.as_secs()
            );
            end_client(install, socket, marker).await
        }
        Ending::Ended(at) if at.elapsed() >= EXIT_WAIT => {
            log::warn!(
                "[codex_engine] {socket}: its shell had not restarted Codex {} s after its client was ended",
                EXIT_WAIT.as_secs()
            );
            Ending::GaveUp
        }
        other => other,
    }
}

/// Ends a window's Codex client from outside, after telling its shell to
/// restore the terminal the client leaves in raw mode.
async fn end_client(install: &Install, socket: &str, marker: &Path) -> Ending {
    // Opened without create, so a refresh cancelled meanwhile stays cancelled.
    let told = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(marker.with_extension("refresh"))
        .and_then(|mut file| std::io::Write::write_all(&mut file, ENDED.as_bytes()));
    if let Err(e) = told {
        log::warn!("[codex_engine] {socket}: refresh not started: {e}");
        return Ending::GaveUp;
    }
    match kill_client(install, socket).await {
        Ok(()) => {
            log::info!("[codex_engine] {socket}: refreshing; ended its Codex client");
            Ending::Ended(Instant::now())
        }
        Err(e) => {
            log::warn!("[codex_engine] {socket}: could not end its Codex client: {e}");
            Ending::GaveUp
        }
    }
}

/// Stops the Codex client connected with `--remote <socket>`: on Windows its
/// process tree, in WSL with SIGTERM, then SIGKILL if it is still running.
async fn kill_client(install: &Install, socket: &str) -> Result<(), String> {
    let mut command = match &install.kind {
        InstallKind::Windows => {
            // The address goes in the environment so this command's own
            // command line does not match.
            let script = concat!(
                "$remote = '--remote ' + $env:SWITCHY_CODEX_REMOTE + ' '\n",
                "$found = @(Get-CimInstance Win32_Process -Filter \"Name='cmd.exe' or Name='node.exe' or Name='codex.exe'\" |\n",
                "    Where-Object { $_.CommandLine -and $_.CommandLine.Contains($remote) })\n",
                "if ($found.Count -eq 0) { exit 3 }\n",
                "foreach ($process in $found) { & taskkill.exe /PID $process.ProcessId /T /F *> $null }\n",
                "exit 0\n",
            );
            let encoded: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut command = Command::new("powershell.exe");
            command
                .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
                .arg(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    encoded,
                ))
                .env("SWITCHY_CODEX_REMOTE", socket);
            #[cfg(windows)]
            command.creation_flags(0x0800_0000);
            command.kill_on_drop(true);
            command
        }
        InstallKind::Wsl(_) => {
            let pattern = client_pattern(socket)?;
            install.shell(&format!(
                "p={}\npkill -TERM -f -- \"$p\" || exit 3\nfor i in 1 2 3 4 5 6 7 8 9 10; do pgrep -f -- \"$p\" >/dev/null || exit 0; sleep 0.3; done\npkill -KILL -f -- \"$p\"\nexit 0\n",
                shell_quote(&pattern)
            ))
        }
    };
    let output = tokio::time::timeout(CALL_TIMEOUT, command.output())
        .await
        .map_err(|_| "it did not answer in time".to_string())?
        .map_err(|e| e.to_string())?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(3) => Err("no Codex client is connected to it".to_string()),
        _ => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
    }
}

/// The `pkill -f` pattern for the WSL Codex client on `socket`. The socket's
/// first character is bracketed so the shell running `pkill` does not match
/// its own command line.
fn client_pattern(socket: &str) -> Result<String, String> {
    let rest = socket
        .strip_prefix('/')
        .ok_or_else(|| format!("{socket} is not an absolute path"))?;
    let mut pattern = String::from("--remote unix://[/]");
    for c in rest.chars() {
        if ".[]()*+?{}|^$\\".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push_str("( |$)");
    Ok(pattern)
}

/// The Orca command line: on the PATH, or where Orca installs it.
fn orca_cli() -> Option<PathBuf> {
    let name = if cfg!(windows) { "orca.exe" } else { "orca" };
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|path| path.is_file())
        })
        .or_else(|| {
            dirs::data_local_dir()
                .map(|dir| dir.join("Programs/orca/resources/bin").join(name))
                .filter(|path| path.is_file())
        })
}

/// Runs the Orca command line and returns its JSON answer.
async fn orca(cli: &Path, args: &[&str]) -> Result<Value, String> {
    let mut command = Command::new(cli);
    command.args(args).arg("--json");
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    command.kill_on_drop(true);
    let output = tokio::time::timeout(CALL_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("orca {} did not answer in time", args.join(" ")))?
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let answer: Value = text
        .find('{')
        .and_then(|start| serde_json::from_str(&text[start..]).ok())
        .ok_or_else(|| format!("orca {} answered {}", args.join(" "), text.trim()))?;
    if answer.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!("orca {} answered {answer}", args.join(" ")));
    }
    Ok(answer)
}

/// The handle Orca now gives the pane a window's Codex started in: found by
/// its pane key, or by the handle it had then.
fn orca_handle(terminals: &[Value], pane: &str, handle: &str) -> Option<String> {
    let field = |terminal: &Value, key: &str| {
        terminal
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    terminals
        .iter()
        .find(|terminal| {
            field(terminal, "paneKey") == pane
                || format!("{}:{}", field(terminal, "tabId"), field(terminal, "leafId")) == pane
        })
        .or_else(|| {
            terminals
                .iter()
                .find(|terminal| !handle.is_empty() && field(terminal, "handle") == handle)
        })
        .map(|terminal| field(terminal, "handle"))
        .filter(|handle| !handle.is_empty())
}

/// Quits Codex in an Orca pane as a user would: Ctrl+C clears anything typed
/// but not sent, then `/exit` and Enter. Each goes as plain input: with
/// `--enter`, Orca treats the text as an agent prompt and watches for its
/// submission, and sent right after a reply that call outlasted Codex and the
/// restarted Codex quit too. Enter goes on its own so Codex does not take it
/// as part of a paste.
/// Returns the pane's handle.
async fn type_exit(pane: &str, handle: &str) -> Result<String, String> {
    let cli = orca_cli().ok_or("the Orca command line is not installed")?;
    let list = orca(&cli, &["terminal", "list"]).await?;
    let terminals = list
        .pointer("/result/terminals")
        .and_then(Value::as_array)
        .ok_or("Orca listed no terminals")?;
    let handle = orca_handle(terminals, pane, handle).ok_or("its Orca pane is closed")?;
    orca(&cli, &["terminal", "send", "--terminal", &handle, "--interrupt"]).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    orca(&cli, &["terminal", "send", "--terminal", &handle, "--text", "/exit"]).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    orca(&cli, &["terminal", "send", "--terminal", &handle, "--text", "\r"]).await?;
    Ok(handle)
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
        assert_eq!(bashrc.matches(POSIX_PROFILE_TAG).count(), 1);
        assert!(bashrc.starts_with("export A=1\n"));
        assert_eq!(
            std::fs::read_to_string(codex_dir.join(SHELL_FILE)).expect("shell"),
            posix_shell(&posix_config_path(&codex_dir))
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
    fn posix_profiles_preserve_user_content_and_quote_custom_paths() {
        let home = tempfile::TempDir::new().expect("temp home");
        let codex_dir = home.path().join("custom codex's config");
        std::fs::write(
            home.path().join(".bashrc"),
            format!("export KEPT=1\n{LEGACY_BASHRC_LINE}\n"),
        )
        .unwrap();
        std::fs::write(home.path().join(".zshrc"), "export ALSO_KEPT=1\n").unwrap();
        let config_dir = posix_config_path(&codex_dir);
        install_posix_shell(&codex_dir, home.path(), &config_dir, true).unwrap();
        install_posix_shell(&codex_dir, home.path(), &config_dir, true).unwrap();
        for (name, prefix) in [
            (".bashrc", "export KEPT=1\n"),
            (".zshrc", "export ALSO_KEPT=1\n"),
        ] {
            let profile = std::fs::read_to_string(home.path().join(name)).unwrap();
            assert!(profile.starts_with(prefix));
            assert_eq!(profile.matches(POSIX_PROFILE_TAG).count(), 1);
            assert!(profile.contains(&shell_quote(&format!("{config_dir}/{SHELL_FILE}"))));
            assert!(!profile.contains(LEGACY_BASHRC_LINE));
        }
        assert!(std::fs::read_to_string(codex_dir.join(SHELL_FILE))
            .unwrap()
            .contains(&format!("_sw_config_dir={}", shell_quote(&config_dir))));
        assert_eq!(
            posix_config_path(Path::new(r"\\wsl.localhost\Ubuntu\home\me\custom config")),
            "/home/me/custom config"
        );
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

    #[tokio::test]
    async fn a_window_whose_server_refuses_is_forgotten_and_one_failing_otherwise_is_kept() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let db = Arc::new(Database::memory().unwrap());
        let home = tempfile::TempDir::new().expect("temp home");
        let install = Install {
            kind: InstallKind::Windows,
            codex_dir: home.path().join(".codex"),
        };
        std::fs::create_dir_all(install.sessions_dir()).expect("sessions dir");
        let window = |name: &str, socket: &str| {
            let marker = install.sessions_dir().join(format!("{name}.session"));
            std::fs::write(&marker, format!("{socket}\nrefresh-on-exit-v1\n")).unwrap();
            std::fs::write(marker.with_extension("token"), "token").unwrap();
            std::fs::write(marker.with_extension("refresh"), "").unwrap();
            marker
        };
        let files = |marker: &Path| {
            ["session", "token", "refresh"]
                .map(|extension| marker.with_extension(extension).exists())
        };

        // Nothing listens on a port that was just released.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let socket = format!("ws://127.0.0.1:{port}");
        let gone = window("gone", &socket);
        assert_eq!(serve(&db, &install, &socket, &gone, None).await, Ok(()));
        assert_eq!(files(&gone), [false; 3]);

        // A server that turns the window's token away.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream.read(&mut [0u8; 4096]).await;
            let _ = stream
                .write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n")
                .await;
        });
        let unauthorized = window("unauthorized", &socket);
        assert!(serve(&db, &install, &socket, &unauthorized, None).await.is_err());
        assert_eq!(files(&unauthorized), [true; 3]);
        assert_eq!(install.sessions().len(), 1);
    }

    #[test]
    fn a_wsl_relay_reports_a_gone_socket_only_for_a_missing_or_refusing_socket() {
        let failed = |cause: &str| {
            format!("Error: failed to connect to socket at /home/me/.codex/switchy-sessions/1-2.sock\n\nCaused by:\n    {cause}\n")
        };
        assert!(wsl_socket_gone(&failed(
            "No such file or directory (os error 2)"
        )));
        assert!(wsl_socket_gone(&failed(
            "Connection refused (os error 111)"
        )));
        assert!(!wsl_socket_gone(&failed("Permission denied (os error 13)")));
        assert!(!wsl_socket_gone(&failed(
            "Resource temporarily unavailable (os error 11)"
        )));
        assert!(!wsl_socket_gone(""));
        assert!(!wsl_socket_gone("No such file or directory (os error 2)"));
    }

    /// The threads a window's server holds after it was started and one
    /// conversation was opened: the empty one it started with, the
    /// conversation, the short-lived thread that titled it, and a subagent's.
    fn window_threads(conversation: &str) -> Vec<Value> {
        vec![
            json!({ "id": "01a154ce-1092-7ff0-9287-bfe0d4a8a215", "parentThreadId": null, "ephemeral": false, "preview": "" }),
            json!({ "id": conversation, "parentThreadId": null, "ephemeral": false, "preview": "Reply with BRAVO." }),
            json!({ "id": "01a154cd-4748-7d42-8593-2b9900c8c65b", "parentThreadId": null, "ephemeral": true, "preview": "" }),
            json!({ "id": "01a154cf-0000-7000-8000-000000000001", "parentThreadId": conversation, "ephemeral": false, "preview": "Check the tests." }),
        ]
    }

    #[test]
    fn a_window_names_its_conversation_only_when_it_has_exactly_one() {
        let conversation = "01a154cd-3b7a-75a1-a33e-087f6c86447c";
        let mut threads = window_threads(conversation);
        assert_eq!(own_conversation(&threads).as_deref(), Some(conversation));

        // Only the empty thread it started with: no conversation yet.
        assert_eq!(own_conversation(&threads[..1]), None);

        // A second conversation opened in the same window.
        threads.push(
            json!({ "id": "01a154d0-5555-7000-8000-000000000002", "preview": "Another one." }),
        );
        assert_eq!(own_conversation(&threads), None);

        // Anything but a thread id is never handed to the shell.
        let odd = vec![json!({ "id": "BRAVO; rm -rf ~", "preview": "x" })];
        assert_eq!(own_conversation(&odd), None);
        assert!(is_thread_id(conversation));
        assert!(!is_thread_id("01a154cd3b7a-75a1-a33e-087f6c86447c-"));
        assert!(!is_thread_id("01a154cd-3b7a-75a1-a33e-087f6c86447"));
    }

    /// A window server that answers `thread/loaded/list` and `thread/read`
    /// from `threads`, and every other request with an empty result.
    async fn conversation_server(threads: Vec<Value>) -> String {
        live_server(Arc::new(Mutex::new(threads))).await
    }

    /// `conversation_server` over threads the test can change.
    async fn live_server(threads: Arc<Mutex<Vec<Value>>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(message)) = ws.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                let Some(id) = request.get("id").cloned() else {
                    continue;
                };
                let threads = threads.lock().unwrap().clone();
                let result = match request["method"].as_str() {
                    Some("thread/loaded/list") => json!({
                        "data": threads.iter().map(|thread| thread["id"].clone()).collect::<Vec<_>>(),
                    }),
                    Some("thread/read") => json!({
                        "thread": threads.iter().find(|thread| thread["id"] == request["params"]["threadId"]),
                    }),
                    _ => json!({}),
                };
                let reply = json!({ "id": id, "result": result }).to_string();
                if ws.send(Message::Text(reply.into())).await.is_err() {
                    break;
                }
            }
        });
        socket
    }

    #[tokio::test]
    async fn a_queued_window_names_its_conversation_for_the_shell_until_it_closes() {
        let db = Arc::new(Database::memory().unwrap());
        let home = tempfile::TempDir::new().expect("temp home");
        let install = Install {
            kind: InstallKind::Windows,
            codex_dir: home.path().join(".codex"),
        };
        std::fs::create_dir_all(install.sessions_dir()).expect("sessions dir");
        let conversation = "01a154cd-3b7a-75a1-a33e-087f6c86447c";
        let socket = conversation_server(window_threads(conversation)).await;
        let marker = install.sessions_dir().join("window.session");
        std::fs::write(&marker, format!("{socket}\nrefresh-on-exit-v1\n")).unwrap();
        std::fs::write(marker.with_extension("token"), "token").unwrap();
        std::fs::write(marker.with_extension("refresh"), "").unwrap();
        let named = marker.with_extension("thread");

        let serving = {
            let (db, install, socket, marker) =
                (db.clone(), install.clone(), socket.clone(), marker.clone());
            tokio::spawn(async move { serve(&db, &install, &socket, &marker, None).await })
        };
        let read = || std::fs::read_to_string(&named).unwrap_or_default();
        assert!(
            eventually(|| read() == format!("{conversation}\n")).await,
            "named {:?}",
            read()
        );

        // A cancelled refresh takes the name back.
        std::fs::remove_file(marker.with_extension("refresh")).unwrap();
        nudge();
        assert!(eventually(|| !named.exists()).await);

        // A closed window leaves nothing behind.
        std::fs::write(marker.with_extension("refresh"), "").unwrap();
        nudge();
        assert!(eventually(|| named.exists()).await);
        std::fs::remove_file(&marker).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(10), serving)
            .await
            .expect("serve returns once the window closes");
        assert_eq!(result.unwrap(), Ok(()));
        assert!(!named.exists());
    }

    async fn eventually(mut done: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while !done() && started.elapsed() < Duration::from_secs(10) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        done()
    }

    #[test]
    fn a_session_records_the_terminal_its_codex_runs_in() {
        let home = tempfile::TempDir::new().expect("temp home");
        let install = Install {
            kind: InstallKind::Windows,
            codex_dir: home.path().join(".codex"),
        };
        std::fs::create_dir_all(install.sessions_dir()).expect("sessions dir");
        let lines = "refresh-on-exit-v1\nC:\\Projects\ncodex-cli 0.163.0\nC:\\Users\\me\\.codex\n";
        for (name, rest) in [
            ("orca", "C:\\npm\\codex.cmd\nterminal=orca tab-1:leaf-2 term_abc\n"),
            ("orca_without_handle", "C:\\npm\\codex.cmd\nterminal=orca tab-1:leaf-2 \n"),
            // The WSL shell leaves the executable line empty.
            ("native", "\nterminal=native\n"),
            ("before_one_click", "C:\\npm\\codex.cmd\n"),
        ] {
            std::fs::write(
                install.sessions_dir().join(format!("{name}.session")),
                format!("ws://127.0.0.1:1\n{lines}{rest}"),
            )
            .unwrap();
        }
        let terminal = |name: &str| {
            install
                .sessions()
                .into_iter()
                .find(|session| session.marker.file_stem().unwrap() == name)
                .unwrap()
                .terminal
        };
        let orca = |handle: &str| Terminal::Orca {
            pane: "tab-1:leaf-2".into(),
            handle: handle.into(),
        };
        assert_eq!(terminal("orca"), Some(orca("term_abc")));
        assert_eq!(terminal("orca_without_handle"), Some(orca("")));
        assert_eq!(terminal("native"), Some(Terminal::Native));
        assert_eq!(terminal("before_one_click"), None);
        assert_eq!(Terminal::parse("terminal=tmux"), None);
    }

    #[test]
    fn an_orca_pane_is_found_by_its_key_or_else_its_handle() {
        let terminals = vec![
            json!({ "handle": "term_a", "tabId": "tab-1", "leafId": "leaf-1" }),
            json!({ "handle": "term_b", "tabId": "tab-1", "leafId": "leaf-2" }),
            json!({ "handle": "term_c", "paneKey": "tab-3:leaf-3" }),
        ];
        let find = |pane, handle| orca_handle(&terminals, pane, handle);
        assert_eq!(find("tab-1:leaf-2", "term_old").as_deref(), Some("term_b"));
        assert_eq!(find("tab-3:leaf-3", "").as_deref(), Some("term_c"));
        assert_eq!(find("tab-9:leaf-9", "term_a").as_deref(), Some("term_a"));
        assert_eq!(find("tab-9:leaf-9", ""), None);
        assert_eq!(find("tab-9:leaf-9", "term_gone"), None);
    }

    #[test]
    fn the_wsl_client_pattern_matches_the_client_and_not_the_command_running_it() {
        let socket = "/home/me/.codex/switchy-sessions/12-34.sock";
        let pattern = client_pattern(socket).unwrap();
        let regex = regex::Regex::new(&pattern).unwrap();
        assert!(regex.is_match(&format!("node /usr/bin/codex --remote unix://{socket} -m gpt")));
        assert!(regex.is_match(&format!("/vendor/codex --remote unix://{socket}")));
        // Another window's socket, the server and the relay of this one.
        assert!(!regex.is_match(&format!("codex --remote unix://{socket}x")));
        assert!(!regex.is_match(&format!("codex --remote unix:///home/me/.codex/switchy-sessions/12-345.sock")));
        assert!(!regex.is_match(&format!("codex app-server --listen unix://{socket}")));
        assert!(!regex.is_match(&format!("codex app-server proxy --sock {socket}")));
        assert!(!regex.is_match(&format!("bash -lc p={}", shell_quote(&pattern))));
        assert!(client_pattern("relative.sock").is_err());
    }

    /// A process whose command line names `socket` the way a window's Codex
    /// client does, and that runs until it is stopped.
    #[cfg(windows)]
    fn fake_client(socket: &str) -> std::process::Child {
        std::process::Command::new("cmd.exe")
            .args(["/d", "/k", "rem", "--remote", socket, "--remote-auth-token-env", "X"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("fake client")
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_requested_refresh_waits_for_the_turn_then_ends_the_client_and_can_be_cancelled() {
        let db = Arc::new(Database::memory().unwrap());
        let home = tempfile::TempDir::new().expect("temp home");
        let install = Install {
            kind: InstallKind::Windows,
            codex_dir: home.path().join(".codex"),
        };
        std::fs::create_dir_all(install.sessions_dir()).expect("sessions dir");
        let conversation = "01a154cd-3b7a-75a1-a33e-087f6c86447c";
        let threads = Arc::new(Mutex::new(window_threads(conversation)));
        let set_status = |status: &str| {
            for thread in threads.lock().unwrap().iter_mut() {
                if thread["id"] == conversation {
                    thread["status"] = json!({ "type": status });
                }
            }
        };
        set_status("active");
        let socket = live_server(threads.clone()).await;
        let marker = install.sessions_dir().join("window.session");
        std::fs::write(
            &marker,
            format!("{socket}\nrefresh-on-exit-v1\nC:\\\ncodex-cli 0.163.0\n\n\nterminal=native\n"),
        )
        .unwrap();
        std::fs::write(marker.with_extension("token"), "token").unwrap();
        let refresh = marker.with_extension("refresh");
        std::fs::write(&refresh, "").unwrap();
        let mut client = fake_client(&socket);
        let serving = {
            let (db, install, socket, marker) =
                (db.clone(), install.clone(), socket.clone(), marker.clone());
            tokio::spawn(async move {
                serve(&db, &install, &socket, &marker, Some(&Terminal::Native)).await
            })
        };
        let ended = || std::fs::read_to_string(&refresh).unwrap_or_default() == ENDED;

        // Mid-turn: the refresh waits and the client keeps running.
        assert!(eventually(|| is_waiting(&socket)).await);
        tokio::time::sleep(TICK * 2).await;
        assert!(!ended());
        assert!(client.try_wait().unwrap().is_none());

        // Cancelled, the window is left alone after its turn ends.
        std::fs::remove_file(&refresh).unwrap();
        nudge();
        assert!(eventually(|| !is_waiting(&socket)).await);
        set_status("idle");
        tokio::time::sleep(TICK * 2).await;
        assert!(!refresh.exists());
        assert!(client.try_wait().unwrap().is_none());

        // Requested while idle, the client is ended at once and its shell is
        // told to restore the terminal.
        std::fs::write(&refresh, "").unwrap();
        nudge();
        assert!(eventually(ended).await);
        assert!(eventually(|| client.try_wait().unwrap().is_some()).await);
        assert_eq!(
            std::fs::read_to_string(marker.with_extension("thread")).unwrap(),
            format!("{conversation}\n")
        );
        assert!(!is_waiting(&socket));

        // Ending is not cancelled any more, and the window's shell takes it
        // from here.
        std::fs::remove_file(&marker).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(10), serving)
            .await
            .expect("serve returns once the window closes");
        assert_eq!(result.unwrap(), Ok(()));
    }

    #[tokio::test]
    async fn an_orca_pane_that_does_not_exit_after_exit_is_ended_from_outside() {
        let home = tempfile::TempDir::new().expect("temp home");
        let install = Install {
            kind: InstallKind::Windows,
            codex_dir: home.path().join(".codex"),
        };
        std::fs::create_dir_all(install.sessions_dir()).expect("sessions dir");
        let marker = install.sessions_dir().join("window.session");
        let refresh = marker.with_extension("refresh");
        std::fs::write(&refresh, "").unwrap();
        let orca = Terminal::Orca {
            pane: "tab:leaf".into(),
            handle: "term_x".into(),
        };
        let socket = "ws://127.0.0.1:9";
        let just_typed = Ending::Typed(Instant::now());
        assert_eq!(
            end_step(&install, socket, &marker, &orca, just_typed).await,
            just_typed
        );
        assert_eq!(std::fs::read_to_string(&refresh).unwrap(), "");

        // Nothing answers on that address, so ending it fails and is not
        // retried; the shell would still restore the terminal.
        let long_ago = Instant::now() - EXIT_WAIT - Duration::from_secs(1);
        assert_eq!(
            end_step(&install, socket, &marker, &orca, Ending::Typed(long_ago)).await,
            Ending::GaveUp
        );
        assert_eq!(std::fs::read_to_string(&refresh).unwrap(), ENDED);

        // A refresh cancelled meanwhile is not started.
        std::fs::remove_file(&refresh).unwrap();
        assert_eq!(
            end_client(&install, socket, &marker).await,
            Ending::GaveUp
        );
        assert!(!refresh.exists());
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
                json!({ "method": "thread/tokenUsage/updated", "params": {} })
                    .to_string()
                    .into(),
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
        assert!(window_busy(&mut tx, &mut rx, &db, &mut next_id)
            .await
            .is_err());

        let (mut tx, mut rx) = scripted_server(Vec::new());
        assert_eq!(
            window_busy(&mut tx, &mut rx, &db, &mut next_id).await,
            Ok(None)
        );
    }
}
