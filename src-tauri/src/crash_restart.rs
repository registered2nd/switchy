//! Restart after a crash, and an account of how the previous run ended
//!
//! A tool routed through the proxy has no endpoint while Switchy is down, so on
//! Windows Switchy asks Windows Error Reporting to start it again after a crash
//! or a hang. A refused memory allocation ends a Rust program without a panic,
//! so the panic hook never sees it; instead, each run leaves a marker file that
//! a clean exit removes. When the next start finds the marker, it looks up what
//! Windows recorded about that run and appends it to `crash.log`.

use std::path::PathBuf;

const MARKER_FILE: &str = "running.json";

fn marker_path() -> PathBuf {
    crate::config::get_app_config_dir().join(MARKER_FILE)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RunMarker {
    pid: u32,
    version: String,
    /// UTC, RFC 3339
    started: String,
}

/// Asks Windows Error Reporting to start Switchy again, in the tray, if this
/// run crashes or hangs. Windows restarts only a process that has run for at
/// least 60 seconds, so a crash during startup cannot loop. A normal quit, a
/// `taskkill` or the installer stopping Switchy does not trigger it.
#[cfg(target_os = "windows")]
pub fn register_restart() {
    #[link(name = "kernel32")]
    extern "system" {
        fn RegisterApplicationRestart(command_line: *const u16, flags: u32) -> i32;
    }
    const RESTART_NO_PATCH: u32 = 4;
    const RESTART_NO_REBOOT: u32 = 8;

    let args: Vec<u16> = crate::auto_launch::AUTOSTART_ARG
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let hr =
        unsafe { RegisterApplicationRestart(args.as_ptr(), RESTART_NO_PATCH | RESTART_NO_REBOOT) };
    if hr == 0 {
        log::info!("[crash_restart] Windows will restart Switchy after a crash or hang");
    } else {
        log::warn!("[crash_restart] could not register for restart after a crash: {hr:#x}");
    }
}

/// Records this run, after reporting a previous run that did not exit cleanly.
pub fn begin_run() {
    let path = marker_path();
    let previous = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<RunMarker>(&text).ok());

    let marker = RunMarker {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        started: chrono::Utc::now().to_rfc3339(),
    };
    if let Ok(text) = serde_json::to_string(&marker) {
        if let Err(e) = std::fs::write(&path, text) {
            log::warn!("[crash_restart] could not write {}: {e}", path.display());
        }
    }

    if let Some(previous) = previous {
        // The event log queries take a moment; keep them off the startup path
        let _ = std::thread::Builder::new()
            .name("previous-run".to_string())
            .spawn(move || report_previous_run(&previous));
    }
}

/// Marks this run as ended cleanly.
pub fn end_run() {
    let _ = std::fs::remove_file(marker_path());
}

fn report_previous_run(previous: &RunMarker) {
    let started = local_time(&previous.started);
    let crash = find_windows_crash(previous);

    let Some(crash) = crash else {
        log::info!(
            "[crash_restart] previous run (pid {}, {}, started {started}) ended without shutting down; Windows recorded no crash for it",
            previous.pid,
            previous.version
        );
        return;
    };

    let mut lines = vec![
        format!(
            "Previous run: pid {}, version {}, started {started}",
            previous.pid, previous.version
        ),
        crash,
    ];
    lines.extend(find_low_memory(previous));
    lines.extend(find_dump(previous));
    let details = lines.join("\n");

    log::warn!("[crash_restart] previous run crashed:\n{details}");

    let separator = "=".repeat(80);
    let entry = format!(
        "\n{separator}\n[ABNORMAL EXIT] recorded at the next start, {}\n{separator}\n\n{details}\n\n{separator}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
    );
    let log_path = crate::config::get_app_config_dir().join("crash.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        use std::io::Write;
        let _ = file.write_all(entry.as_bytes());
    }
}

fn local_time(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| rfc3339.to_string())
}

#[cfg(target_os = "windows")]
fn query_event_log(log: &str, query: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let output = std::process::Command::new("wevtutil")
        .args([
            "qe",
            log,
            &format!("/q:{query}"),
            "/rd:true",
            "/c:1",
            "/f:xml",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    text.contains("<Event").then_some(text)
}

/// The text between `start` and the next `end` in `text`.
fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let len = text[from..].find(end)?;
    Some(&text[from..from + len])
}

#[cfg(target_os = "windows")]
fn find_windows_crash(previous: &RunMarker) -> Option<String> {
    let query = format!(
        "*[System[Provider[@Name='Application Error'] and EventID=1000] and EventData[Data[@Name='AppName']='switchy.exe' and Data[@Name='ProcessId']='{:#x}']]",
        previous.pid
    );
    let event = query_event_log("Application", &query)?;
    Some(describe_crash(&event))
}

#[cfg(not(target_os = "windows"))]
fn find_windows_crash(_previous: &RunMarker) -> Option<String> {
    None
}

fn describe_crash(event: &str) -> String {
    let time = between(event, "SystemTime='", "'")
        .map(local_time)
        .unwrap_or_else(|| "an unknown time".to_string());
    let code = between(event, "<Data Name='ExceptionCode'>", "<").unwrap_or("?");
    let offset = between(event, "<Data Name='FaultingOffset'>", "<").unwrap_or("?");
    let module = between(event, "<Data Name='ModuleName'>", "<").unwrap_or("?");
    let mut text = format!(
        "Windows recorded a crash at {time}: exception 0x{code} in {module} at offset 0x{offset}"
    );
    if code.eq_ignore_ascii_case("c0000409") && module.eq_ignore_ascii_case("switchy.exe") {
        text.push_str(
            "\nThis is an abort with no panic report. When Windows ran out of memory at the same time, it was a refused memory allocation.",
        );
    }
    text
}

/// Windows' low-memory warnings since the previous run started.
#[cfg(target_os = "windows")]
fn find_low_memory(previous: &RunMarker) -> Option<String> {
    let since = chrono::DateTime::parse_from_rfc3339(&previous.started)
        .ok()?
        .with_timezone(&chrono::Utc)
        .format("%Y-%m-%dT%H:%M:%S.000Z");
    let query = format!("*[System[EventID=1003 and TimeCreated[@SystemTime>='{since}']]]");
    let event = query_event_log(
        "Microsoft-Windows-Resource-Exhaustion-Detector/Operational",
        &query,
    )?;
    Some(describe_low_memory(&event))
}

#[cfg(not(target_os = "windows"))]
fn find_low_memory(_previous: &RunMarker) -> Option<String> {
    None
}

fn describe_low_memory(event: &str) -> String {
    let time = between(event, "SystemTime='", "'")
        .map(local_time)
        .unwrap_or_else(|| "an unknown time".to_string());
    let gb = |tag: &str| {
        between(event, &format!("<{tag}>"), "<")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| format!("{:.1} GB", v / 1_073_741_824.0))
            .unwrap_or_else(|| "?".to_string())
    };
    format!(
        "Windows ran out of memory at {time}: {} committed of a {} limit",
        gb("SystemCommitCharge"),
        gb("SystemCommitLimit")
    )
}

/// The crash dump Windows keeps when local dumps are enabled.
#[cfg(target_os = "windows")]
fn find_dump(previous: &RunMarker) -> Option<String> {
    let dump = dirs::data_local_dir()?
        .join("CrashDumps")
        .join(format!("switchy.exe.{}.dmp", previous.pid));
    dump.exists()
        .then(|| format!("Crash dump: {}", dump.display()))
}

#[cfg(not(target_os = "windows"))]
fn find_dump(_previous: &RunMarker) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const CRASH: &str = "<Event><System><Provider Name='Application Error'/><EventID>1000</EventID><TimeCreated SystemTime='2026-10-20T12:10:37.6054377Z'/></System><EventData><Data Name='AppName'>switchy.exe</Data><Data Name='ModuleName'>switchy.exe</Data><Data Name='ExceptionCode'>c0000409</Data><Data Name='FaultingOffset'>0000000000321c11</Data><Data Name='ProcessId'>0x103e4</Data></EventData></Event>";

    const LOW_MEMORY: &str = "<Event><System><EventID>1003</EventID><TimeCreated SystemTime='2026-10-20T12:10:37.3130549Z'/></System><UserData><CommitLimitExhaustion><SystemCommitLimit>274209533952</SystemCommitLimit><SystemCommitCharge>273944322048</SystemCommitCharge></CommitLimitExhaustion></UserData></Event>";

    #[test]
    fn describes_an_abort_in_switchy() {
        let text = describe_crash(CRASH);
        assert!(text.contains("exception 0xc0000409 in switchy.exe at offset 0x0000000000321c11"));
        assert!(text.contains("refused memory allocation"));
    }

    #[test]
    fn describes_commit_exhaustion() {
        let text = describe_low_memory(LOW_MEMORY);
        assert!(
            text.contains("255.1 GB committed of a 255.4 GB limit"),
            "{text}"
        );
    }

    #[test]
    fn between_finds_the_value() {
        assert_eq!(between("a<b>c</b>", "<b>", "<"), Some("c"));
        assert_eq!(between("a", "<b>", "<"), None);
    }
}
