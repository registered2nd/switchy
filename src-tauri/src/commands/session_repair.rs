#![allow(non_snake_case)]

use crate::session_repair;

/// Claude Code transcripts an account switch left unreadable.
#[tauri::command]
pub async fn scan_broken_sessions() -> Result<Vec<session_repair::BrokenSessionInfo>, String> {
    tauri::async_runtime::spawn_blocking(session_repair::scan_broken_sessions)
        .await
        .map_err(|e| format!("Failed to scan sessions: {e}"))
}

#[tauri::command]
pub async fn repair_broken_session(
    sourcePath: String,
) -> Result<session_repair::RepairResult, String> {
    let path = std::path::PathBuf::from(sourcePath);
    tauri::async_runtime::spawn_blocking(move || session_repair::repair_session(&path))
        .await
        .map_err(|e| format!("Failed to repair session: {e}"))?
}
