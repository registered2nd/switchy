//! Panic hook
//!
//! Captures panic information when the app crashes and appends it to `<app_config_dir>/crash.log` (default `~/.switchy/crash.log`),
//! so users and developers can diagnose crashes.

use std::fs::OpenOptions;
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::sync::OnceLock;

/// App version (from Cargo.toml)
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

static APP_CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init_app_config_dir(dir: PathBuf) {
    let _ = APP_CONFIG_DIR.set(dir);
}

/// Get the default app config directory (never panics)
fn default_app_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(crate::paths::APP_DIR)
}

/// Get the app config directory (prefers the value set at init; never panics)
fn get_app_config_dir() -> PathBuf {
    APP_CONFIG_DIR
        .get()
        .cloned()
        .unwrap_or_else(default_app_config_dir)
}

/// Get the crash log file path
fn get_crash_log_path() -> PathBuf {
    get_app_config_dir().join("crash.log")
}

/// Get the log directory path
pub fn get_log_dir() -> PathBuf {
    get_app_config_dir().join("logs")
}

/// Collect environment information safely (never panics)
fn get_system_info() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let family = std::env::consts::FAMILY;

    // Current working directory, safely
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Current thread information, safely
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let thread_id = format!("{:?}", thread.id());

    format!(
        "OS: {os} ({family})\n\
         Arch: {arch}\n\
         App Version: {APP_VERSION}\n\
         Working Dir: {cwd}\n\
         Thread: {thread_name} (ID: {thread_id})"
    )
}

/// Install the panic hook, which captures crash information and writes it to the log file
///
/// Call this at app startup so every panic is recorded.
/// Each log entry contains:
/// - timestamp
/// - app version and system information
/// - panic message
/// - location (file:line)
/// - backtrace (full call stack)
pub fn setup_panic_hook() {
    // Enable backtraces (so release builds capture them too)
    if std::env::var("RUST_BACKTRACE").is_err() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        let log_path = get_crash_log_path();

        // Make sure the directory exists
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Build the crash entry (catch_unwind guards the time formatting against a nested panic)
        let timestamp = std::panic::catch_unwind(|| {
            chrono::Local::now()
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|_| {
            // Fall back to a unix timestamp if chrono panics
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| format!("unix:{}.{:03}", d.as_secs(), d.subsec_millis()))
                .unwrap_or_else(|_| "unknown".to_string())
        });

        // System information
        let system_info = std::panic::catch_unwind(get_system_info)
            .unwrap_or_else(|_| "Failed to get system info".to_string());

        // Panic message (try several ways to extract it)
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            // Try the Display trait
            format!("{panic_info}")
        };

        // Location
        let location = if let Some(loc) = panic_info.location() {
            format!(
                "File: {}\n         Line: {}\n         Column: {}",
                loc.file(),
                loc.line(),
                loc.column()
            )
        } else {
            "Unknown location".to_string()
        };

        // Capture the backtrace (full call stack)
        let backtrace = std::backtrace::Backtrace::force_capture();
        let backtrace_str = format!("{backtrace}");

        // Format the log entry
        let separator = "=".repeat(80);
        let sub_separator = "-".repeat(40);
        let crash_entry = format!(
            r#"
{separator}
[CRASH REPORT] {timestamp}
{separator}

{sub_separator}
System Information
{sub_separator}
{system_info}

{sub_separator}
Error Details
{sub_separator}
Message: {message}

Location: {location}

{sub_separator}
Stack Trace (Backtrace)
{sub_separator}
{backtrace_str}

{separator}
"#
        );

        // Append to the file
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = file.write_all(crash_entry.as_bytes());
            let _ = file.flush();

            // Print the log file location to stderr
            eprintln!("\n[Switchy] Crash log saved to: {}", log_path.display());
        }

        // Also print to stderr (useful during development)
        eprintln!("{crash_entry}");

        // Call the default hook
        default_hook(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_log_path() {
        let path = get_crash_log_path();
        assert!(path.ends_with("crash.log"));
        assert!(path.to_string_lossy().contains(crate::paths::APP_DIR));
    }

    #[test]
    fn test_system_info() {
        let info = get_system_info();
        assert!(info.contains("OS:"));
        assert!(info.contains("Arch:"));
        assert!(info.contains("App Version:"));
    }
}
