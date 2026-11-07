use tauri::State;

use std::str::FromStr;

use crate::app_config::AppType;
use crate::services::codex_account::{self, CodexIdentity};
use crate::services::login_account::{self, LoginIdentity};
use crate::store::AppState;

#[tauri::command]
pub fn get_codex_account_identity(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] providerId: String,
) -> Result<Option<CodexIdentity>, String> {
    codex_account::read_identity(state.inner(), &providerId).map_err(|e| e.to_string())
}

/// The account a Cursor CLI or Antigravity CLI provider holds.
#[tauri::command]
pub fn get_login_account_identity(
    state: State<'_, AppState>,
    app: String,
    #[allow(non_snake_case)] providerId: String,
) -> Result<Option<LoginIdentity>, String> {
    let app = AppType::from_str(&app).map_err(|e| e.to_string())?;
    login_account::read_identity(state.inner(), &app, &providerId).map_err(|e| e.to_string())
}
