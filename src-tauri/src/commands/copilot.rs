//! GitHub Copilot Tauri Commands
//!
//! Tauri commands for Copilot OAuth authentication, with multi-account support.

use crate::proxy::providers::copilot_auth::{
    CopilotAuthManager, CopilotAuthStatus, CopilotModel, CopilotUsageResponse, GitHubAccount,
    GitHubDeviceCodeResponse,
};
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

/// Copilot authentication state
pub struct CopilotAuthState(pub Arc<RwLock<CopilotAuthManager>>);

// ==================== Device code flow ====================

/// Start the device code flow
///
/// Returns the device code and user code for OAuth authentication
#[tauri::command]
pub async fn copilot_start_device_flow(
    state: State<'_, CopilotAuthState>,
) -> Result<GitHubDeviceCodeResponse, String> {
    let auth_manager = state.0.read().await;
    auth_manager
        .start_device_flow()
        .await
        .map_err(|e| e.to_string())
}

/// Poll for the OAuth token (backward compatible)
///
/// Polls GitHub with the device code until the user finishes authorizing.
/// Returns true once authorized, false while still waiting.
#[tauri::command(rename_all = "camelCase")]
pub async fn copilot_poll_for_auth(
    device_code: String,
    state: State<'_, CopilotAuthState>,
) -> Result<bool, String> {
    let auth_manager = state.0.write().await;
    match auth_manager.poll_for_token(&device_code).await {
        Ok(Some(_account)) => {
            log::info!("[CopilotAuth] User authorized");
            Ok(true)
        }
        Ok(None) => Ok(false),
        Err(crate::proxy::providers::copilot_auth::CopilotAuthError::AuthorizationPending) => {
            Ok(false)
        }
        Err(e) => {
            log::error!("[CopilotAuth] Polling failed: {e}");
            Err(e.to_string())
        }
    }
}

/// Poll for the OAuth token (multi-account version)
///
/// Returns the newly added account once authorization succeeds
#[tauri::command(rename_all = "camelCase")]
pub async fn copilot_poll_for_account(
    device_code: String,
    state: State<'_, CopilotAuthState>,
) -> Result<Option<GitHubAccount>, String> {
    let auth_manager = state.0.write().await;
    match auth_manager.poll_for_token(&device_code).await {
        Ok(account) => Ok(account),
        Err(crate::proxy::providers::copilot_auth::CopilotAuthError::AuthorizationPending) => {
            Ok(None)
        }
        Err(e) => {
            log::error!("[CopilotAuth] Polling failed: {e}");
            Err(e.to_string())
        }
    }
}

// ==================== Multi-account management ====================

/// List all authenticated accounts
#[tauri::command]
pub async fn copilot_list_accounts(
    state: State<'_, CopilotAuthState>,
) -> Result<Vec<GitHubAccount>, String> {
    let auth_manager = state.0.read().await;
    Ok(auth_manager.list_accounts().await)
}

/// Remove the given account
#[tauri::command(rename_all = "camelCase")]
pub async fn copilot_remove_account(
    account_id: String,
    state: State<'_, CopilotAuthState>,
) -> Result<(), String> {
    let auth_manager = state.0.write().await;
    auth_manager
        .remove_account(&account_id)
        .await
        .map_err(|e| e.to_string())
}

/// Set the default account
#[tauri::command(rename_all = "camelCase")]
pub async fn copilot_set_default_account(
    account_id: String,
    state: State<'_, CopilotAuthState>,
) -> Result<(), String> {
    let auth_manager = state.0.write().await;
    auth_manager
        .set_default_account(&account_id)
        .await
        .map_err(|e| e.to_string())
}

// ==================== Status ====================

/// Get the authentication status (all accounts)
#[tauri::command]
pub async fn copilot_get_auth_status(
    state: State<'_, CopilotAuthState>,
) -> Result<CopilotAuthStatus, String> {
    let auth_manager = state.0.read().await;
    Ok(auth_manager.get_status().await)
}

/// Check whether any account is authenticated
#[tauri::command]
pub async fn copilot_is_authenticated(state: State<'_, CopilotAuthState>) -> Result<bool, String> {
    let auth_manager = state.0.read().await;
    Ok(auth_manager.is_authenticated().await)
}

/// Sign out of all Copilot accounts
#[tauri::command]
pub async fn copilot_logout(state: State<'_, CopilotAuthState>) -> Result<(), String> {
    let auth_manager = state.0.write().await;
    auth_manager.clear_auth().await.map_err(|e| e.to_string())
}

// ==================== Token retrieval ====================

/// Get a valid Copilot token (backward compatible: uses the first account)
///
/// Internal; used for proxied requests
#[tauri::command]
pub async fn copilot_get_token(state: State<'_, CopilotAuthState>) -> Result<String, String> {
    let auth_manager = state.0.read().await;
    auth_manager
        .get_valid_token()
        .await
        .map_err(|e| e.to_string())
}

/// Get a valid Copilot token for the given account
#[tauri::command(rename_all = "camelCase")]
pub async fn copilot_get_token_for_account(
    account_id: String,
    state: State<'_, CopilotAuthState>,
) -> Result<String, String> {
    let auth_manager = state.0.read().await;
    auth_manager
        .get_valid_token_for_account(&account_id)
        .await
        .map_err(|e| e.to_string())
}

// ==================== Models and usage ====================

/// Get the available Copilot models (backward compatible: uses the first account)
#[tauri::command]
pub async fn copilot_get_models(
    state: State<'_, CopilotAuthState>,
) -> Result<Vec<CopilotModel>, String> {
    let auth_manager = state.0.read().await;
    auth_manager.fetch_models().await.map_err(|e| e.to_string())
}

/// Get the available Copilot models for the given account
#[tauri::command(rename_all = "camelCase")]
pub async fn copilot_get_models_for_account(
    account_id: String,
    state: State<'_, CopilotAuthState>,
) -> Result<Vec<CopilotModel>, String> {
    let auth_manager = state.0.read().await;
    auth_manager
        .fetch_models_for_account(&account_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get Copilot usage (backward compatible: uses the first account)
#[tauri::command]
pub async fn copilot_get_usage(
    state: State<'_, CopilotAuthState>,
) -> Result<CopilotUsageResponse, String> {
    let auth_manager = state.0.read().await;
    auth_manager.fetch_usage().await.map_err(|e| e.to_string())
}

/// Get Copilot usage for the given account
#[tauri::command(rename_all = "camelCase")]
pub async fn copilot_get_usage_for_account(
    account_id: String,
    state: State<'_, CopilotAuthState>,
) -> Result<CopilotUsageResponse, String> {
    let auth_manager = state.0.read().await;
    auth_manager
        .fetch_usage_for_account(&account_id)
        .await
        .map_err(|e| e.to_string())
}
