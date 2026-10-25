use std::str::FromStr;

use tauri::State;

use crate::app_config::AppType;
use crate::database::Account;
use crate::services::accounts::{self, account_balance, account_quota, BalanceReport};
use crate::services::subscription::SubscriptionQuota;
use crate::store::AppState;

/// Every plan account and balance monitor.
#[tauri::command]
pub fn get_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, String> {
    state.db.get_accounts().map_err(|e| e.to_string())
}

/// Adds or edits a plan account or a balance monitor. A plan account's
/// programs get its new name, key and host at once.
#[tauri::command]
pub fn save_account(state: State<'_, AppState>, account: Account) -> Result<(), String> {
    accounts::save_account(state.inner(), &account).map_err(|e| e.to_string())
}

/// Deletes the account and the providers it made in programs; refused while
/// Claude Code is using it.
#[tauri::command]
pub fn delete_account(state: State<'_, AppState>, id: String) -> Result<(), String> {
    accounts::delete_account(state.inner(), &id).map_err(|e| e.to_string())
}

/// "Used in": puts a plan account into a program (`claude` or `opencode`) and
/// switches the program to it. Answers the provider id it runs as there.
#[tauri::command]
pub fn use_account_in(
    state: State<'_, AppState>,
    id: String,
    app: String,
) -> Result<String, String> {
    let app_type = AppType::from_str(&app).map_err(|e| e.to_string())?;
    accounts::use_account_in(state.inner(), &id, app_type).map_err(|e| e.to_string())
}

/// Adds an empty sign-in card to a program whose accounts are its own logins;
/// answers its provider id. Switch to it and sign in in the program.
#[tauri::command]
pub fn add_sign_in_account(
    state: State<'_, AppState>,
    app: String,
    name: Option<String>,
) -> Result<String, String> {
    let app_type = AppType::from_str(&app).map_err(|e| e.to_string())?;
    accounts::add_sign_in_account(state.inner(), app_type, name).map_err(|e| e.to_string())
}

/// Takes a plan account out of OpenCode, which holds several at once.
#[tauri::command]
pub fn remove_account_from(
    state: State<'_, AppState>,
    id: String,
    app: String,
) -> Result<(), String> {
    let app_type = AppType::from_str(&app).map_err(|e| e.to_string())?;
    accounts::remove_account_from(state.inner(), &id, app_type).map_err(|e| e.to_string())
}

/// A plan account's quota windows, read with its key.
#[tauri::command]
pub async fn get_account_quota(
    state: State<'_, AppState>,
    id: String,
) -> Result<SubscriptionQuota, String> {
    account_quota(&state.db, &id)
        .await
        .map_err(|e| e.to_string())
}

/// A balance monitor's balance and what was spent today and this month.
#[tauri::command]
pub async fn get_account_balance(
    state: State<'_, AppState>,
    id: String,
) -> Result<BalanceReport, String> {
    account_balance(&state.db, &id)
        .await
        .map_err(|e| e.to_string())
}
