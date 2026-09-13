use tauri::State;

use crate::services::claude_account::{self, CaptureOutcome, CapturedIdentity};
use crate::store::AppState;

#[tauri::command]
pub fn capture_claude_account(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] providerId: String,
    force: Option<bool>,
) -> Result<CaptureOutcome, String> {
    claude_account::capture(state.inner(), &providerId, force.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_claude_account(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] providerId: String,
) -> Result<(), String> {
    claude_account::clear(state.inner(), &providerId).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_captured_claude_identity(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] providerId: String,
) -> Result<Option<CapturedIdentity>, String> {
    claude_account::read_captured_identity(state.inner(), &providerId).map_err(|e| e.to_string())
}
