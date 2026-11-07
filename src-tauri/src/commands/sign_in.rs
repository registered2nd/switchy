use tauri::AppHandle;

use crate::services::sign_in::{self, SignInApp, SignInStart, SignInStatus, SignedIn};

/// Starts signing a Codex or Claude card's account in without switching to
/// it: answers the link to open in that account's own browser, and whether
/// the sign-in finishes by itself (`callback`) or with `sign_in_finish`
/// (`code`).
#[tauri::command]
pub async fn sign_in_start(
    handle: AppHandle,
    app: String,
    #[allow(non_snake_case)] providerId: String,
) -> Result<SignInStart, String> {
    sign_in::start(&handle, SignInApp::parse(&app)?, &providerId)
}

/// Finishes a sign-in with the code the browser showed (Claude's
/// `code#state`) or the address it was sent back to. The exchange leaves
/// through the card's own proxy.
#[tauri::command]
pub async fn sign_in_finish(
    handle: AppHandle,
    #[allow(non_snake_case)] sessionId: String,
    code: String,
) -> Result<SignedIn, String> {
    sign_in::finish(&handle, &sessionId, &code).await
}

/// Where a sign-in stands: pending, done, failed or expired.
#[tauri::command]
pub async fn sign_in_status(
    #[allow(non_snake_case)] sessionId: String,
) -> Result<SignInStatus, String> {
    sign_in::status(&sessionId)
}
