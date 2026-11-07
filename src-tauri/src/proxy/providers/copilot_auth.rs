//! GitHub Copilot Authentication Module
//!
//! Implements the GitHub OAuth device-code flow and Copilot token management.
//! Supports multiple accounts; each provider can be bound to a different GitHub account.
//!
//! ## Auth flow
//! 1. Start the device-code flow and get device_code and user_code
//! 2. The user authorizes on GitHub in the browser
//! 3. Poll for the access_token
//! 4. Use the GitHub token to get a Copilot token
//! 5. Refresh the Copilot token automatically (60 seconds before expiry)
//!
//! ## Multi-account support (v3)
//! - Each GitHub account stores its own token
//! - Providers bind to an account through meta.authBinding
//! - The v1 single-account format migrates automatically to the v3 multi-account + default-account format

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// GitHub OAuth client ID (the one VS Code uses)
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// GitHub device-code URL
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";

/// GitHub OAuth Token URL
const GITHUB_OAUTH_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// Copilot Token URL
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// GitHub User API URL
const GITHUB_USER_URL: &str = "https://api.github.com/user";

/// Token refresh lead time (seconds)
const TOKEN_REFRESH_BUFFER_SECONDS: i64 = 60;

/// Copilot API endpoint
const COPILOT_MODELS_URL: &str = "https://api.githubcopilot.com/models";

/// Copilot API header constants
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.110.1";
pub const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.38.2";
pub const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.38.2";
pub const COPILOT_API_VERSION: &str = "2025-10-01";
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// Copilot usage API URL
const COPILOT_USAGE_URL: &str = "https://api.github.com/copilot_internal/user";

/// Default Copilot API endpoint
const DEFAULT_COPILOT_API_ENDPOINT: &str = "https://api.githubcopilot.com";

/// Copilot usage response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotUsageResponse {
    /// Copilot plan type
    pub copilot_plan: String,
    /// Quota reset date
    pub quota_reset_date: String,
    /// Quota snapshots
    pub quota_snapshots: QuotaSnapshots,
    /// API endpoint info (used to resolve the API URL dynamically)
    #[serde(default)]
    pub endpoints: Option<CopilotEndpoints>,
}

/// Copilot API endpoint info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotEndpoints {
    /// API endpoint URL
    pub api: String,
    /// Telemetry endpoint URL
    #[serde(default)]
    pub telemetry: Option<String>,
}

/// Quota snapshots
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSnapshots {
    /// Chat quota
    pub chat: QuotaDetail,
    /// Completions quota
    pub completions: QuotaDetail,
    /// Premium interactions quota
    pub premium_interactions: QuotaDetail,
}

/// Quota details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaDetail {
    /// Total quota
    pub entitlement: i64,
    /// Remaining quota
    pub remaining: i64,
    /// Remaining percentage
    pub percent_remaining: f64,
    /// Whether unlimited
    pub unlimited: bool,
}

/// Copilot model available to the account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotModel {
    /// Model ID (used in API calls)
    pub id: String,
    /// Model display name
    pub name: String,
    /// Model vendor
    pub vendor: String,
    /// Whether it is shown in the model picker
    pub model_picker_enabled: bool,
}

/// Copilot Models API response
#[derive(Debug, Deserialize)]
struct CopilotModelsResponse {
    data: Vec<CopilotModelsResponseItem>,
}

/// Copilot Models API response item
#[derive(Debug, Deserialize)]
struct CopilotModelsResponseItem {
    id: String,
    name: String,
    vendor: String,
    model_picker_enabled: bool,
}

/// Copilot auth error
#[derive(Debug, thiserror::Error)]
pub enum CopilotAuthError {
    #[error("Device-code flow not started")]
    DeviceFlowNotStarted,

    #[error("Waiting for user authorization")]
    AuthorizationPending,

    #[error("User denied authorization")]
    AccessDenied,

    #[error("Device code expired")]
    ExpiredToken,

    #[error("GitHub token is invalid or expired")]
    GitHubTokenInvalid,

    #[error("Failed to get Copilot token: {0}")]
    CopilotTokenFetchFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("User has no Copilot subscription")]
    NoCopilotSubscription,

    #[error("Account not found: {0}")]
    AccountNotFound(String),
}

impl From<reqwest::Error> for CopilotAuthError {
    fn from(err: reqwest::Error) -> Self {
        CopilotAuthError::NetworkError(err.to_string())
    }
}

impl From<std::io::Error> for CopilotAuthError {
    fn from(err: std::io::Error) -> Self {
        CopilotAuthError::IoError(err.to_string())
    }
}

/// GitHub device-code response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubDeviceCodeResponse {
    /// Device code (used for polling)
    pub device_code: String,
    /// User code (shown to the user)
    pub user_code: String,
    /// Verification URL
    pub verification_uri: String,
    /// Expiry time (seconds)
    pub expires_in: u64,
    /// Polling interval (seconds)
    pub interval: u64,
}

/// GitHub OAuth token response
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubOAuthResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Copilot Token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotToken {
    /// JWT Token
    pub token: String,
    /// Expiry timestamp (Unix seconds)
    pub expires_at: i64,
}

impl CopilotToken {
    /// Whether the token is about to expire (within 60 seconds)
    pub fn is_expiring_soon(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.expires_at - now < TOKEN_REFRESH_BUFFER_SECONDS
    }
}

/// Copilot token API response
#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: i64,
    #[allow(dead_code)]
    refresh_in: Option<i64>,
}

/// GitHub user info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: u64,
    pub avatar_url: Option<String>,
}

/// GitHub account (public info returned to the frontend)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAccount {
    /// GitHub user ID (as a string, used as the unique key)
    pub id: String,
    /// GitHub username
    pub login: String,
    /// Avatar URL
    pub avatar_url: Option<String>,
    /// Authentication timestamp
    pub authenticated_at: i64,
}

impl From<&GitHubAccountData> for GitHubAccount {
    fn from(data: &GitHubAccountData) -> Self {
        GitHubAccount {
            id: data.user.id.to_string(),
            login: data.user.login.clone(),
            avatar_url: data.user.avatar_url.clone(),
            authenticated_at: data.authenticated_at,
        }
    }
}

/// Copilot auth status (multi-account)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotAuthStatus {
    /// All authenticated accounts
    pub accounts: Vec<GitHubAccount>,
    /// Default account ID (explicit, so it does not depend on HashMap order)
    pub default_account_id: Option<String>,
    /// Status message when migrating legacy auth data failed (shown in the frontend)
    pub migration_error: Option<String>,
    /// Whether authenticated (backward compatible: true if any account exists)
    pub authenticated: bool,
    /// GitHub username (backward compatible: the first account's username)
    pub username: Option<String>,
    /// Copilot token expiry (backward compatible: the first account's expiry)
    pub expires_at: Option<i64>,
}

/// Account data (internal storage)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubAccountData {
    /// GitHub OAuth Token
    ///
    /// Security note: the token is persisted locally so the login can be reused.
    /// It is not kept in the system keychain; it relies on private file permissions (0600 on Unix).
    pub github_token: String,
    /// User info
    pub user: GitHubUser,
    /// Authentication timestamp
    pub authenticated_at: i64,
}

/// Persisted store (v3 multi-account + default-account format)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CopilotAuthStore {
    /// Store format version (3 = multi-account + default-account format)
    #[serde(default)]
    version: u32,
    /// Account data (key = GitHub user ID)
    #[serde(default)]
    accounts: HashMap<String, GitHubAccountData>,
    /// Default account ID
    #[serde(skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
    /// Fields kept for the v1 single-account format
    #[serde(skip_serializing_if = "Option::is_none")]
    github_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authenticated_at: Option<i64>,
}

/// Copilot auth manager (multi-account)
pub struct CopilotAuthManager {
    /// All GitHub accounts (key = GitHub user ID)
    accounts: Arc<RwLock<HashMap<String, GitHubAccountData>>>,
    /// Default account ID
    default_account_id: Arc<RwLock<Option<String>>>,
    /// Per-account refresh lock, so concurrent refreshes do not hit the GitHub API twice
    refresh_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// Copilot token cache (key = GitHub user ID, in memory, refreshed automatically)
    copilot_tokens: Arc<RwLock<HashMap<String, CopilotToken>>>,
    /// Copilot models cache (key = GitHub user ID, reused only within the process)
    copilot_models: Arc<RwLock<HashMap<String, Vec<CopilotModel>>>>,
    /// Copilot API endpoint cache (key = GitHub user ID, fetched from /copilot_internal/user)
    api_endpoints: Arc<RwLock<HashMap<String, String>>>,
    /// Per-account endpoint fetch lock, so concurrent fetches do not hit the GitHub API twice
    endpoint_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// HTTP client
    http_client: Client,
    /// Store path
    storage_path: PathBuf,
    /// Legacy-format token awaiting migration
    pending_migration: Arc<RwLock<Option<String>>>,
    /// Status message when migrating legacy auth data failed
    migration_error: Arc<RwLock<Option<String>>>,
}

impl CopilotAuthManager {
    /// Creates a new auth manager
    pub fn new(data_dir: PathBuf) -> Self {
        let storage_path = data_dir.join("copilot_auth.json");

        let manager = Self {
            accounts: Arc::new(RwLock::new(HashMap::new())),
            default_account_id: Arc::new(RwLock::new(None)),
            refresh_locks: Arc::new(RwLock::new(HashMap::new())),
            copilot_tokens: Arc::new(RwLock::new(HashMap::new())),
            copilot_models: Arc::new(RwLock::new(HashMap::new())),
            api_endpoints: Arc::new(RwLock::new(HashMap::new())),
            endpoint_locks: Arc::new(RwLock::new(HashMap::new())),
            http_client: Client::new(),
            storage_path,
            pending_migration: Arc::new(RwLock::new(None)),
            migration_error: Arc::new(RwLock::new(None)),
        };

        // Try loading from disk (synchronous, no network requests)
        if let Err(e) = manager.load_from_disk_sync() {
            log::warn!("[CopilotAuth] Failed to load store: {e}");
        }

        manager
    }

    // ==================== Multi-account management ====================

    /// Lists all authenticated accounts
    pub async fn list_accounts(&self) -> Vec<GitHubAccount> {
        let accounts = self.accounts.read().await.clone();
        let default_account_id = self.resolve_default_account_id().await;
        Self::sorted_accounts(&accounts, default_account_id.as_deref())
    }

    /// Returns the given account
    pub async fn get_account(&self, account_id: &str) -> Option<GitHubAccount> {
        let accounts = self.accounts.read().await;
        accounts.get(account_id).map(GitHubAccount::from)
    }

    /// Removes the given account
    pub async fn remove_account(&self, account_id: &str) -> Result<(), CopilotAuthError> {
        log::info!("[CopilotAuth] Removing account: {account_id}");

        {
            let mut accounts = self.accounts.write().await;
            if accounts.remove(account_id).is_none() {
                return Err(CopilotAuthError::AccountNotFound(account_id.to_string()));
            }
        }

        // Also drop the cached Copilot token
        {
            let mut tokens = self.copilot_tokens.write().await;
            tokens.remove(account_id);
        }
        {
            let mut models = self.copilot_models.write().await;
            models.remove(account_id);
        }
        {
            let mut refresh_locks = self.refresh_locks.write().await;
            refresh_locks.remove(account_id);
        }
        // Clear the API endpoint cache
        {
            let mut api_endpoints = self.api_endpoints.write().await;
            api_endpoints.remove(account_id);
        }
        {
            let mut endpoint_locks = self.endpoint_locks.write().await;
            endpoint_locks.remove(account_id);
        }

        {
            let accounts = self.accounts.read().await;
            let mut default_account_id = self.default_account_id.write().await;
            if default_account_id.as_deref() == Some(account_id) {
                *default_account_id = Self::fallback_default_account_id(&accounts);
            }
        }

        // Persist
        self.save_to_disk().await?;

        Ok(())
    }

    /// Adds a new account (internal, called after OAuth completes)
    async fn add_account_internal(
        &self,
        github_token: String,
        user: GitHubUser,
    ) -> Result<GitHubAccount, CopilotAuthError> {
        let account_id = user.id.to_string();
        let now = chrono::Utc::now().timestamp();

        let account_data = GitHubAccountData {
            github_token,
            user: user.clone(),
            authenticated_at: now,
        };

        let account = GitHubAccount {
            id: account_id.clone(),
            login: user.login.clone(),
            avatar_url: user.avatar_url.clone(),
            authenticated_at: now,
        };

        {
            let mut accounts = self.accounts.write().await;
            accounts.insert(account_id, account_data);
        }

        {
            let mut default_account_id = self.default_account_id.write().await;
            if default_account_id.is_none() {
                *default_account_id = Some(account.id.clone());
            }
        }

        self.set_migration_error(None).await;

        // Persist
        self.save_to_disk().await?;

        log::info!("[CopilotAuth] Account added: {}", user.login);

        Ok(account)
    }

    /// Sets the default account
    pub async fn set_default_account(&self, account_id: &str) -> Result<(), CopilotAuthError> {
        {
            let accounts = self.accounts.read().await;
            if !accounts.contains_key(account_id) {
                return Err(CopilotAuthError::AccountNotFound(account_id.to_string()));
            }
        }

        {
            let mut default_account_id = self.default_account_id.write().await;
            *default_account_id = Some(account_id.to_string());
        }

        self.save_to_disk().await?;
        Ok(())
    }

    // ==================== Device-code flow ====================

    /// Starts the device-code flow
    pub async fn start_device_flow(&self) -> Result<GitHubDeviceCodeResponse, CopilotAuthError> {
        log::info!("[CopilotAuth] Starting device-code flow");

        let response = self
            .http_client
            .post(GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .header("User-Agent", COPILOT_USER_AGENT)
            .form(&[("client_id", GITHUB_CLIENT_ID), ("scope", "read:user")])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::NetworkError(format!(
                "GitHub device-code request failed: {status} - {text}"
            )));
        }

        let device_code: GitHubDeviceCodeResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!(
            "[CopilotAuth] Got device code, user_code: {}",
            device_code.user_code
        );

        Ok(device_code)
    }

    /// Polls for the OAuth token (returns the newly added account on success)
    pub async fn poll_for_token(
        &self,
        device_code: &str,
    ) -> Result<Option<GitHubAccount>, CopilotAuthError> {
        log::debug!("[CopilotAuth] Polling for OAuth token");

        let response = self
            .http_client
            .post(GITHUB_OAUTH_TOKEN_URL)
            .header("Accept", "application/json")
            .header("User-Agent", COPILOT_USER_AGENT)
            .form(&[
                ("client_id", GITHUB_CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;

        let oauth_response: GitHubOAuthResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        // Check for errors
        if let Some(error) = oauth_response.error {
            return match error.as_str() {
                "authorization_pending" => Err(CopilotAuthError::AuthorizationPending),
                "slow_down" => Err(CopilotAuthError::AuthorizationPending),
                "expired_token" => Err(CopilotAuthError::ExpiredToken),
                "access_denied" => Err(CopilotAuthError::AccessDenied),
                _ => Err(CopilotAuthError::NetworkError(format!(
                    "{}: {}",
                    error,
                    oauth_response.error_description.unwrap_or_default()
                ))),
            };
        }

        // Get the access_token
        let access_token = oauth_response
            .access_token
            .ok_or_else(|| CopilotAuthError::ParseError("Missing access_token".to_string()))?;

        log::info!("[CopilotAuth] OAuth token obtained");

        // Get user info
        let user = self.fetch_user_info_with_token(&access_token).await?;

        // Verify the Copilot subscription (by getting a Copilot token)
        self.fetch_copilot_token_with_github_token(&access_token, &user.id.to_string())
            .await?;

        // Add the account
        let account = self.add_account_internal(access_token, user).await?;

        Ok(Some(account))
    }

    // ==================== Token retrieval ====================

    /// Returns a valid Copilot token for the given account (refreshed automatically)
    pub async fn get_valid_token_for_account(
        &self,
        account_id: &str,
    ) -> Result<String, CopilotAuthError> {
        // Make sure migration is done
        self.ensure_migration_complete().await?;

        // Check the cached token
        {
            let tokens = self.copilot_tokens.read().await;
            if let Some(copilot_token) = tokens.get(account_id) {
                if !copilot_token.is_expiring_soon() {
                    return Ok(copilot_token.token.clone());
                }
            }
        }

        // Refresh needed
        log::info!("[CopilotAuth] Copilot token for account {account_id} needs a refresh");

        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _refresh_guard = refresh_lock.lock().await;

        // Double-check: another request may have refreshed it while we waited for the lock
        {
            let tokens = self.copilot_tokens.read().await;
            if let Some(copilot_token) = tokens.get(account_id) {
                if !copilot_token.is_expiring_soon() {
                    return Ok(copilot_token.token.clone());
                }
            }
        }

        // Get the account's GitHub token
        let github_token = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .map(|a| a.github_token.clone())
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?
        };

        // Refresh the Copilot token
        self.fetch_copilot_token_with_github_token(&github_token, account_id)
            .await?;

        // Return the new token
        let tokens = self.copilot_tokens.read().await;
        tokens.get(account_id).map(|t| t.token.clone()).ok_or(
            CopilotAuthError::CopilotTokenFetchFailed("Still no token after refresh".to_string()),
        )
    }

    /// Returns a valid Copilot token (backward compatible: uses the first account)
    pub async fn get_valid_token(&self) -> Result<String, CopilotAuthError> {
        // Make sure migration is done
        self.ensure_migration_complete().await?;

        match self.resolve_default_account_id().await {
            Some(id) => self.get_valid_token_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    // ==================== Models and usage ====================

    /// Returns the Copilot models available to the given account
    pub async fn fetch_models_for_account(
        &self,
        account_id: &str,
    ) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        self.ensure_migration_complete().await?;

        {
            let models = self.copilot_models.read().await;
            if let Some(cached) = models.get(account_id) {
                return Ok(cached.clone());
            }
        }

        let models = self.fetch_models_for_account_uncached(account_id).await?;
        {
            let mut cache = self.copilot_models.write().await;
            cache.insert(account_id.to_string(), models.clone());
        }
        Ok(models)
    }

    async fn fetch_models_for_account_uncached(
        &self,
        account_id: &str,
    ) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        let copilot_token = self.get_valid_token_for_account(account_id).await?;

        log::info!("[CopilotAuth] Fetching available Copilot models for account {account_id}");

        let response = self
            .http_client
            .get(COPILOT_MODELS_URL)
            .header("Authorization", format!("Bearer {copilot_token}"))
            .header("Content-Type", "application/json")
            .header("copilot-integration-id", "vscode-chat")
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "Failed to fetch model list: {status} - {text}"
            )));
        }

        let models_response: CopilotModelsResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        let models: Vec<CopilotModel> = models_response
            .data
            .into_iter()
            .filter(|m| m.model_picker_enabled)
            .map(|m| CopilotModel {
                id: m.id,
                name: m.name,
                vendor: m.vendor,
                model_picker_enabled: m.model_picker_enabled,
            })
            .collect();

        log::info!("[CopilotAuth] Got {} available models", models.len());

        Ok(models)
    }

    pub async fn get_model_vendor_for_account(
        &self,
        account_id: &str,
        model_id: &str,
    ) -> Result<Option<String>, CopilotAuthError> {
        let models = self.fetch_models_for_account(account_id).await?;
        Ok(models
            .into_iter()
            .find(|model| model.id == model_id)
            .map(|model| model.vendor))
    }

    /// Returns the available Copilot models (backward compatible: uses the first account)
    pub async fn fetch_models(&self) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.fetch_models_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    pub async fn get_model_vendor(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, CopilotAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.get_model_vendor_for_account(&id, model_id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    /// Returns Copilot usage for the given account
    pub async fn fetch_usage_for_account(
        &self,
        account_id: &str,
    ) -> Result<CopilotUsageResponse, CopilotAuthError> {
        let github_token = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .map(|a| a.github_token.clone())
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?
        };

        log::info!("[CopilotAuth] Fetching Copilot usage for account {account_id}");

        let response = self
            .http_client
            .get(COPILOT_USAGE_URL)
            .header("Authorization", format!("token {github_token}"))
            .header("Content-Type", "application/json")
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "Failed to fetch usage: {status} - {text}"
            )));
        }

        let usage: CopilotUsageResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        // Store the dynamic API endpoint, if any
        if let Some(ref endpoints) = usage.endpoints {
            let mut api_endpoints = self.api_endpoints.write().await;
            api_endpoints.insert(account_id.to_string(), endpoints.api.clone());
            // Log at debug level so enterprise-internal domains stay out of the logs
            log::debug!("[CopilotAuth] Saved dynamic API endpoint for account {account_id}");
        }

        log::info!(
            "[CopilotAuth] Got usage, plan: {}, reset date: {}",
            usage.copilot_plan,
            usage.quota_reset_date
        );

        Ok(usage)
    }

    /// Returns Copilot usage (backward compatible: uses the first account)
    pub async fn fetch_usage(&self) -> Result<CopilotUsageResponse, CopilotAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.fetch_usage_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    // ==================== Status queries ====================

    /// Returns the given account's API endpoint (from cache, or fetched lazily from the API on a miss)
    pub async fn get_api_endpoint(&self, account_id: &str) -> String {
        let _ = self.ensure_migration_complete().await;

        {
            let endpoints = self.api_endpoints.read().await;
            if let Some(endpoint) = endpoints.get(account_id) {
                return endpoint.clone();
            }
        }

        // Serialize concurrent fetches for the same account to avoid duplicate GitHub API requests
        let lock = self.get_endpoint_lock(account_id).await;
        let _guard = lock.lock().await;

        // Check again after taking the lock: another request may have filled it
        {
            let endpoints = self.api_endpoints.read().await;
            if let Some(endpoint) = endpoints.get(account_id) {
                return endpoint.clone();
            }
        }

        match self.fetch_and_cache_endpoint(account_id).await {
            Ok(endpoint) => endpoint,
            Err(e) => {
                log::debug!(
                    "[CopilotAuth] Failed to fetch dynamic API endpoint for account {account_id}: {e}; using the default"
                );
                DEFAULT_COPILOT_API_ENDPOINT.to_string()
            }
        }
    }

    /// Returns the default account's API endpoint
    pub async fn get_default_api_endpoint(&self) -> String {
        let _ = self.ensure_migration_complete().await;

        match self.resolve_default_account_id().await {
            Some(id) => self.get_api_endpoint(&id).await,
            None => DEFAULT_COPILOT_API_ENDPOINT.to_string(),
        }
    }

    async fn fetch_and_cache_endpoint(&self, account_id: &str) -> Result<String, CopilotAuthError> {
        let github_token = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .map(|a| a.github_token.clone())
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?
        };

        log::debug!("[CopilotAuth] Lazily fetching dynamic API endpoint for account {account_id}");

        let response = self
            .http_client
            .get(COPILOT_USAGE_URL)
            .header("Authorization", format!("token {github_token}"))
            .header("Content-Type", "application/json")
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if !response.status().is_success() {
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "Failed to fetch API endpoint: {}",
                response.status()
            )));
        }

        let usage: CopilotUsageResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        let endpoint = match usage.endpoints {
            Some(endpoints) => endpoints.api.clone(),
            None => DEFAULT_COPILOT_API_ENDPOINT.to_string(),
        };

        // Cache the endpoint (including the default) to avoid repeat requests
        let mut api_endpoints = self.api_endpoints.write().await;
        api_endpoints.insert(account_id.to_string(), endpoint.clone());
        log::debug!("[CopilotAuth] Cached API endpoint for account {account_id}");

        Ok(endpoint)
    }

    async fn get_endpoint_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let locks = self.endpoint_locks.read().await;
            if let Some(lock) = locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.endpoint_locks.write().await;
        Arc::clone(
            locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// Returns the auth status (multi-account)
    pub async fn get_status(&self) -> CopilotAuthStatus {
        // Make sure migration is done
        let _ = self.ensure_migration_complete().await;

        let accounts = self.accounts.read().await.clone();
        let default_account_id = self.resolve_default_account_id().await;
        let copilot_tokens = self.copilot_tokens.read().await.clone();
        let migration_error = self.migration_error.read().await.clone();

        let account_list = Self::sorted_accounts(&accounts, default_account_id.as_deref());
        let authenticated = !account_list.is_empty();
        let username = default_account_id
            .as_ref()
            .and_then(|id| accounts.get(id))
            .map(|a| a.user.login.clone())
            .or_else(|| account_list.first().map(|a| a.login.clone()));

        // Expiry of the default account
        let expires_at = default_account_id
            .as_ref()
            .and_then(|id| copilot_tokens.get(id))
            .map(|t| t.expires_at);

        CopilotAuthStatus {
            accounts: account_list,
            default_account_id,
            migration_error,
            authenticated,
            username,
            expires_at,
        }
    }

    /// Whether authenticated (any account exists)
    pub async fn is_authenticated(&self) -> bool {
        let accounts = self.accounts.read().await;
        !accounts.is_empty()
    }

    /// Clears all auth (logs out every account)
    pub async fn clear_auth(&self) -> Result<(), CopilotAuthError> {
        log::info!("[CopilotAuth] Clearing all auth");

        // Clear in-memory state first so the user sees a logout even if deleting the file fails
        {
            let mut accounts = self.accounts.write().await;
            accounts.clear();
        }
        {
            let mut default_account_id = self.default_account_id.write().await;
            default_account_id.take();
        }
        self.set_migration_error(None).await;
        {
            let mut tokens = self.copilot_tokens.write().await;
            tokens.clear();
        }
        {
            let mut models = self.copilot_models.write().await;
            models.clear();
        }
        {
            let mut refresh_locks = self.refresh_locks.write().await;
            refresh_locks.clear();
        }
        // Clear the API endpoint cache
        {
            let mut api_endpoints = self.api_endpoints.write().await;
            api_endpoints.clear();
        }
        {
            let mut endpoint_locks = self.endpoint_locks.write().await;
            endpoint_locks.clear();
        }

        // Delete the store file last
        if self.storage_path.exists() {
            std::fs::remove_file(&self.storage_path)?;
        }

        Ok(())
    }

    // ==================== Internal ====================

    fn fallback_default_account_id(
        accounts: &HashMap<String, GitHubAccountData>,
    ) -> Option<String> {
        accounts
            .iter()
            .max_by(|(id_a, a), (id_b, b)| {
                a.authenticated_at
                    .cmp(&b.authenticated_at)
                    .then_with(|| id_b.cmp(id_a))
            })
            .map(|(id, _)| id.clone())
    }

    fn sorted_accounts(
        accounts: &HashMap<String, GitHubAccountData>,
        default_account_id: Option<&str>,
    ) -> Vec<GitHubAccount> {
        let mut account_list: Vec<GitHubAccount> =
            accounts.values().map(GitHubAccount::from).collect();
        account_list.sort_by(|a, b| {
            let a_default = default_account_id == Some(a.id.as_str());
            let b_default = default_account_id == Some(b.id.as_str());

            b_default
                .cmp(&a_default)
                .then_with(|| b.authenticated_at.cmp(&a.authenticated_at))
                .then_with(|| a.login.cmp(&b.login))
        });
        account_list
    }

    async fn resolve_default_account_id(&self) -> Option<String> {
        let stored_default = self.default_account_id.read().await.clone();
        let accounts = self.accounts.read().await;

        if let Some(default_id) = stored_default {
            if accounts.contains_key(&default_id) {
                return Some(default_id);
            }
        }

        Self::fallback_default_account_id(&accounts)
    }

    async fn get_refresh_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let refresh_locks = self.refresh_locks.read().await;
            if let Some(lock) = refresh_locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut refresh_locks = self.refresh_locks.write().await;
        Arc::clone(
            refresh_locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    async fn set_migration_error(&self, message: Option<String>) {
        let mut migration_error = self.migration_error.write().await;
        *migration_error = message;
    }

    fn write_store_atomic(&self, content: &str) -> Result<(), CopilotAuthError> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let parent = self
            .storage_path
            .parent()
            .ok_or_else(|| CopilotAuthError::IoError("Invalid store path".to_string()))?;
        let file_name = self
            .storage_path
            .file_name()
            .ok_or_else(|| CopilotAuthError::IoError("Invalid store file name".to_string()))?
            .to_string_lossy()
            .to_string();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp_path = parent.join(format!("{file_name}.tmp.{ts}"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            fs::rename(&tmp_path, &self.storage_path)?;
            fs::set_permissions(&self.storage_path, fs::Permissions::from_mode(0o600))?;
        }

        #[cfg(windows)]
        {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            if self.storage_path.exists() {
                let _ = fs::remove_file(&self.storage_path);
            }
            fs::rename(&tmp_path, &self.storage_path)?;
        }

        Ok(())
    }

    /// Fetches GitHub user info with the given token
    async fn fetch_user_info_with_token(
        &self,
        github_token: &str,
    ) -> Result<GitHubUser, CopilotAuthError> {
        let response = self
            .http_client
            .get(GITHUB_USER_URL)
            .header("Authorization", format!("token {github_token}"))
            .header("User-Agent", COPILOT_USER_AGENT)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        let user: GitHubUser = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!("[CopilotAuth] Got user info: {}", user.login);

        Ok(user)
    }

    /// Gets a Copilot token using a GitHub token
    async fn fetch_copilot_token_with_github_token(
        &self,
        github_token: &str,
        account_id: &str,
    ) -> Result<(), CopilotAuthError> {
        log::debug!("[CopilotAuth] Fetching Copilot token for account {account_id}");

        let response = self
            .http_client
            .get(COPILOT_TOKEN_URL)
            .header("Authorization", format!("token {github_token}"))
            .header("User-Agent", COPILOT_USER_AGENT)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if response.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(CopilotAuthError::NoCopilotSubscription);
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "{status}: {text}"
            )));
        }

        let token_response: CopilotTokenResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!(
            "[CopilotAuth] Got Copilot token for account {}, expires at: {}",
            account_id,
            token_response.expires_at
        );

        let copilot_token = CopilotToken {
            token: token_response.token,
            expires_at: token_response.expires_at,
        };

        let mut tokens = self.copilot_tokens.write().await;
        tokens.insert(account_id.to_string(), copilot_token);

        Ok(())
    }

    // ==================== Storage and migration ====================

    /// Loads from disk (tokens only, no network requests)
    fn load_from_disk_sync(&self) -> Result<(), CopilotAuthError> {
        if !self.storage_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.storage_path)?;
        let store: CopilotAuthStore = serde_json::from_str(&content)
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        if store.version >= 2 {
            // v2 multi-account format
            if let Ok(mut accounts) = self.accounts.try_write() {
                *accounts = store.accounts;
                log::info!("[CopilotAuth] Loaded {} accounts from disk", accounts.len());
            }
            if let Ok(mut default_account_id) = self.default_account_id.try_write() {
                *default_account_id = store.default_account_id;
                if default_account_id.is_none() {
                    if let Ok(accounts) = self.accounts.try_read() {
                        *default_account_id = Self::fallback_default_account_id(&accounts);
                    }
                }
            }
        } else if store.github_token.is_some() {
            // v1 single-account format; mark for migration
            log::info!("[CopilotAuth] Found legacy format; will migrate on first access");
            if let Ok(mut pending) = self.pending_migration.try_write() {
                *pending = store.github_token;
            }
        }

        Ok(())
    }

    /// Makes sure migration is done
    async fn ensure_migration_complete(&self) -> Result<(), CopilotAuthError> {
        let pending = {
            let guard = self.pending_migration.read().await;
            guard.clone()
        };

        if let Some(legacy_token) = pending {
            log::info!("[CopilotAuth] Migrating legacy format");

            // Get user info
            match self.fetch_user_info_with_token(&legacy_token).await {
                Ok(user) => {
                    let account_id = user.id.to_string();

                    // Try getting a Copilot token to verify the subscription
                    if let Err(e) = self
                        .fetch_copilot_token_with_github_token(&legacy_token, &account_id)
                        .await
                    {
                        log::warn!("[CopilotAuth] Failed to verify Copilot subscription during migration: {e}");
                    }

                    // Add the account
                    self.add_account_internal(legacy_token, user).await?;
                    self.set_migration_error(None).await;

                    log::info!("[CopilotAuth] Legacy format migrated");
                }
                Err(e) => {
                    self.set_migration_error(Some(format!(
                        "Legacy Copilot auth migration failed: {e}"
                    )))
                    .await;
                    log::warn!("[CopilotAuth] Migration failed; the old token may be invalid: {e}");
                }
            }

            // Clear the pending-migration marker
            {
                let mut pending = self.pending_migration.write().await;
                *pending = None;
            }
        }

        Ok(())
    }

    /// Saves to disk
    async fn save_to_disk(&self) -> Result<(), CopilotAuthError> {
        let accounts = self.accounts.read().await.clone();
        let default_account_id = self.resolve_default_account_id().await;

        let store = CopilotAuthStore {
            version: 3,
            accounts,
            default_account_id,
            github_token: None,
            authenticated_at: None,
        };

        let content = serde_json::to_string_pretty(&store)
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        self.write_store_atomic(&content)?;

        log::info!(
            "[CopilotAuth] Saved to disk ({} accounts)",
            store.accounts.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_copilot_token_expiry() {
        let now = chrono::Utc::now().timestamp();

        // Unexpired token (expires in 1 hour, outside the 60-second buffer)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now + 3600,
        };
        assert!(!token.is_expiring_soon());

        // Token about to expire (expires in 30 seconds, inside the 60-second buffer)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now + 30,
        };
        assert!(token.is_expiring_soon());

        // Expired token (also inside the buffer)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now - 100,
        };
        assert!(token.is_expiring_soon());
    }

    #[test]
    fn test_auth_status_serialization() {
        let status = CopilotAuthStatus {
            accounts: vec![GitHubAccount {
                id: "12345".to_string(),
                login: "testuser".to_string(),
                avatar_url: Some("https://example.com/avatar.png".to_string()),
                authenticated_at: 1234567890,
            }],
            default_account_id: Some("12345".to_string()),
            migration_error: None,
            authenticated: true,
            username: Some("testuser".to_string()),
            expires_at: Some(1234567890),
        };

        let json = serde_json::to_string(&status).unwrap();
        let parsed: CopilotAuthStatus = serde_json::from_str(&json).unwrap();

        assert!(parsed.authenticated);
        assert_eq!(parsed.default_account_id, Some("12345".to_string()));
        assert_eq!(parsed.username, Some("testuser".to_string()));
        assert_eq!(parsed.expires_at, Some(1234567890));
        assert_eq!(parsed.accounts.len(), 1);
        assert_eq!(parsed.accounts[0].id, "12345");
        assert_eq!(parsed.accounts[0].login, "testuser");
    }

    #[test]
    fn test_multi_account_store_serialization() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "12345".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token".to_string(),
                user: GitHubUser {
                    login: "alice".to_string(),
                    id: 12345,
                    avatar_url: Some("https://example.com/alice.png".to_string()),
                },
                authenticated_at: 1700000000,
            },
        );
        accounts.insert(
            "67890".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token_2".to_string(),
                user: GitHubUser {
                    login: "bob".to_string(),
                    id: 67890,
                    avatar_url: None,
                },
                authenticated_at: 1700000001,
            },
        );

        let store = CopilotAuthStore {
            version: 3,
            accounts,
            default_account_id: Some("67890".to_string()),
            github_token: None,
            authenticated_at: None,
        };

        let json = serde_json::to_string_pretty(&store).unwrap();
        let parsed: CopilotAuthStore = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, 3);
        assert_eq!(parsed.default_account_id, Some("67890".to_string()));
        assert_eq!(parsed.accounts.len(), 2);
        assert!(parsed.accounts.contains_key("12345"));
        assert!(parsed.accounts.contains_key("67890"));
        assert_eq!(parsed.accounts["12345"].user.login, "alice");
        assert_eq!(parsed.accounts["67890"].user.login, "bob");
    }

    #[test]
    fn test_legacy_format_detection() {
        // Legacy format (v1)
        let legacy_json = r#"{
            "github_token": "gho_legacy_token",
            "authenticated_at": 1700000000
        }"#;

        let store: CopilotAuthStore = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(store.version, 0); // default value
        assert!(store.github_token.is_some());
        assert!(store.accounts.is_empty());
    }

    #[test]
    fn test_github_account_from_data() {
        let data = GitHubAccountData {
            github_token: "gho_test".to_string(),
            user: GitHubUser {
                login: "testuser".to_string(),
                id: 99999,
                avatar_url: Some("https://example.com/avatar.png".to_string()),
            },
            authenticated_at: 1700000000,
        };

        let account = GitHubAccount::from(&data);
        assert_eq!(account.id, "99999");
        assert_eq!(account.login, "testuser");
        assert_eq!(
            account.avatar_url,
            Some("https://example.com/avatar.png".to_string())
        );
        assert_eq!(account.authenticated_at, 1700000000);
    }

    #[test]
    fn test_fallback_default_account_prefers_latest_authenticated() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "12345".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token".to_string(),
                user: GitHubUser {
                    login: "alice".to_string(),
                    id: 12345,
                    avatar_url: None,
                },
                authenticated_at: 1700000000,
            },
        );
        accounts.insert(
            "67890".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token_2".to_string(),
                user: GitHubUser {
                    login: "bob".to_string(),
                    id: 67890,
                    avatar_url: None,
                },
                authenticated_at: 1700000001,
            },
        );

        assert_eq!(
            CopilotAuthManager::fallback_default_account_id(&accounts),
            Some("67890".to_string())
        );
    }

    #[tokio::test]
    async fn test_get_model_vendor_from_cache() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        {
            let mut default_account_id = manager.default_account_id.write().await;
            *default_account_id = Some("12345".to_string());
        }
        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                },
            );
        }
        {
            let mut models = manager.copilot_models.write().await;
            models.insert(
                "12345".to_string(),
                vec![
                    CopilotModel {
                        id: "gpt-5.4".to_string(),
                        name: "GPT-5.4".to_string(),
                        vendor: "OpenAI".to_string(),
                        model_picker_enabled: true,
                    },
                    CopilotModel {
                        id: "claude-sonnet-4".to_string(),
                        name: "Claude Sonnet 4".to_string(),
                        vendor: "Anthropic".to_string(),
                        model_picker_enabled: true,
                    },
                ],
            );
        }

        let vendor = manager
            .get_model_vendor_for_account("12345", "gpt-5.4")
            .await
            .unwrap();
        assert_eq!(vendor.as_deref(), Some("OpenAI"));

        let default_vendor = manager.get_model_vendor("claude-sonnet-4").await.unwrap();
        assert_eq!(default_vendor.as_deref(), Some("Anthropic"));
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_cached_value() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Set the api_endpoints cache manually
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise.example.com".to_string(),
            );
        }

        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, "https://copilot-api.enterprise.example.com");
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_default_when_not_cached() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let endpoint = manager.get_api_endpoint("99999").await;
        assert_eq!(endpoint, "https://api.githubcopilot.com");
    }

    #[tokio::test]
    async fn test_get_default_api_endpoint_uses_default_account() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Set the default account
        {
            let mut default_account_id = manager.default_account_id.write().await;
            *default_account_id = Some("12345".to_string());
        }
        // Add account data
        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                },
            );
        }
        // Set the API endpoint cache
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.corp.example.com".to_string(),
            );
        }

        let endpoint = manager.get_default_api_endpoint().await;
        assert_eq!(endpoint, "https://copilot-api.corp.example.com");
    }

    #[tokio::test]
    async fn test_remove_account_clears_api_endpoint_cache() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Add account data
        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                },
            );
        }
        // Set the API endpoint cache
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise.example.com".to_string(),
            );
        }

        // Confirm the cache exists
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert!(api_endpoints.contains_key("12345"));
        }

        // Remove the account
        manager.remove_account("12345").await.unwrap();

        // Confirm the cache was cleared
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert!(!api_endpoints.contains_key("12345"));
        }
    }

    #[tokio::test]
    async fn test_clear_auth_clears_all_api_endpoint_cache() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Add API endpoint caches for several accounts
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise1.example.com".to_string(),
            );
            api_endpoints.insert(
                "67890".to_string(),
                "https://copilot-api.enterprise2.example.com".to_string(),
            );
        }

        // Confirm the caches exist
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert_eq!(api_endpoints.len(), 2);
        }

        // Clear all auth
        manager.clear_auth().await.unwrap();

        // Confirm the caches are empty
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert!(api_endpoints.is_empty());
        }
    }

    #[tokio::test]
    async fn test_clear_auth_cleans_memory_even_when_file_removal_fails() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Create a directory at storage_path so remove_file fails
        std::fs::create_dir_all(&manager.storage_path).unwrap();

        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                },
            );
        }
        {
            let mut default_account_id = manager.default_account_id.write().await;
            *default_account_id = Some("12345".to_string());
        }
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise.example.com".to_string(),
            );
        }

        let result = manager.clear_auth().await;
        // Should still return an error for the file deletion failure
        assert!(result.is_err());

        // But memory state should already be cleaned
        let accounts = manager.accounts.read().await;
        assert!(accounts.is_empty());
        drop(accounts);

        let default_account_id = manager.default_account_id.read().await;
        assert!(default_account_id.is_none());
        drop(default_account_id);

        let api_endpoints = manager.api_endpoints.read().await;
        assert!(api_endpoints.is_empty());
    }

    #[tokio::test]
    async fn test_get_api_endpoint_cache_hit_skips_fetch() {
        // A cache hit returns immediately without a network request
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let enterprise_endpoint = "https://copilot-api.enterprise.example.com".to_string();
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert("12345".to_string(), enterprise_endpoint.clone());
        }

        // Even without account data, a cache hit returns immediately
        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, enterprise_endpoint);
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_default_for_unknown_account() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, DEFAULT_COPILOT_API_ENDPOINT);
    }

    #[tokio::test]
    async fn test_fetch_and_cache_endpoint_requires_account() {
        // With no such account, fetch_and_cache_endpoint returns AccountNotFound
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let result = manager.fetch_and_cache_endpoint("nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CopilotAuthError::AccountNotFound(id) => assert_eq!(id, "nonexistent"),
            other => panic!("expected AccountNotFound, got: {other:?}"),
        }
    }
}
