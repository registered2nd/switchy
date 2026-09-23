use crate::services::subscription::SubscriptionQuota;

/// Query the official subscription quota
///
/// Reads the OAuth credentials the CLI tools already have and calls the official API for usage.
/// Needs no AppState (no database access); reads files and sends HTTP directly.
#[tauri::command]
pub async fn get_subscription_quota(tool: String) -> Result<SubscriptionQuota, String> {
    crate::services::subscription::get_subscription_quota(&tool).await
}

/// Per-provider subscription quota.
///
/// Claude: reads `~/.switchy/accounts/{provider_id}/credentials.json` (the
/// captured snapshot). Codex: reads the login stored in the provider's own
/// `auth`. Returns NotFound when the provider has nothing to read from —
/// caller should fall back to `get_subscription_quota` for live credentials.
#[tauri::command]
pub async fn get_subscription_quota_for_provider(
    state: tauri::State<'_, crate::store::AppState>,
    tool: Option<String>,
    provider_id: String,
) -> Result<SubscriptionQuota, String> {
    match tool.as_deref().unwrap_or("claude") {
        "codex" => {
            crate::services::subscription::get_codex_quota_for_provider(&state.db, &provider_id)
                .await
        }
        "claude" => {
            crate::services::subscription::get_claude_quota_for_provider(
                state.inner(),
                &provider_id,
            )
            .await
        }
        other => Ok(SubscriptionQuota::not_found(other)),
    }
}
