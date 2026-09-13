use tauri::State;

use crate::services::codex_account::{self, CodexIdentity};
use crate::store::AppState;

#[tauri::command]
pub fn get_codex_account_identity(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] providerId: String,
) -> Result<Option<CodexIdentity>, String> {
    codex_account::read_identity(state.inner(), &providerId).map_err(|e| e.to_string())
}
