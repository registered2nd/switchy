//! Model list commands
//!
//! Tauri commands the frontend uses to fetch available models in the provider form.

use crate::services::model_fetch::{self, FetchedModel};

/// Get the available models for a provider
///
/// Uses the OpenAI-compatible GET /v1/models endpoint.
/// Mainly for third-party aggregators (SiliconFlow, OpenRouter, etc.).
#[tauri::command(rename_all = "camelCase")]
pub async fn fetch_models_for_config(
    base_url: String,
    api_key: String,
    is_full_url: Option<bool>,
) -> Result<Vec<FetchedModel>, String> {
    model_fetch::fetch_models(&base_url, &api_key, is_full_url.unwrap_or(false)).await
}
