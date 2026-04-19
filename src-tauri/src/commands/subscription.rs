use crate::services::subscription::SubscriptionQuota;

/// 查询官方订阅额度
///
/// 读取 CLI 工具已有的 OAuth 凭据并调用官方 API 获取使用额度。
/// 不需要 AppState（不访问数据库），直接读文件 + 发 HTTP。
#[tauri::command]
pub async fn get_subscription_quota(tool: String) -> Result<SubscriptionQuota, String> {
    crate::services::subscription::get_subscription_quota(&tool).await
}

/// Per-provider Claude subscription quota from a captured snapshot.
///
/// Reads `~/.switchy/accounts/{provider_id}/credentials.json` and queries
/// Anthropic's OAuth usage API. Returns NotFound if the provider has no
/// captured snapshot — caller should fall back to `get_subscription_quota`
/// for live credentials in that case.
#[tauri::command]
pub async fn get_subscription_quota_for_provider(
    provider_id: String,
) -> Result<SubscriptionQuota, String> {
    crate::services::subscription::get_claude_quota_for_provider(&provider_id).await
}
