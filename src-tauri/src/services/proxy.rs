//! Proxy service business logic
//!
//! Starts and stops the proxy server and manages its config

use crate::app_config::AppType;
use crate::config::{get_claude_settings_path, read_json_file, write_json_file};
use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::server::ProxyServer;
use crate::proxy::switch_lock::SwitchLockManager;
use crate::proxy::types::*;
use crate::services::live_merge;
use crate::services::provider::{
    build_effective_settings_with_common_config, write_live_with_common_config,
};
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Placeholder written during live-config takeover (stops clients complaining about a missing key without exposing the real token)
const PROXY_TOKEN_PLACEHOLDER: &str = "PROXY_MANAGED";

/// "Model override" fields removed from the Claude live config in proxy takeover mode.
///
/// Why: switching providers in takeover mode does not write the live config back; if these fields stayed,
/// Claude Code would keep requesting the old model name and fail when the new provider does not support it.
const CLAUDE_MODEL_OVERRIDE_ENV_KEYS: [&str; 6] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_REASONING_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    // Legacy key (deprecated): older versions used this field to select the small/fast model
    "ANTHROPIC_SMALL_FAST_MODEL",
];

/// Claude `env` token keys the takeover replaces with the placeholder or removes.
const CLAUDE_TOKEN_ENV_KEYS: [&str; 4] = [
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "OPENROUTER_API_KEY",
    "OPENAI_API_KEY",
];

#[derive(Clone)]
pub struct ProxyService {
    db: Arc<Database>,
    server: Arc<RwLock<Option<ProxyServer>>>,
    /// AppHandle, passed to ProxyServer so failover can update the UI
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
    switch_locks: SwitchLockManager,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HotSwitchOutcome {
    pub logical_target_changed: bool,
}

impl ProxyService {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            server: Arc::new(RwLock::new(None)),
            app_handle: Arc::new(RwLock::new(None)),
            switch_locks: SwitchLockManager::new(),
        }
    }

    /// Remove the model override fields from the Claude live config in takeover mode.
    ///
    /// This avoids "switching providers after takeover still uses the old model".
    /// Note: the token/base URL takeover placeholders are left alone; only model fields are removed.
    pub fn cleanup_claude_model_overrides_in_live(&self) -> Result<(), String> {
        let mut config = self.read_claude_live()?;

        let Some(env) = config.get_mut("env").and_then(|v| v.as_object_mut()) else {
            return Ok(());
        };

        let mut changed = false;
        for key in CLAUDE_MODEL_OVERRIDE_ENV_KEYS {
            if env.remove(key).is_some() {
                changed = true;
            }
        }

        if changed {
            self.write_claude_live(&config)?;
        }

        Ok(())
    }

    /// True when the current Claude provider is an Official account whose
    /// login has not been captured: the proxy would have nothing to present.
    fn claude_takeover_would_strand_official(&self) -> bool {
        crate::settings::get_effective_current_provider(&self.db, &AppType::Claude)
            .ok()
            .flatten()
            .and_then(|id| self.db.get_provider_by_id(&id, "claude").ok().flatten())
            .is_some_and(|p| {
                p.category.as_deref() == Some("official")
                    && !crate::proxy::claude_pool::is_oauth_provider(&p)
            })
    }

    /// Whether the Claude takeover should leave Claude Code signed in with its
    /// subscription: `provider` (the current one when `None`) is an Official
    /// provider with a captured login.
    fn claude_takeover_keeps_login(&self, provider: Option<&Provider>) -> bool {
        let owned;
        let provider = match provider {
            Some(p) => p,
            None => {
                let Some(id) =
                    crate::settings::get_effective_current_provider(&self.db, &AppType::Claude)
                        .ok()
                        .flatten()
                else {
                    return false;
                };
                let Ok(Some(p)) = self.db.get_provider_by_id(&id, "claude") else {
                    return false;
                };
                owned = p;
                &owned
            }
        };
        crate::proxy::claude_pool::is_oauth_provider(provider)
    }

    /// Points Claude Code at the proxy. With `keep_login`, no token placeholder
    /// is written and any token keys are removed, so Claude Code keeps using
    /// its subscription login and the proxy replaces it per request.
    fn apply_claude_takeover_fields(config: &mut Value, proxy_url: &str, keep_login: bool) {
        if !config.is_object() {
            *config = json!({});
        }

        let root = config
            .as_object_mut()
            .expect("Claude config should be normalized to an object");
        let env = root.entry("env".to_string()).or_insert_with(|| json!({}));
        if !env.is_object() {
            *env = json!({});
        }

        let env = env
            .as_object_mut()
            .expect("Claude env should be normalized to an object");
        env.insert("ANTHROPIC_BASE_URL".to_string(), json!(proxy_url));

        for key in CLAUDE_MODEL_OVERRIDE_ENV_KEYS {
            env.remove(key);
        }

        let token_keys = [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
        ];

        if keep_login {
            for key in token_keys {
                env.remove(key);
            }
            return;
        }

        let mut replaced_any = false;
        for key in token_keys {
            if env.contains_key(key) {
                env.insert(key.to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                replaced_any = true;
            }
        }

        if !replaced_any {
            env.insert(
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                json!(PROXY_TOKEN_PLACEHOLDER),
            );
        }
    }

    /// Rewrites Claude's live settings for `provider` while the proxy has them.
    /// Only the keys a provider owns change (see
    /// `services::provider::claude_provider_owned`), and
    /// only where they differ between `previous` (the provider the file was
    /// written for) and `provider`. Everything else in the file (hooks,
    /// plugins, the status line) belongs to the user and other tools and
    /// stays. Without `previous`, the record of what the takeover wrote is
    /// the base; without a live file the provider's settings are written as
    /// they are.
    pub async fn sync_claude_live_from_provider_while_proxy_active(
        &self,
        provider: &Provider,
        previous: Option<&Provider>,
    ) -> Result<(), String> {
        let (proxy_url, _) = self.build_proxy_urls().await?;
        let effective_settings = self.claude_settings_under_takeover(provider, &proxy_url)?;
        let base = match previous {
            Some(previous) => Some(crate::services::provider::claude_provider_owned(
                &self.claude_settings_under_takeover(previous, &proxy_url)?,
            )),
            None => self
                .live_written(&AppType::Claude)
                .await
                .map(|written| crate::services::provider::claude_provider_owned(&written)),
        };
        let mut merged = match self.read_claude_live().ok() {
            Some(live) => Self::merge_live(
                &AppType::Claude,
                &base.unwrap_or_else(|| json!({})),
                crate::services::provider::claude_provider_owned(&effective_settings),
                &live,
            ),
            None => effective_settings.clone(),
        };
        let keep_login = self.claude_takeover_keeps_login(Some(provider));
        Self::apply_claude_takeover_fields(&mut merged, &proxy_url, keep_login);
        self.write_claude_live(&merged)?;
        self.record_live_written(&AppType::Claude, &effective_settings)
            .await;
        Ok(())
    }

    /// `provider`'s effective Claude settings as the takeover writes them.
    fn claude_settings_under_takeover(
        &self,
        provider: &Provider,
        proxy_url: &str,
    ) -> Result<Value, String> {
        let mut settings = build_effective_settings_with_common_config(
            self.db.as_ref(),
            &AppType::Claude,
            provider,
        )
        .map_err(|e| format!("Could not build the effective Claude config: {e}"))?;
        let keep_login = self.claude_takeover_keeps_login(Some(provider));
        Self::apply_claude_takeover_fields(&mut settings, proxy_url, keep_login);
        Ok(crate::services::provider::sanitize_claude_settings_for_live(&settings))
    }

    /// Set the AppHandle (called during app initialization)
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        futures::executor::block_on(async {
            *self.app_handle.write().await = Some(handle);
        });
    }

    /// Start the proxy server
    pub async fn start(&self) -> Result<ProxyServerInfo, String> {
        // 1. Set proxy_enabled = true on start
        let mut global_config = self
            .db
            .get_global_proxy_config()
            .await
            .map_err(|e| format!("Could not read the global proxy config: {e}"))?;

        if !global_config.proxy_enabled {
            global_config.proxy_enabled = true;
            self.db
                .update_global_proxy_config(global_config.clone())
                .await
                .map_err(|e| format!("Could not update the proxy master switch: {e}"))?;
        }

        // 2. Get the config
        let config = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("Could not read the proxy config: {e}"))?;

        // 3. If already running: make sure the state is persisted (if needed) and return the current info
        if let Some(server) = self.server.read().await.as_ref() {
            let status = server.get_status().await;
            return Ok(ProxyServerInfo {
                address: status.address,
                port: status.port,
                // The original start time cannot be recovered exactly; the current time is enough for the UI
                started_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        // 4. Create and start the server
        let app_handle = self.app_handle.read().await.clone();
        let server = ProxyServer::new(config.clone(), self.db.clone(), app_handle);
        let info = server
            .start()
            .await
            .map_err(|e| format!("Could not start the proxy server: {e}"))?;

        // 5. Keep the server instance
        *self.server.write().await = Some(server);

        log::info!("Proxy server started: {}:{}", info.address, info.port);
        Ok(info)
    }

    /// Start the proxy server (with live-config takeover)
    pub async fn start_with_takeover(&self) -> Result<ProxyServerInfo, String> {
        // 1. Back up each app's live config
        self.backup_live_configs().await?;

        // 2. Sync the token in the live config to the database (so the proxy reads the latest token)
        if let Err(e) = self.sync_live_to_providers().await {
            // The takeover config has not been written yet, but the backup may hold secrets; try to clean it up
            if let Err(clean_err) = self.db.delete_all_live_backups().await {
                log::warn!("Failed to clean up the live backup: {clean_err}");
            }
            return Err(e);
        }

        // 3. Persist the takeover flag before writing the takeover config:
        //    if power is lost or the process is killed mid-takeover, the next start detects it and recovers automatically.
        if let Err(e) = self.db.set_live_takeover_active(true).await {
            if let Err(clean_err) = self.db.delete_all_live_backups().await {
                log::warn!("Failed to clean up the live backup: {clean_err}");
            }
            return Err(format!("Could not set the takeover state: {e}"));
        }

        // 4. Take over each app's live config (write the proxy address, clear the token)
        if let Err(e) = self.takeover_live_configs().await {
            // Takeover failed (possibly a partial write): try to restore the original config; if that fails, keep the flag and backup for automatic recovery on the next start.
            log::error!("Live config takeover failed; trying to restore the original config: {e}");
            match self.restore_live_configs().await {
                Ok(()) => {
                    let _ = self.db.set_live_takeover_active(false).await;
                    let _ = self.db.delete_all_live_backups().await;
                }
                Err(restore_err) => {
                    log::error!("Failed to restore the original config; keeping the backup for recovery on the next start: {restore_err}");
                }
            }
            return Err(e);
        }

        // 5. Start the proxy server
        match self.start().await {
            Ok(info) => Ok(info),
            Err(e) => {
                // Start failed; restore the original config
                log::error!("Proxy failed to start; trying to restore the original config: {e}");
                match self.restore_live_configs().await {
                    Ok(()) => {
                        let _ = self.db.set_live_takeover_active(false).await;
                        let _ = self.db.delete_all_live_backups().await;
                    }
                    Err(restore_err) => {
                        log::error!("Failed to restore the original config; keeping the backup for recovery on the next start: {restore_err}");
                    }
                }
                Err(e)
            }
        }
    }

    /// Get each app's takeover status (whether its live config is rewritten to point at the local proxy)
    pub async fn get_takeover_status(&self) -> Result<ProxyTakeoverStatus, String> {
        // Read from proxy_config.enabled first; still honors the old live_backup detection
        let claude_enabled = self
            .db
            .get_proxy_config_for_app("claude")
            .await
            .map(|c| c.enabled)
            .unwrap_or(false);
        let codex_enabled = self
            .db
            .get_proxy_config_for_app("codex")
            .await
            .map(|c| c.enabled)
            .unwrap_or(false);
        let gemini_enabled = self
            .db
            .get_proxy_config_for_app("gemini")
            .await
            .map(|c| c.enabled)
            .unwrap_or(false);
        // OpenCode and OpenClaw don't support proxy features, always return false
        let opencode_enabled = false;
        let openclaw_enabled = false;

        Ok(ProxyTakeoverStatus {
            claude: claude_enabled,
            codex: codex_enabled,
            gemini: gemini_enabled,
            opencode: opencode_enabled,
            openclaw: openclaw_enabled,
        })
    }

    /// Turn live takeover on or off for an app
    ///
    /// - On: start the proxy service automatically and take over only this app's live config
    /// - Off: restore only this app's live config; if nothing else is taken over, stop the proxy service automatically
    pub async fn set_takeover_for_app(&self, app_type: &str, enabled: bool) -> Result<(), String> {
        let app = AppType::from_str(app_type).map_err(|e| format!("Invalid app type: {e}"))?;
        let app_type_str = app.as_str();

        if enabled {
            // 1) Start the proxy service if it is not running
            if !self.is_running().await {
                self.start().await?;
            }

            // 2) Already taken over: return (idempotent), unless the backup or placeholders are missing, in which case redo the takeover
            let current_config = self
                .db
                .get_proxy_config_for_app(app_type_str)
                .await
                .map_err(|e| format!("Could not read the {app_type_str} config: {e}"))?;

            if current_config.enabled {
                let has_backup = match self.db.get_live_backup(app_type_str).await {
                    Ok(v) => v.is_some(),
                    Err(e) => {
                        log::warn!("Failed to read the {app_type_str} backup (continuing to rebuild the takeover): {e}");
                        false
                    }
                };
                let live_taken_over = self.detect_takeover_in_live_config_for_app(&app);

                if has_backup || live_taken_over {
                    return Ok(());
                }

                log::warn!(
                    "{app_type_str} is marked as taken over but the backup or placeholders are missing; retaking over and restoring the backup"
                );
            }

            // An Official Claude provider is served with its captured login.
            // Without one the proxy has nothing to answer with, so refuse
            // before touching any file.
            if matches!(app, AppType::Claude) && self.claude_takeover_would_strand_official() {
                return Err(
                    "The current Claude provider is an Official account whose login has not been captured. Capture it from the provider card first, or switch to a provider with its own API key."
                        .to_string(),
                );
            }

            // 3) Back up the live config (strict: error if the target app does not exist)
            self.backup_live_config_strict(&app).await?;

            // 4) Sync the live token to the database (this app only)
            if let Err(e) = self.sync_live_to_provider(&app).await {
                self.delete_backups_for_app(&app).await;
                return Err(e);
            }

            // 5) Write the takeover config (this app only)
            if let Err(e) = self.takeover_live_config_strict(&app).await {
                log::error!("{app_type_str} live config takeover failed; trying to restore: {e}");
                match self.restore_live_config_for_app(&app).await {
                    Ok(()) => {
                        // Clear the backup only after a successful restore, so a failure never loses the only rollback source
                        self.delete_backups_for_app(&app).await;
                    }
                    Err(restore_err) => {
                        log::error!(
                            "{app_type_str} failed to restore the live config; keeping the backup for recovery on the next start: {restore_err}"
                        );
                    }
                }
                return Err(e);
            }

            // 6) Set proxy_config.enabled = true
            let mut updated_config = self
                .db
                .get_proxy_config_for_app(app_type_str)
                .await
                .map_err(|e| format!("Could not read the {app_type_str} config: {e}"))?;
            updated_config.enabled = true;
            self.db
                .update_proxy_config_for_app(updated_config)
                .await
                .map_err(|e| format!("Could not set the {app_type_str} enabled state: {e}"))?;

            // 7) Legacy compatibility: write the any-of flag (a failure does not affect functionality)
            let _ = self.db.set_live_takeover_active(true).await;
            return Ok(());
        }

        // Turning takeover off: check the enabled state
        let current_config = self
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| format!("Could not read the {app_type_str} config: {e}"))?;

        if !current_config.enabled {
            return Ok(()); // Not taken over; idempotent return
        }

        // 1) Restore the live config
        self.restore_live_config_for_app(&app).await?;

        // 2) Delete this app's backup (so secrets are not stored long term)
        self.db
            .delete_live_backup(app_type_str)
            .await
            .map_err(|e| format!("Could not delete the {app_type_str} live backup: {e}"))?;
        if let Some(key) = Self::mirror_backup_key(&app) {
            let _ = self.db.delete_live_backup(key).await;
        }

        // 3) Set proxy_config.enabled = false
        let mut updated_config = self
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| format!("Could not read the {app_type_str} config: {e}"))?;
        updated_config.enabled = false;
        self.db
            .update_proxy_config_for_app(updated_config)
            .await
            .map_err(|e| format!("Could not clear the {app_type_str} enabled state: {e}"))?;

        // 4) Clear this app's health status (turning the proxy off resets the queue state)
        self.db
            .clear_provider_health_for_app(app_type_str)
            .await
            .map_err(|e| format!("Could not clear the {app_type_str} health state: {e}"))?;

        // 5) If nothing else is taken over, update the legacy flag and stop the proxy service
        // Check whether any other app still has enabled = true
        let any_enabled = self
            .db
            .is_live_takeover_active()
            .await
            .map_err(|e| format!("Could not check the takeover state: {e}"))?;

        if !any_enabled {
            let _ = self.db.set_live_takeover_active(false).await;

            if self.is_running().await {
                // No app is taken over any more; just stop the service
                let _ = self.stop().await;
            }
        }

        Ok(())
    }

    /// Points Codex's live config at the proxy.
    ///
    /// A ChatGPT login is left in place and the built-in provider is redirected
    /// with `openai_base_url`: Codex stays in ChatGPT mode (same model list, same
    /// session history, which is filed per provider id) and the proxy replaces
    /// the login on each request. Writing the API-key placeholder instead would
    /// switch Codex to API-key mode against an address it does not read.
    ///
    /// Anything else gets the API-key takeover: placeholder key plus the active
    /// model provider's `base_url`.
    fn apply_codex_takeover_fields(
        config: &mut Value,
        proxy_url: &str,
        proxy_codex_base_url: &str,
    ) {
        let config_str = config
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let chatgpt_mode = config
            .get("auth")
            .is_some_and(crate::proxy::codex_pool::is_chatgpt_live_auth)
            && !Self::codex_config_names_model_provider(&config_str);

        if chatgpt_mode {
            let backend_url = format!(
                "{}{}",
                proxy_url.trim_end_matches('/'),
                crate::proxy::codex_pool::BACKEND_PATH_PREFIX
            );
            let config_str = Self::set_codex_openai_base_url(&config_str, Some(&backend_url));
            config["config"] = json!(Self::set_codex_chatgpt_base_url(
                &config_str,
                crate::proxy::local_tls::Install::Windows
            ));
            return;
        }

        if let Some(auth) = config.get_mut("auth").and_then(|v| v.as_object_mut()) {
            auth.insert("OPENAI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
        }
        config["config"] = json!(Self::update_toml_base_url(
            &config_str,
            proxy_codex_base_url
        ));
    }

    fn codex_config_names_model_provider(toml_str: &str) -> bool {
        toml_str
            .parse::<toml_edit::DocumentMut>()
            .ok()
            .and_then(|doc| {
                doc.get("model_provider")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .is_some_and(|name| name != "openai")
    }

    /// Sets or removes the top-level `openai_base_url` key. Removing it also
    /// removes a `chatgpt_base_url` that points at the proxy. Unparseable TOML
    /// is returned untouched.
    fn set_codex_openai_base_url(toml_str: &str, url: Option<&str>) -> String {
        let Ok(mut doc) = toml_str.parse::<toml_edit::DocumentMut>() else {
            return toml_str.to_string();
        };
        match url {
            Some(url) => doc["openai_base_url"] = toml_edit::value(url),
            None => {
                doc.as_table_mut().remove("openai_base_url");
                let chatgpt_is_local = doc
                    .get("chatgpt_base_url")
                    .and_then(|v| v.as_str())
                    .is_some_and(Self::is_local_proxy_url);
                if chatgpt_is_local {
                    doc.as_table_mut().remove("chatgpt_base_url");
                }
            }
        }
        doc.to_string()
    }

    /// Points Codex's other ChatGPT backend calls (the usage `/status` shows)
    /// at the proxy's HTTPS endpoint when `install` trusts it; otherwise they
    /// stay direct, since Codex exits at startup on an untrusted one.
    fn set_codex_chatgpt_base_url(
        toml_str: &str,
        install: crate::proxy::local_tls::Install,
    ) -> String {
        let Some(url) = crate::proxy::local_tls::codex_chatgpt_base_url(install) else {
            return toml_str.to_string();
        };
        let Ok(mut doc) = toml_str.parse::<toml_edit::DocumentMut>() else {
            return toml_str.to_string();
        };
        doc["chatgpt_base_url"] = toml_edit::value(url);
        doc.to_string()
    }

    fn codex_openai_base_url_is_local(toml_str: &str) -> bool {
        toml_str
            .parse::<toml_edit::DocumentMut>()
            .ok()
            .and_then(|doc| {
                doc.get("openai_base_url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .is_some_and(|url| Self::is_local_proxy_url(&url))
    }

    /// Sync the token in the live config to the database
    ///
    /// Called before the live token is cleared, so the provider config in the database has the latest token.
    /// That way the proxy reads the right credentials from the database.
    async fn sync_live_to_provider(&self, app_type: &AppType) -> Result<(), String> {
        let live_config = match app_type {
            AppType::Claude => self.read_claude_live()?,
            AppType::Codex => self.read_codex_live()?,
            AppType::Gemini => self.read_gemini_live()?,
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy features
                return Err("OpenCode cannot be proxied".to_string());
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy features
                return Err("OpenClaw cannot be proxied".to_string());
            }
        };

        self.sync_live_config_to_provider(app_type, &live_config)
            .await
    }

    async fn sync_live_config_to_provider(
        &self,
        app_type: &AppType,
        live_config: &Value,
    ) -> Result<(), String> {
        match app_type {
            AppType::Claude => {
                let provider_id =
                    crate::settings::get_effective_current_provider(&self.db, &AppType::Claude)
                        .map_err(|e| format!("Could not read the current Claude provider: {e}"))?;

                if let Some(provider_id) = provider_id {
                    if let Ok(Some(mut provider)) =
                        self.db.get_provider_by_id(&provider_id, "claude")
                    {
                        if let Some(env) = live_config.get("env").and_then(|v| v.as_object()) {
                            let token_pair = [
                                "ANTHROPIC_AUTH_TOKEN",
                                "ANTHROPIC_API_KEY",
                                "OPENROUTER_API_KEY",
                                "OPENAI_API_KEY",
                            ]
                            .into_iter()
                            .find_map(|key| {
                                env.get(key)
                                    .and_then(|v| v.as_str())
                                    .map(|s| (key, s.trim()))
                            })
                            .filter(|(_, token)| {
                                !token.is_empty() && *token != PROXY_TOKEN_PLACEHOLDER
                            });

                            if let Some((token_key, token)) = token_pair {
                                let env_obj = provider
                                    .settings_config
                                    .get_mut("env")
                                    .and_then(|v| v.as_object_mut());

                                match env_obj {
                                    Some(obj) => {
                                        if token_key == "ANTHROPIC_AUTH_TOKEN"
                                            || token_key == "ANTHROPIC_API_KEY"
                                        {
                                            let mut updated = false;
                                            if obj.contains_key("ANTHROPIC_AUTH_TOKEN") {
                                                obj.insert(
                                                    "ANTHROPIC_AUTH_TOKEN".to_string(),
                                                    json!(token),
                                                );
                                                updated = true;
                                            }
                                            if obj.contains_key("ANTHROPIC_API_KEY") {
                                                obj.insert(
                                                    "ANTHROPIC_API_KEY".to_string(),
                                                    json!(token),
                                                );
                                                updated = true;
                                            }
                                            if !updated {
                                                obj.insert(token_key.to_string(), json!(token));
                                            }
                                        } else {
                                            obj.insert(token_key.to_string(), json!(token));
                                        }
                                    }
                                    None => {
                                        // Write at least one usable token
                                        if provider.settings_config.is_null() {
                                            provider.settings_config = json!({});
                                        }

                                        if let Some(root) = provider.settings_config.as_object_mut()
                                        {
                                            root.insert(
                                                "env".to_string(),
                                                json!({ token_key: token }),
                                            );
                                        } else {
                                            log::warn!(
                                                "Claude provider settings_config is malformed (not an object); skipping the token write (provider: {provider_id})"
                                            );
                                        }
                                    }
                                }

                                if let Err(e) = self.db.update_provider_settings_config(
                                    "claude",
                                    &provider_id,
                                    &provider.settings_config,
                                ) {
                                    log::warn!(
                                        "Failed to sync the Claude token to the database: {e}"
                                    );
                                } else {
                                    log::info!(
                                        "Synced the Claude token to the database (provider: {provider_id})"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            AppType::Codex => {
                let provider_id =
                    crate::settings::get_effective_current_provider(&self.db, &AppType::Codex)
                        .map_err(|e| format!("Could not read the current Codex provider: {e}"))?;

                if let Some(provider_id) = provider_id {
                    if let Ok(Some(mut provider)) =
                        self.db.get_provider_by_id(&provider_id, "codex")
                    {
                        if let Some(token) = live_config
                            .get("auth")
                            .and_then(|v| v.get("OPENAI_API_KEY"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty() && *s != PROXY_TOKEN_PLACEHOLDER)
                        {
                            if let Some(auth_obj) = provider
                                .settings_config
                                .get_mut("auth")
                                .and_then(|v| v.as_object_mut())
                            {
                                auth_obj.insert("OPENAI_API_KEY".to_string(), json!(token));
                            } else {
                                if provider.settings_config.is_null() {
                                    provider.settings_config = json!({});
                                }

                                if let Some(root) = provider.settings_config.as_object_mut() {
                                    root.insert(
                                        "auth".to_string(),
                                        json!({ "OPENAI_API_KEY": token }),
                                    );
                                } else {
                                    log::warn!(
                                        "Codex provider settings_config is malformed (not an object); skipping the token write (provider: {provider_id})"
                                    );
                                }
                            }

                            if let Err(e) = self.db.update_provider_settings_config(
                                "codex",
                                &provider_id,
                                &provider.settings_config,
                            ) {
                                log::warn!("Failed to sync the Codex token to the database: {e}");
                            } else {
                                log::info!("Synced the Codex token to the database (provider: {provider_id})");
                            }
                        }
                    }
                }
            }
            AppType::Gemini => {
                let provider_id =
                    crate::settings::get_effective_current_provider(&self.db, &AppType::Gemini)
                        .map_err(|e| format!("Could not read the current Gemini provider: {e}"))?;

                if let Some(provider_id) = provider_id {
                    if let Ok(Some(mut provider)) =
                        self.db.get_provider_by_id(&provider_id, "gemini")
                    {
                        if let Some(token) = live_config
                            .get("env")
                            .and_then(|v| v.get("GEMINI_API_KEY"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty() && *s != PROXY_TOKEN_PLACEHOLDER)
                        {
                            if let Some(env_obj) = provider
                                .settings_config
                                .get_mut("env")
                                .and_then(|v| v.as_object_mut())
                            {
                                env_obj.insert("GEMINI_API_KEY".to_string(), json!(token));
                            } else {
                                if provider.settings_config.is_null() {
                                    provider.settings_config = json!({});
                                }

                                if let Some(root) = provider.settings_config.as_object_mut() {
                                    root.insert(
                                        "env".to_string(),
                                        json!({ "GEMINI_API_KEY": token }),
                                    );
                                } else {
                                    log::warn!(
                                        "Gemini provider settings_config is malformed (not an object); skipping the token write (provider: {provider_id})"
                                    );
                                }
                            }

                            if let Err(e) = self.db.update_provider_settings_config(
                                "gemini",
                                &provider_id,
                                &provider.settings_config,
                            ) {
                                log::warn!("Failed to sync the Gemini token to the database: {e}");
                            } else {
                                log::info!(
                                    "Synced the Gemini token to the database (provider: {provider_id})"
                                );
                            }
                        }
                    }
                }
            }
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy features, skip silently
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy features, skip silently
            }
        }

        Ok(())
    }

    async fn sync_live_to_providers(&self) -> Result<(), String> {
        if let Ok(live_config) = self.read_claude_live() {
            self.sync_live_config_to_provider(&AppType::Claude, &live_config)
                .await?;
        }

        if let Ok(live_config) = self.read_codex_live() {
            self.sync_live_config_to_provider(&AppType::Codex, &live_config)
                .await?;
        }

        if let Ok(live_config) = self.read_gemini_live() {
            self.sync_live_config_to_provider(&AppType::Gemini, &live_config)
                .await?;
        }

        log::info!("Live config token sync complete");
        Ok(())
    }

    /// Stop the proxy server
    pub async fn stop(&self) -> Result<(), String> {
        if let Some(server) = self.server.write().await.take() {
            server
                .stop()
                .await
                .map_err(|e| format!("Could not stop the proxy server: {e}"))?;

            // Set proxy_enabled = false on stop
            let mut global_config = self
                .db
                .get_global_proxy_config()
                .await
                .map_err(|e| format!("Could not read the global proxy config: {e}"))?;

            if global_config.proxy_enabled {
                global_config.proxy_enabled = false;
                if let Err(e) = self.db.update_global_proxy_config(global_config).await {
                    log::warn!("Could not update the proxy master switch: {e}");
                }
            }

            log::info!("Proxy server stopped");
            Ok(())
        } else {
            Err("The proxy server is not running".to_string())
        }
    }

    /// Stop the proxy server and restore the live config (used when the user turns it off)
    ///
    /// Clears the proxy state in the settings table, so it is not restored automatically on the next start.
    pub async fn stop_with_restore(&self) -> Result<(), String> {
        // 1. Stop the proxy server (restore continues even if it is not running)
        if let Err(e) = self.stop().await {
            log::warn!(
                "Failed to stop the proxy server (continuing to restore the live config): {e}"
            );
        }

        // 2. Restore the original live config
        self.restore_live_configs().await?;

        // 3. Clear the takeover state in the proxy_config table (legacy compatibility)
        self.db
            .set_live_takeover_active(false)
            .await
            .map_err(|e| format!("Could not clear the takeover state: {e}"))?;

        // 4. Clear every app's enabled state (turned off by the user; no automatic restore next time)
        for app_type in ["claude", "codex", "gemini"] {
            if let Ok(mut config) = self.db.get_proxy_config_for_app(app_type).await {
                if config.enabled {
                    config.enabled = false;
                    if let Err(e) = self.db.update_proxy_config_for_app(config).await {
                        log::warn!("Failed to clear the {app_type} enabled state: {e}");
                    }
                }
            }
        }

        // 5. Delete the backups
        self.db
            .delete_all_live_backups()
            .await
            .map_err(|e| format!("Could not delete the backup: {e}"))?;

        // 6. Reset health status (so the health badges return to normal)
        self.db
            .clear_all_provider_health()
            .await
            .map_err(|e| format!("Could not reset the health state: {e}"))?;

        // Note: the failover queue and switch are kept for the next time the proxy is turned on
        log::info!("Proxy stopped; live config restored");
        Ok(())
    }

    /// Stop the proxy server and restore the live config, keeping the proxy state in the settings table
    ///
    /// Used on a normal app exit, so the proxy state is restored automatically on the next start
    pub async fn stop_with_restore_keep_state(&self) -> Result<(), String> {
        // 1. Stop the proxy server (restore continues even if it is not running)
        if let Err(e) = self.stop().await {
            log::warn!(
                "Failed to stop the proxy server (continuing to restore the live config): {e}"
            );
        }

        // 2. Restore the original live config
        self.restore_live_configs().await?;

        // 3. Update the live_takeover_active flag in the proxy_config table (legacy compatibility)
        //    Note: proxy_config.enabled is kept so it is restored automatically on the next start
        if let Ok(mut config) = self.db.get_proxy_config().await {
            config.live_takeover_active = false;
            let _ = self.db.update_proxy_config(config).await;
        }

        // 4. Delete the backups (the live config is restored, so they are no longer needed)
        self.db
            .delete_all_live_backups()
            .await
            .map_err(|e| format!("Could not delete the backup: {e}"))?;

        // 5. Reset health status
        self.db
            .clear_all_provider_health()
            .await
            .map_err(|e| format!("Could not reset the health state: {e}"))?;

        log::info!("Proxy stopped; live config restored (proxy state kept; it will be restored on the next start)");
        Ok(())
    }

    /// Back up each app's live config
    async fn backup_live_configs(&self) -> Result<(), String> {
        // Claude
        if let Ok(config) = self.read_claude_live() {
            let json_str = serde_json::to_string(&config)
                .map_err(|e| format!("Could not serialize the Claude config: {e}"))?;
            self.db
                .start_live_backup("claude", &json_str)
                .await
                .map_err(|e| format!("Could not back up the Claude config: {e}"))?;
        }

        // Codex
        if let Ok(config) = self.read_codex_live() {
            let json_str = serde_json::to_string(&config)
                .map_err(|e| format!("Could not serialize the Codex config: {e}"))?;
            self.db
                .start_live_backup("codex", &json_str)
                .await
                .map_err(|e| format!("Could not back up the Codex config: {e}"))?;
        }

        // Gemini
        if let Ok(config) = self.read_gemini_live() {
            let json_str = serde_json::to_string(&config)
                .map_err(|e| format!("Could not serialize the Gemini config: {e}"))?;
            self.db
                .start_live_backup("gemini", &json_str)
                .await
                .map_err(|e| format!("Could not back up the Gemini config: {e}"))?;
        }

        log::info!("Backed up the live config of every app");
        Ok(())
    }

    /// Back up an app's live config (strict: error if the target config does not exist)
    async fn backup_live_config_strict(&self, app_type: &AppType) -> Result<(), String> {
        let (app_type_str, config) = match app_type {
            AppType::Claude => ("claude", self.read_claude_live()?),
            AppType::Codex => ("codex", self.read_codex_live()?),
            AppType::Gemini => ("gemini", self.read_gemini_live()?),
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy features
                return Err("OpenCode cannot be proxied".to_string());
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy features
                return Err("OpenClaw cannot be proxied".to_string());
            }
        };

        let json_str = serde_json::to_string(&config)
            .map_err(|e| format!("Could not serialize the {app_type_str} config: {e}"))?;
        self.db
            .start_live_backup(app_type_str, &json_str)
            .await
            .map_err(|e| format!("Could not back up the {app_type_str} config: {e}"))?;

        Ok(())
    }

    /// Build the proxy address written to the live config (handles 0.0.0.0, IPv6 and other special cases)
    async fn build_proxy_urls(&self) -> Result<(String, String), String> {
        let config = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("Could not read the proxy config: {e}"))?;

        // listen_address may be 0.0.0.0 (listen on all interfaces), but clients cannot connect to 0.0.0.0;
        // so the loopback address is preferred when writing back to each app's config.
        let connect_host = match config.listen_address.as_str() {
            "0.0.0.0" => "127.0.0.1".to_string(),
            "::" => "::1".to_string(),
            _ => config.listen_address.clone(),
        };
        let connect_host_for_url = if connect_host.contains(':') && !connect_host.starts_with('[') {
            format!("[{connect_host}]")
        } else {
            connect_host
        };

        let proxy_origin = format!("http://{}:{}", connect_host_for_url, config.listen_port);
        let proxy_url = proxy_origin.clone();
        let proxy_codex_base_url = format!("{}/v1", proxy_origin.trim_end_matches('/'));

        Ok((proxy_url, proxy_codex_base_url))
    }

    /// Take over each app's live config (write the proxy address)
    ///
    /// The proxy server's routes already tell app types apart by API endpoint:
    /// - `/v1/messages` → Claude
    /// - `/v1/chat/completions`, `/v1/responses` → Codex
    /// - `/v1beta/*` → Gemini
    ///
    /// so no app prefix is needed in the URL.
    async fn takeover_live_configs(&self) -> Result<(), String> {
        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;

        // Claude: set ANTHROPIC_BASE_URL and replace the real token with a placeholder (the proxy injects the real token)
        if let Ok(mut live_config) = self.read_claude_live() {
            let keep_login = self.claude_takeover_keeps_login(None);
            Self::apply_claude_takeover_fields(&mut live_config, &proxy_url, keep_login);
            self.write_claude_live_during_takeover(&live_config).await?;
            log::info!("Claude live config taken over, proxy address: {proxy_url}");
            self.take_over_mirror(&AppType::Claude, &proxy_url, keep_login)
                .await;
        }

        // Codex: set base_url in config.toml and OPENAI_API_KEY in auth.json (the proxy injects the real token)
        if let Ok(mut live_config) = self.read_codex_live() {
            Self::apply_codex_takeover_fields(&mut live_config, &proxy_url, &proxy_codex_base_url);
            self.write_codex_live_during_takeover(&live_config).await?;
            log::info!("Codex live config taken over, proxy address: {proxy_codex_base_url}");
            self.take_over_mirror(&AppType::Codex, &proxy_url, false)
                .await;
        }

        // Gemini: set GOOGLE_GEMINI_BASE_URL and replace the real token with a placeholder (the proxy injects the real token)
        if let Ok(mut live_config) = self.read_gemini_live() {
            if let Some(env) = live_config.get_mut("env").and_then(|v| v.as_object_mut()) {
                env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(&proxy_url));
                // Use a placeholder so no missing-key warning is shown
                env.insert("GEMINI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
            } else {
                live_config["env"] = json!({
                    "GOOGLE_GEMINI_BASE_URL": &proxy_url,
                    "GEMINI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                });
            }
            self.write_gemini_live(&live_config)?;
            log::info!("Gemini live config taken over, proxy address: {proxy_url}");
        }

        Ok(())
    }

    /// Take over an app's live config (strict: error if the target config does not exist)
    async fn takeover_live_config_strict(&self, app_type: &AppType) -> Result<(), String> {
        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;

        match app_type {
            AppType::Claude => {
                let mut live_config = self.read_claude_live()?;
                let keep_login = self.claude_takeover_keeps_login(None);
                Self::apply_claude_takeover_fields(&mut live_config, &proxy_url, keep_login);
                self.write_claude_live_during_takeover(&live_config).await?;
                log::info!("Claude live config taken over, proxy address: {proxy_url}");
                self.take_over_mirror(app_type, &proxy_url, keep_login)
                    .await;
            }
            AppType::Codex => {
                let mut live_config = self.read_codex_live()?;
                Self::apply_codex_takeover_fields(
                    &mut live_config,
                    &proxy_url,
                    &proxy_codex_base_url,
                );
                self.write_codex_live_during_takeover(&live_config).await?;
                log::info!("Codex live config taken over, proxy address: {proxy_codex_base_url}");
                self.take_over_mirror(app_type, &proxy_url, false).await;
            }
            AppType::Gemini => {
                let mut live_config = self.read_gemini_live()?;

                if let Some(env) = live_config.get_mut("env").and_then(|v| v.as_object_mut()) {
                    env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(&proxy_url));
                    env.insert("GEMINI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                } else {
                    live_config["env"] = json!({
                        "GOOGLE_GEMINI_BASE_URL": &proxy_url,
                        "GEMINI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                    });
                }

                self.write_gemini_live(&live_config)?;
                log::info!("Gemini live config taken over, proxy address: {proxy_url}");
            }
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy features
                return Err("OpenCode cannot be proxied".to_string());
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy features
                return Err("OpenClaw cannot be proxied".to_string());
            }
        }

        Ok(())
    }

    /// Take over an app's live config (best effort: skip if the config is missing or unreadable)
    async fn takeover_live_config_best_effort(&self, app_type: &AppType) -> Result<(), String> {
        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;

        match app_type {
            AppType::Claude => {
                if let Ok(mut live_config) = self.read_claude_live() {
                    let keep_login = self.claude_takeover_keeps_login(None);
                    Self::apply_claude_takeover_fields(&mut live_config, &proxy_url, keep_login);
                    let _ = self.write_claude_live_during_takeover(&live_config).await;
                    self.take_over_mirror(app_type, &proxy_url, keep_login)
                        .await;
                }
            }
            AppType::Codex => {
                if let Ok(mut live_config) = self.read_codex_live() {
                    Self::apply_codex_takeover_fields(
                        &mut live_config,
                        &proxy_url,
                        &proxy_codex_base_url,
                    );
                    let _ = self.write_codex_live_during_takeover(&live_config).await;
                    self.take_over_mirror(app_type, &proxy_url, false).await;
                }
            }
            AppType::Gemini => {
                if let Ok(mut live_config) = self.read_gemini_live() {
                    if let Some(env) = live_config.get_mut("env").and_then(|v| v.as_object_mut()) {
                        env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(&proxy_url));
                        env.insert("GEMINI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                    } else {
                        live_config["env"] = json!({
                            "GOOGLE_GEMINI_BASE_URL": &proxy_url,
                            "GEMINI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                        });
                    }

                    let _ = self.write_gemini_live(&live_config);
                }
            }
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy features, skip silently
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy features, skip silently
            }
        }

        Ok(())
    }

    /// Restore an app's live config (does nothing without a backup)
    async fn restore_live_config_for_app(&self, app_type: &AppType) -> Result<(), String> {
        let _guard = self.switch_locks.lock_for_app(app_type.as_str()).await;
        self.restore_live_config_for_app_inner(app_type).await
    }

    async fn restore_live_config_for_app_inner(&self, app_type: &AppType) -> Result<(), String> {
        if !matches!(app_type, AppType::Claude | AppType::Codex | AppType::Gemini) {
            // OpenCode / OpenClaw / Kimi don't support proxy features, skip silently
            return Ok(());
        }
        let app_type_str = app_type.as_str();
        self.restore_mirror(app_type).await;
        if let Ok(Some(backup)) = self.db.get_live_backup(app_type_str).await {
            let config: Value = serde_json::from_str(&backup.original_config)
                .map_err(|e| format!("Could not parse the {app_type_str} backup: {e}"))?;
            let config = self.config_to_restore(app_type, config).await;
            self.write_live_config_for_app(app_type, &config)?;
            log::info!("{app_type_str} live config restored");
        }

        Ok(())
    }

    /// Restore the original live config
    async fn restore_live_configs(&self) -> Result<(), String> {
        let mut errors = Vec::new();

        for app_type in [AppType::Claude, AppType::Codex, AppType::Gemini] {
            if let Err(e) = self
                .restore_live_config_for_app_with_fallback(&app_type)
                .await
            {
                errors.push(e);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    async fn restore_live_config_for_app_with_fallback(
        &self,
        app_type: &AppType,
    ) -> Result<(), String> {
        let _guard = self.switch_locks.lock_for_app(app_type.as_str()).await;
        self.restore_live_config_for_app_with_fallback_inner(app_type)
            .await
    }

    async fn restore_live_config_for_app_with_fallback_inner(
        &self,
        app_type: &AppType,
    ) -> Result<(), String> {
        let app_type_str = app_type.as_str();
        self.restore_mirror(app_type).await;

        // 1) Restore from the live backup first (the only reliable source of the "original live" config)
        let backup = self
            .db
            .get_live_backup(app_type_str)
            .await
            .map_err(|e| format!("Could not read the {app_type_str} live backup: {e}"))?;
        if let Some(backup) = backup {
            let config: Value = serde_json::from_str(&backup.original_config)
                .map_err(|e| format!("Could not parse the {app_type_str} backup: {e}"))?;
            let config = self.config_to_restore(app_type, config).await;
            self.write_live_config_for_app(app_type, &config)?;
            log::info!("{app_type_str} live config restored from backup");
            return Ok(());
        }

        // 2) Fallback: the backup is missing but the live config still has takeover placeholders (abnormal exit or a past bug)
        if !self.detect_takeover_in_live_config_for_app(app_type) {
            return Ok(());
        }

        // 2.1) Prefer rebuilding the live config from the SSOT (current provider), which is more useful than "clearing fields"
        match self.restore_live_from_ssot_for_app(app_type).await {
            Ok(true) => {
                log::info!(
                    "{app_type_str} live config restored from the SSOT (no-backup fallback)"
                );
                return Ok(());
            }
            Ok(false) => {
                log::warn!(
                    "{app_type_str} live backup is missing and the SSOT restore is not possible; trying to clear the takeover placeholders"
                );
            }
            Err(e) => {
                log::error!(
                    "{app_type_str} live backup is missing and the SSOT restore failed; trying to clear the takeover placeholders: {e}"
                );
            }
        }

        // 2.2) Last resort: clear the placeholders and local proxy address as far as possible, so the config is not stuck on proxy placeholders
        self.cleanup_takeover_placeholders_in_live_for_app(app_type)?;
        log::info!("{app_type_str} live takeover placeholders cleared (no-backup fallback)");
        Ok(())
    }

    fn write_live_config_for_app(&self, app_type: &AppType, config: &Value) -> Result<(), String> {
        match app_type {
            AppType::Claude => self.write_claude_live(config),
            AppType::Codex => self.write_codex_live(config),
            AppType::Gemini => self.write_gemini_live(config),
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy features
                Err("OpenCode cannot be proxied".to_string())
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy features
                Err("OpenClaw cannot be proxied".to_string())
            }
        }
    }

    pub fn detect_takeover_in_live_config_for_app(&self, app_type: &AppType) -> bool {
        match app_type {
            AppType::Claude => match self.read_claude_live() {
                Ok(config) => Self::is_claude_live_taken_over(&config),
                Err(_) => false,
            },
            AppType::Codex => match self.read_codex_live() {
                Ok(config) => Self::is_codex_live_taken_over(&config),
                Err(_) => false,
            },
            AppType::Gemini => match self.read_gemini_live() {
                Ok(config) => Self::is_gemini_live_taken_over(&config),
                Err(_) => false,
            },
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy takeover
                false
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy takeover
                false
            }
        }
    }

    /// When the live backup is missing, write the live config back from the SSOT (current provider) to undo the placeholder takeover.
    ///
    /// Returns:
    /// - Ok(true): written back
    /// - Ok(false): no current provider, or the provider does not exist; nothing written
    async fn restore_live_from_ssot_for_app(&self, app_type: &AppType) -> Result<bool, String> {
        let current_id = crate::settings::get_effective_current_provider(&self.db, app_type)
            .map_err(|e| format!("Could not read the current {app_type:?} provider: {e}"))?;

        let Some(current_id) = current_id else {
            return Ok(false);
        };

        let providers = self
            .db
            .get_all_providers(app_type.as_str())
            .map_err(|e| format!("Could not read the {app_type:?} provider list: {e}"))?;

        let Some(provider) = providers.get(&current_id) else {
            return Ok(false);
        };

        if matches!(app_type, AppType::Claude | AppType::Codex) {
            // Same rule as a restore from the backup: only the keys the
            // takeover manages change.
            let target = build_effective_settings_with_common_config(
                self.db.as_ref(),
                app_type,
                provider,
            )
            .map_err(|e| format!("Could not build the effective {app_type:?} config: {e}"))?;
            let config = self.config_to_restore(app_type, target).await;
            self.write_live_config_for_app(app_type, &config)?;
            return Ok(true);
        }

        write_live_with_common_config(self.db.as_ref(), app_type, provider)
            .map_err(|e| format!("Could not write the {app_type:?} live config: {e}"))?;

        Ok(true)
    }

    fn cleanup_takeover_placeholders_in_live_for_app(
        &self,
        app_type: &AppType,
    ) -> Result<(), String> {
        match app_type {
            AppType::Claude => self.cleanup_claude_takeover_placeholders_in_live(),
            AppType::Codex => self.cleanup_codex_takeover_placeholders_in_live(),
            AppType::Gemini => self.cleanup_gemini_takeover_placeholders_in_live(),
            AppType::OpenCode | AppType::Kimi => {
                // OpenCode doesn't support proxy features
                Ok(())
            }
            AppType::OpenClaw => {
                // OpenClaw doesn't support proxy features
                Ok(())
            }
        }
    }

    fn is_local_proxy_url(url: &str) -> bool {
        let url = url.trim();
        let Some(rest) = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
        else {
            return false;
        };
        rest.starts_with("127.0.0.1")
            || rest.starts_with("localhost")
            || rest.starts_with("0.0.0.0")
            || rest.starts_with("[::1]")
            || rest.starts_with("[::]")
            || rest.starts_with("::1")
            || rest.starts_with("::")
    }

    fn cleanup_claude_takeover_placeholders_in_live(&self) -> Result<(), String> {
        let mut config = self.read_claude_live()?;
        if config.get("env").and_then(|v| v.as_object()).is_none() {
            return Ok(());
        }
        Self::strip_claude_takeover(&mut config);
        self.write_claude_live(&config)?;
        Ok(())
    }

    /// Removes the token placeholders and the local proxy address a Claude
    /// takeover writes.
    fn strip_claude_takeover(config: &mut Value) {
        let Some(env) = config.get_mut("env").and_then(|v| v.as_object_mut()) else {
            return;
        };

        for key in CLAUDE_TOKEN_ENV_KEYS {
            if env.get(key).and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER) {
                env.remove(key);
            }
        }

        if env
            .get("ANTHROPIC_BASE_URL")
            .and_then(|v| v.as_str())
            .map(Self::is_local_proxy_url)
            .unwrap_or(false)
        {
            env.remove("ANTHROPIC_BASE_URL");
        }
    }

    fn cleanup_codex_takeover_placeholders_in_live(&self) -> Result<(), String> {
        let mut config = self.read_codex_live()?;

        if let Some(auth) = config.get_mut("auth").and_then(|v| v.as_object_mut()) {
            if auth.get("OPENAI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER)
            {
                auth.remove("OPENAI_API_KEY");
            }
        }

        if let Some(cfg_str) = config.get("config").and_then(|v| v.as_str()) {
            let mut updated = Self::remove_local_toml_base_url(cfg_str);
            if Self::codex_openai_base_url_is_local(&updated) {
                updated = Self::set_codex_openai_base_url(&updated, None);
            }
            config["config"] = json!(updated);
        }

        self.write_codex_live(&config)?;
        Ok(())
    }

    /// Remove local proxy base_url from TOML (delegates to the shared codex_config implementation)
    fn remove_local_toml_base_url(toml_str: &str) -> String {
        crate::codex_config::remove_codex_toml_base_url_if(toml_str, Self::is_local_proxy_url)
    }

    fn cleanup_gemini_takeover_placeholders_in_live(&self) -> Result<(), String> {
        let mut config = self.read_gemini_live()?;

        let Some(env) = config.get_mut("env").and_then(|v| v.as_object_mut()) else {
            return Ok(());
        };

        if env.get("GEMINI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER) {
            env.remove("GEMINI_API_KEY");
        }

        if env
            .get("GOOGLE_GEMINI_BASE_URL")
            .and_then(|v| v.as_str())
            .map(Self::is_local_proxy_url)
            .unwrap_or(false)
        {
            env.remove("GOOGLE_GEMINI_BASE_URL");
        }

        self.write_gemini_live(&config)?;
        Ok(())
    }

    /// Check whether live takeover mode is active
    pub async fn is_takeover_active(&self) -> Result<bool, String> {
        let status = self.get_takeover_status().await?;
        Ok(status.claude || status.codex || status.gemini)
    }

    /// Recover from an abnormal exit (called on startup)
    ///
    /// Called when a leftover live backup is detected.
    /// Restores the live config, clears the takeover flag and deletes the backup.
    pub async fn recover_from_crash(&self) -> Result<(), String> {
        // 1. Restore the live config
        self.restore_live_configs().await?;

        // 2. Clear the takeover flag
        self.db
            .set_live_takeover_active(false)
            .await
            .map_err(|e| format!("Could not clear the takeover state: {e}"))?;

        // 3. Delete the backup
        self.db
            .delete_all_live_backups()
            .await
            .map_err(|e| format!("Could not delete the backup: {e}"))?;

        log::info!("Live config recovered after an abnormal exit");
        Ok(())
    }

    /// Detect whether the live config is left in a "taken over" state
    ///
    /// A fallback: when the database backup is missing but the live file already holds proxy placeholders,
    /// startup can use this to trigger the restore.
    pub fn detect_takeover_in_live_configs(&self) -> bool {
        if let Ok(config) = self.read_claude_live() {
            if Self::is_claude_live_taken_over(&config) {
                return true;
            }
        }

        if let Ok(config) = self.read_codex_live() {
            if Self::is_codex_live_taken_over(&config) {
                return true;
            }
        }

        if let Ok(config) = self.read_gemini_live() {
            if Self::is_gemini_live_taken_over(&config) {
                return true;
            }
        }

        [AppType::Claude, AppType::Codex]
            .iter()
            .any(|app| Self::mirror_is_taken_over(app))
    }

    fn is_claude_live_taken_over(config: &Value) -> bool {
        let env = match config.get("env").and_then(|v| v.as_object()) {
            Some(env) => env,
            None => return false,
        };

        if env
            .get("ANTHROPIC_BASE_URL")
            .and_then(|v| v.as_str())
            .is_some_and(Self::is_local_proxy_url)
        {
            return true;
        }

        for key in [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
        ] {
            if env.get(key).and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER) {
                return true;
            }
        }

        false
    }

    fn is_codex_live_taken_over(config: &Value) -> bool {
        if config
            .get("config")
            .and_then(|v| v.as_str())
            .is_some_and(Self::codex_openai_base_url_is_local)
        {
            return true;
        }
        let auth = match config.get("auth").and_then(|v| v.as_object()) {
            Some(auth) => auth,
            None => return false,
        };
        auth.get("OPENAI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER)
    }

    fn is_gemini_live_taken_over(config: &Value) -> bool {
        let env = match config.get("env").and_then(|v| v.as_object()) {
            Some(env) => env,
            None => return false,
        };
        env.get("GEMINI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER)
    }

    /// Update the live backup from the provider config (for hot switching in proxy mode)
    ///
    /// Unlike backup_live_configs(), this builds the backup from the provider's settings_config
    /// rather than reading the live file (which the proxy has taken over).
    pub async fn update_live_backup_from_provider(
        &self,
        app_type: &str,
        provider: &Provider,
    ) -> Result<(), String> {
        let _guard = self.switch_locks.lock_for_app(app_type).await;
        self.update_live_backup_from_provider_inner(app_type, provider)
            .await
    }

    /// Only for callers that already hold the per-app switch lock. A Codex
    /// backup changes only in the keys `provider` owns.
    async fn update_live_backup_from_provider_inner(
        &self,
        app_type: &str,
        provider: &Provider,
    ) -> Result<(), String> {
        let app_type_enum =
            AppType::from_str(app_type).map_err(|_| format!("Unknown app type: {app_type}"))?;
        let mut effective_settings =
            build_effective_settings_with_common_config(self.db.as_ref(), &app_type_enum, provider)
                .map_err(|e| format!("Could not build the effective {app_type} config: {e}"))?;

        if matches!(app_type_enum, AppType::Codex) {
            let existing_backup = self
                .db
                .get_live_backup(app_type)
                .await
                .map_err(|e| format!("Could not read the existing {app_type} backup: {e}"))?;

            if let Some(existing_backup) = existing_backup {
                let existing_value: Value = serde_json::from_str(&existing_backup.original_config)
                    .map_err(|e| format!("Could not parse the existing {app_type} backup: {e}"))?;
                // Only the keys the incoming provider owns change in the
                // backup; the rest is the user's config as it was.
                let old_text = existing_value
                    .get("config")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let new_text = effective_settings
                    .get("config")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let official = provider.category.as_deref() == Some("official");
                match crate::services::provider::codex_config_after_switch(
                    &old_text, &new_text, official,
                ) {
                    Ok(text) => effective_settings["config"] = json!(text),
                    Err(e) => log::warn!("Could not carry the switch into the Codex backup: {e}"),
                }
            }
        }

        let backup_json = match app_type_enum {
            AppType::Claude => serde_json::to_string(&effective_settings)
                .map_err(|e| format!("Could not serialize the Claude config: {e}"))?,
            AppType::Codex => serde_json::to_string(&effective_settings)
                .map_err(|e| format!("Could not serialize the Codex config: {e}"))?,
            AppType::Gemini => {
                // Gemini takeover changes only .env; settings.json (including mcpServers) is left as is.
                let env_backup = if let Some(env) = effective_settings.get("env") {
                    json!({ "env": env })
                } else {
                    json!({ "env": {} })
                };
                serde_json::to_string(&env_backup)
                    .map_err(|e| format!("Could not serialize the Gemini config: {e}"))?
            }
            AppType::OpenCode | AppType::OpenClaw | AppType::Kimi => {
                return Err(format!("Unknown app type: {app_type}"));
            }
        };

        self.ensure_live_written_record(&app_type_enum).await;
        self.update_mirror_backup(&app_type_enum, &effective_settings, provider)
            .await;
        self.db
            .save_live_backup(app_type, &backup_json)
            .await
            .map_err(|e| format!("Could not update the {app_type} backup: {e}"))?;

        log::info!("Updated the {app_type} live backup (hot switch)");
        Ok(())
    }

    pub async fn hot_switch_provider(
        &self,
        app_type: &str,
        provider_id: &str,
    ) -> Result<HotSwitchOutcome, String> {
        let _guard = self.switch_locks.lock_for_app(app_type).await;

        let app_type_enum =
            AppType::from_str(app_type).map_err(|_| format!("Invalid app type: {app_type}"))?;
        let provider = self
            .db
            .get_provider_by_id(provider_id, app_type)
            .map_err(|e| format!("Could not read the provider: {e}"))?
            .ok_or_else(|| format!("No such provider: {provider_id}"))?;

        let previous_id = crate::settings::get_effective_current_provider(&self.db, &app_type_enum)
            .map_err(|e| format!("Could not read the current provider: {e}"))?;
        let logical_target_changed = previous_id.as_deref() != Some(provider_id);
        let previous =
            previous_id.and_then(|id| self.db.get_provider_by_id(&id, app_type).ok().flatten());

        let has_backup = self
            .db
            .get_live_backup(app_type_enum.as_str())
            .await
            .map_err(|e| format!("Could not read the {app_type} backup: {e}"))?
            .is_some();
        let live_taken_over = self.detect_takeover_in_live_config_for_app(&app_type_enum);
        let should_sync_backup = has_backup || live_taken_over;

        self.db
            .set_current_provider(app_type_enum.as_str(), provider_id)
            .map_err(|e| format!("Could not update the current provider: {e}"))?;
        crate::settings::set_current_provider(&app_type_enum, Some(provider_id))
            .map_err(|e| format!("Could not update the local current provider: {e}"))?;

        if should_sync_backup {
            self.update_live_backup_from_provider_inner(app_type, &provider)
                .await?;

            if matches!(app_type_enum, AppType::Claude) {
                self.sync_claude_live_from_provider_while_proxy_active(
                    &provider,
                    previous.as_ref(),
                )
                .await?;
                if let Err(e) = self.cleanup_claude_model_overrides_in_live() {
                    log::warn!("Failed to clear the Claude live model fields (the hot switch is unaffected): {e}");
                }
            }
        }

        if let Some(server) = self.server.read().await.as_ref() {
            server
                .set_active_target(app_type_enum.as_str(), &provider.id, &provider.name)
                .await;
        }

        Ok(HotSwitchOutcome {
            logical_target_changed,
        })
    }

    #[cfg(test)]
    async fn lock_switch_for_test(&self, app_type: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.switch_locks.lock_for_app(app_type).await
    }

    /// Switch provider in proxy mode (hot switch; does not write the live config)
    pub async fn switch_proxy_target(
        &self,
        app_type: &str,
        provider_id: &str,
    ) -> Result<(), String> {
        let outcome = self.hot_switch_provider(app_type, provider_id).await?;

        if outcome.logical_target_changed {
            log::info!("Proxy mode: switched the target provider of {app_type} to {provider_id}");
        } else {
            log::debug!("Proxy mode: {app_type} is already on target provider {provider_id}");
        }
        Ok(())
    }

    // ==================== Handing the live config back ====================

    /// What a restore writes to `app_type`'s live file: `target` (the backup,
    /// or the current provider's settings) merged onto the file as it is on
    /// disk, so only what the takeover changed is put back.
    async fn config_to_restore(&self, app_type: &AppType, mut target: Value) -> Value {
        if matches!(app_type, AppType::Codex) {
            // Codex keeps the login it has (the last account that answered,
            // or one it renewed or signed in itself).
            crate::proxy::codex_pool::keep_live_login(&self.db, &mut target);
        }
        self.merge_onto_live(app_type, target).await
    }

    /// `target` merged onto the live file as it is on disk: Switchy's changes
    /// since it last wrote the file are applied, anyone else's are kept. The
    /// merge base is the record of what the takeover wrote, or, without one,
    /// `target` with the keys the takeover manages as they are on disk.
    /// Claude's settings and Codex's `config.toml` are merged; anything else
    /// in `target` (Codex's login) is written as it is.
    async fn merge_onto_live(&self, app_type: &AppType, target: Value) -> Value {
        let Some(live) = self.read_live_for_merge(app_type) else {
            return target;
        };
        let base = match self.live_written(app_type).await {
            Some(written) => written,
            None => Self::takeover_base(app_type, &target, &live),
        };
        Self::merge_live(app_type, &base, target, &live)
    }

    /// Three-way merge of `target` onto `live` against `base`; see
    /// `merge_onto_live`.
    fn merge_live(app_type: &AppType, base: &Value, mut target: Value, live: &Value) -> Value {
        match app_type {
            AppType::Claude => live_merge::merge_json(Some(base), Some(&target), Some(live))
                .unwrap_or_else(|| json!({})),
            AppType::Codex => {
                let text = |v: &Value| v.get("config").and_then(Value::as_str).map(str::to_string);
                if let (Some(base), Some(ours), Some(theirs)) =
                    (text(base), text(&target), text(live))
                {
                    if let Some(merged) = live_merge::merge_toml(&base, &ours, &theirs) {
                        target["config"] = json!(merged);
                    }
                }
                target
            }
            _ => target,
        }
    }

    /// The part of `app_type`'s live file a restore merges into: Claude's
    /// `settings.json`, or Codex's `config.toml` as `{"config": text}`.
    fn read_live_for_merge(&self, app_type: &AppType) -> Option<Value> {
        match app_type {
            AppType::Claude => self.read_claude_live().ok(),
            AppType::Codex => std::fs::read_to_string(crate::codex_config::get_codex_config_path())
                .ok()
                .map(|text| json!({ "config": text })),
            _ => None,
        }
    }

    /// Stand-in for the record of what the takeover wrote: `target` with the
    /// keys the takeover manages as they are in `live`.
    fn takeover_base(app_type: &AppType, target: &Value, live: &Value) -> Value {
        match app_type {
            AppType::Codex => {
                fn text(v: &Value) -> &str {
                    v.get("config").and_then(Value::as_str).unwrap_or("")
                }
                json!({ "config": Self::codex_takeover_base(text(target), text(live)) })
            }
            _ => Self::claude_takeover_base(target, live),
        }
    }

    /// `target` with the Claude `env` keys the takeover writes or removes
    /// taken from `live`.
    fn claude_takeover_base(target: &Value, live: &Value) -> Value {
        let mut base = target.clone();
        let live_env = live.get("env").and_then(Value::as_object);
        if let Some(root) = base.as_object_mut() {
            let had_env = root.contains_key("env");
            if let Some(env) = root
                .entry("env")
                .or_insert_with(|| json!({}))
                .as_object_mut()
            {
                let keys = std::iter::once("ANTHROPIC_BASE_URL")
                    .chain(CLAUDE_TOKEN_ENV_KEYS)
                    .chain(CLAUDE_MODEL_OVERRIDE_ENV_KEYS);
                for key in keys {
                    match live_env.and_then(|e| e.get(key)) {
                        Some(value) => {
                            env.insert(key.to_string(), value.clone());
                        }
                        None => {
                            env.shift_remove(key);
                        }
                    }
                }
                if env.is_empty() && !had_env {
                    root.shift_remove("env");
                }
            }
        }
        base
    }

    /// `target` (a `config.toml`) with the keys the Codex takeover writes —
    /// `openai_base_url`, `chatgpt_base_url` and the `base_url` of the top level
    /// or of the active model provider — taken from `live`. Unparseable input
    /// returns `target`.
    fn codex_takeover_base(target: &str, live: &str) -> String {
        let (Ok(mut base), Ok(live)) = (
            target.parse::<toml_edit::DocumentMut>(),
            live.parse::<toml_edit::DocumentMut>(),
        ) else {
            return target.to_string();
        };
        for key in ["openai_base_url", "chatgpt_base_url", "base_url"] {
            match live.get(key) {
                Some(item) => base[key] = item.clone(),
                None => {
                    base.as_table_mut().remove(key);
                }
            }
        }
        let providers: Vec<String> = [&base, &live]
            .iter()
            .filter_map(|doc| doc.get("model_provider").and_then(|v| v.as_str()))
            .map(str::to_string)
            .collect();
        for name in providers {
            let live_url = live
                .get("model_providers")
                .and_then(|t| t.get(name.as_str()))
                .and_then(|t| t.get("base_url"))
                .cloned();
            match live_url {
                Some(item) => base["model_providers"][name.as_str()]["base_url"] = item,
                None => {
                    if let Some(table) = base
                        .get_mut("model_providers")
                        .and_then(|t| t.get_mut(name.as_str()))
                        .and_then(toml_edit::Item::as_table_like_mut)
                    {
                        table.remove("base_url");
                    }
                }
            }
        }
        base.to_string()
    }

    /// The record of what the takeover last wrote to `app_type`'s live file.
    async fn live_written(&self, app_type: &AppType) -> Option<Value> {
        self.written(app_type.as_str()).await
    }

    /// The record kept with the backup row `key`.
    async fn written(&self, key: &str) -> Option<Value> {
        let text = self.db.get_live_written(key).await.ok().flatten()?;
        serde_json::from_str(&text).ok()
    }

    /// Records `written` as what the takeover last put in `app_type`'s live
    /// file: Switchy's own content, without keys other tools had added that a
    /// merge kept. A failure is logged: a restore then changes only the keys
    /// the takeover manages.
    async fn record_live_written(&self, app_type: &AppType, written: &Value) {
        self.record_written(app_type.as_str(), written).await;
    }

    async fn record_written(&self, key: &str, written: &Value) {
        let result = match serde_json::to_string(written) {
            Ok(text) => self
                .db
                .record_live_written(key, &text)
                .await
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = result {
            log::warn!("Could not record what the {key} takeover wrote: {e}");
        }
    }

    /// Before a backup is replaced from a provider, makes sure what the
    /// takeover wrote is on record, taking it to be the outgoing backup with
    /// the takeover's keys as they are on disk when nothing was recorded.
    async fn ensure_live_written_record(&self, app_type: &AppType) {
        if !matches!(app_type, AppType::Claude | AppType::Codex)
            || self.live_written(app_type).await.is_some()
        {
            return;
        }
        let Ok(Some(backup)) = self.db.get_live_backup(app_type.as_str()).await else {
            return;
        };
        let Ok(backup) = serde_json::from_str::<Value>(&backup.original_config) else {
            return;
        };
        let Some(live) = self.read_live_for_merge(app_type) else {
            return;
        };
        let base = Self::takeover_base(app_type, &backup, &live);
        self.record_live_written(app_type, &base).await;
    }

    async fn write_claude_live_during_takeover(&self, config: &Value) -> Result<(), String> {
        self.write_claude_live(config)?;
        let written = crate::services::provider::sanitize_claude_settings_for_live(config);
        self.record_live_written(&AppType::Claude, &written).await;
        Ok(())
    }

    async fn write_codex_live_during_takeover(&self, config: &Value) -> Result<(), String> {
        self.write_codex_live(config)?;
        if let Some(text) = config.get("config").and_then(Value::as_str) {
            self.record_live_written(&AppType::Codex, &json!({ "config": text }))
                .await;
        }
        Ok(())
    }

    // ==================== The second install (WSL) ====================

    /// Backup-table key for the second install of `app_type` — the mirror
    /// directory a switch keeps in step, usually the WSL home.
    fn mirror_backup_key(app_type: &AppType) -> Option<&'static str> {
        match app_type {
            AppType::Claude => Some("claude_mirror"),
            AppType::Codex => Some("codex_mirror"),
            _ => None,
        }
    }

    /// The second install's directory, when one is configured and is not the
    /// install Switchy already manages.
    fn mirror_dir(app_type: &AppType) -> Option<std::path::PathBuf> {
        let (dir, own) = match app_type {
            AppType::Claude => (
                crate::settings::get_claude_mirror_override_dir()?,
                crate::config::get_claude_config_dir(),
            ),
            AppType::Codex => (
                crate::settings::get_codex_mirror_override_dir()?,
                crate::codex_config::get_codex_config_dir(),
            ),
            _ => return None,
        };
        (dir != own).then_some(dir)
    }

    /// The second install's live file: Claude's `settings.json` (or the older
    /// `claude.json`), Codex's `config.toml`. Only an existing file is used.
    fn mirror_live_path(app_type: &AppType) -> Option<std::path::PathBuf> {
        let dir = Self::mirror_dir(app_type)?;
        let names: &[&str] = match app_type {
            AppType::Claude => &["settings.json", "claude.json"],
            _ => &["config.toml"],
        };
        names.iter().map(|name| dir.join(name)).find(|p| p.exists())
    }

    /// Whether an install in `dir` reaches the proxy at this machine's
    /// loopback address. A WSL home does only with WSL's mirrored networking.
    fn mirror_reaches_proxy(dir: &std::path::Path) -> bool {
        let path = dir
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        if !(path.starts_with("\\\\wsl$") || path.starts_with("\\\\wsl.localhost")) {
            return true;
        }
        let Ok(text) = std::fs::read_to_string(crate::config::get_home_dir().join(".wslconfig"))
        else {
            return false;
        };
        text.lines().any(|line| {
            let line: String = line
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_lowercase();
            line == "networkingmode=mirrored"
        })
    }

    /// The part of the second install a takeover changes and a restore
    /// merges: Claude's settings, or Codex's `config.toml` as `{"config": text}`.
    fn read_mirror_live(app_type: &AppType, path: &std::path::Path) -> Result<Value, String> {
        match app_type {
            AppType::Claude => {
                let value: Value = read_json_file(path).map_err(|e| e.to_string())?;
                if value.is_object() {
                    Ok(value)
                } else {
                    Err(format!("{} is not a JSON object", path.display()))
                }
            }
            _ => std::fs::read_to_string(path)
                .map(|text| json!({ "config": text }))
                .map_err(|e| e.to_string()),
        }
    }

    fn write_mirror_live(
        app_type: &AppType,
        path: &std::path::Path,
        value: &Value,
    ) -> Result<(), String> {
        match app_type {
            AppType::Claude => write_json_file(path, value).map_err(|e| e.to_string()),
            _ => {
                let text = value.get("config").and_then(Value::as_str).unwrap_or("");
                crate::config::write_text_file(path, text).map_err(|e| e.to_string())
            }
        }
    }

    fn mirror_is_taken_over(app_type: &AppType) -> bool {
        let Some(path) = Self::mirror_live_path(app_type) else {
            return false;
        };
        match (app_type, Self::read_mirror_live(app_type, &path)) {
            (AppType::Claude, Ok(value)) => Self::is_claude_live_taken_over(&value),
            (AppType::Codex, Ok(value)) => value
                .get("config")
                .and_then(Value::as_str)
                .is_some_and(Self::codex_openai_base_url_is_local),
            _ => false,
        }
    }

    /// Points the second install (WSL) at the proxy the way the takeover
    /// points this machine's install at it, so open sessions there are served
    /// by the proxy too. The file as it was is backed up once per takeover.
    /// Codex is taken over only when it is signed in with ChatGPT on the
    /// built-in provider; an API-key install there is left alone. Failures
    /// are logged: this machine's takeover stands without the mirror.
    async fn take_over_mirror(&self, app_type: &AppType, proxy_url: &str, keep_login: bool) {
        let Some(key) = Self::mirror_backup_key(app_type) else {
            return;
        };
        let Some(path) = Self::mirror_live_path(app_type) else {
            return;
        };
        if !path.parent().is_some_and(Self::mirror_reaches_proxy) {
            log::warn!(
                "{} is not taken over: WSL reaches the proxy on this machine's loopback address only with networkingMode=mirrored in .wslconfig",
                path.display()
            );
            return;
        }
        if let Err(e) = self
            .take_over_mirror_inner(app_type, key, &path, proxy_url, keep_login)
            .await
        {
            log::warn!("Could not take over {}: {e}", path.display());
        }
    }

    async fn take_over_mirror_inner(
        &self,
        app_type: &AppType,
        key: &str,
        path: &std::path::Path,
        proxy_url: &str,
        keep_login: bool,
    ) -> Result<(), String> {
        let mut live = Self::read_mirror_live(app_type, path)?;
        let mut original = live.clone();
        match app_type {
            AppType::Claude => {
                Self::strip_claude_takeover(&mut original);
                Self::apply_claude_takeover_fields(&mut live, proxy_url, keep_login);
            }
            _ => {
                let text = live
                    .get("config")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let auth: Value = path
                    .parent()
                    .map(|dir| dir.join("auth.json"))
                    .and_then(|p| read_json_file(&p).ok())
                    .unwrap_or(Value::Null);
                if !crate::proxy::codex_pool::is_chatgpt_live_auth(&auth)
                    || Self::codex_config_names_model_provider(&text)
                {
                    log::info!(
                        "{} is not taken over: it is not signed in with ChatGPT on the built-in provider",
                        path.display()
                    );
                    return Ok(());
                }
                if Self::codex_openai_base_url_is_local(&text) {
                    original["config"] = json!(Self::set_codex_openai_base_url(&text, None));
                }
                let backend_url = format!(
                    "{}{}",
                    proxy_url.trim_end_matches('/'),
                    crate::proxy::codex_pool::BACKEND_PATH_PREFIX
                );
                let text = Self::set_codex_openai_base_url(&text, Some(&backend_url));
                let config_dir = path.parent().unwrap_or(path);
                live["config"] = json!(Self::set_codex_chatgpt_base_url(
                    &text,
                    crate::proxy::local_tls::Install::Wsl(config_dir)
                ));
            }
        }

        let has_backup = self
            .db
            .get_live_backup(key)
            .await
            .map_err(|e| e.to_string())?
            .is_some();
        if !has_backup {
            self.db
                .start_live_backup(key, &original.to_string())
                .await
                .map_err(|e| e.to_string())?;
        }
        Self::write_mirror_live(app_type, path, &live)?;
        self.record_written(key, &live).await;
        log::info!("{} taken over, proxy address: {proxy_url}", path.display());
        Ok(())
    }

    /// Hands the second install back under the same rule as this machine's:
    /// the backup, with any provider switch made during the takeover, merged
    /// onto the file as it is now. With no backup, a leftover proxy address is
    /// removed. Failures are logged.
    async fn restore_mirror(&self, app_type: &AppType) {
        let Some(key) = Self::mirror_backup_key(app_type) else {
            return;
        };
        let Some(path) = Self::mirror_live_path(app_type) else {
            return;
        };
        if let Err(e) = self.restore_mirror_inner(app_type, key, &path).await {
            log::warn!("Could not restore {}: {e}", path.display());
        }
    }

    async fn restore_mirror_inner(
        &self,
        app_type: &AppType,
        key: &str,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let live = Self::read_mirror_live(app_type, path)?;
        let backup = self
            .db
            .get_live_backup(key)
            .await
            .map_err(|e| e.to_string())?
            .map(|b| serde_json::from_str::<Value>(&b.original_config))
            .transpose()
            .map_err(|e| e.to_string())?;

        let Some(mut backup) = backup else {
            let mut cleaned = live.clone();
            match app_type {
                AppType::Claude => Self::strip_claude_takeover(&mut cleaned),
                _ => {
                    let text = live.get("config").and_then(Value::as_str).unwrap_or("");
                    if Self::codex_openai_base_url_is_local(text) {
                        cleaned["config"] = json!(Self::set_codex_openai_base_url(text, None));
                    }
                }
            }
            if cleaned != live {
                Self::write_mirror_live(app_type, path, &cleaned)?;
                log::info!("{} takeover leftovers cleared (no backup)", path.display());
            }
            return Ok(());
        };

        // A switch made during the takeover left the new account's login in
        // the backup; it goes to the mirror as a switch would have put it
        // there, unless the mirror holds a newer login of the same account.
        let login = match backup.as_object_mut() {
            Some(obj) if matches!(app_type, AppType::Codex) => obj.remove("auth"),
            _ => None,
        };

        let base = match self.written(key).await {
            Some(written) => written,
            None => Self::takeover_base(app_type, &backup, &live),
        };
        let merged = Self::merge_live(app_type, &base, backup, &live);
        Self::write_mirror_live(app_type, path, &merged)?;

        if let Some(login) = login.filter(Value::is_object) {
            let auth_path = path.with_file_name("auth.json");
            let current: Value = read_json_file(&auth_path).unwrap_or(Value::Null);
            // As on Windows, the mirror keeps a ChatGPT login it has; the
            // current provider's login fills in only when it has none.
            let keep_current =
                crate::services::codex_account::inspect(&current).is_some_and(|cur| cur.alive);
            if !keep_current {
                write_json_file(&auth_path, &login).map_err(|e| e.to_string())?;
            }
        }
        log::info!("{} restored from backup", path.display());
        Ok(())
    }

    /// Carries a provider switch made during the takeover into the second
    /// install's backup, the way the switch would have written it to the
    /// mirror with the proxy off.
    async fn update_mirror_backup(
        &self,
        app_type: &AppType,
        effective: &Value,
        provider: &Provider,
    ) {
        let Some(key) = Self::mirror_backup_key(app_type) else {
            return;
        };
        let Ok(Some(row)) = self.db.get_live_backup(key).await else {
            return;
        };
        let Ok(mut backup) = serde_json::from_str::<Value>(&row.original_config) else {
            return;
        };
        match app_type {
            AppType::Claude => {
                let settings =
                    crate::services::provider::sanitize_claude_settings_for_live(effective);
                backup = crate::services::provider::merge_claude_connection_into_target(
                    &backup, &settings,
                );
            }
            _ => {
                let current = backup.get("config").and_then(Value::as_str).unwrap_or("");
                let source = effective
                    .get("config")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let official = provider.category.as_deref() == Some("official");
                match crate::services::provider::codex_config_after_switch(
                    current, source, official,
                ) {
                    Ok(text) => backup["config"] = json!(text),
                    Err(e) => {
                        log::warn!("Could not update the {key} backup: {e}");
                        return;
                    }
                }
                if let Some(auth) = effective.get("auth") {
                    backup["auth"] = auth.clone();
                }
            }
        }
        if let Err(e) = self.db.save_live_backup(key, &backup.to_string()).await {
            log::warn!("Could not update the {key} backup: {e}");
        }
    }

    async fn delete_backups_for_app(&self, app_type: &AppType) {
        let _ = self.db.delete_live_backup(app_type.as_str()).await;
        if let Some(key) = Self::mirror_backup_key(app_type) {
            let _ = self.db.delete_live_backup(key).await;
        }
    }

    // ==================== Live config read/write helpers ====================

    /// Update base_url in a TOML string (delegates to the shared codex_config implementation)
    fn update_toml_base_url(toml_str: &str, new_url: &str) -> String {
        crate::codex_config::update_codex_toml_field(toml_str, "base_url", new_url)
            .unwrap_or_else(|_| toml_str.to_string())
    }

    fn read_claude_live(&self) -> Result<Value, String> {
        let path = get_claude_settings_path();
        if !path.exists() {
            return Err("The Claude config file does not exist".to_string());
        }

        let mut value: Value =
            read_json_file(&path).map_err(|e| format!("Could not read the Claude config: {e}"))?;

        if value.is_null() {
            value = json!({});
        }

        if !value.is_object() {
            let kind = match &value {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            return Err(format!(
                "The Claude config file is malformed: its root must be a JSON object (it is {kind}), at {}",
                path.display()
            ));
        }

        Ok(value)
    }

    fn write_claude_live(&self, config: &Value) -> Result<(), String> {
        let path = get_claude_settings_path();
        let settings = crate::services::provider::sanitize_claude_settings_for_live(config);
        write_json_file(&path, &settings)
            .map_err(|e| format!("Could not write the Claude config: {e}"))
    }

    fn read_codex_live(&self) -> Result<Value, String> {
        use crate::codex_config::{get_codex_auth_path, get_codex_config_path};

        let auth_path = get_codex_auth_path();
        if !auth_path.exists() {
            return Err("Codex auth.json does not exist".to_string());
        }

        let auth: Value = read_json_file(&auth_path)
            .map_err(|e| format!("Could not read the Codex auth file: {e}"))?;

        let config_path = get_codex_config_path();
        let config_str = if config_path.exists() {
            std::fs::read_to_string(&config_path)
                .map_err(|e| format!("Could not read the Codex config file: {e}"))?
        } else {
            String::new()
        };

        Ok(json!({
            "auth": auth,
            "config": config_str
        }))
    }

    fn write_codex_live(&self, config: &Value) -> Result<(), String> {
        use crate::codex_config::{
            get_codex_auth_path, get_codex_config_path, write_codex_live_atomic,
        };

        let auth = config.get("auth");
        let config_str = config.get("config").and_then(|v| v.as_str());

        match (auth, config_str) {
            (Some(auth), Some(cfg)) => write_codex_live_atomic(auth, Some(cfg))
                .map_err(|e| format!("Could not write the Codex config: {e}"))?,
            (Some(auth), None) => {
                let auth_path = get_codex_auth_path();
                write_json_file(&auth_path, auth)
                    .map_err(|e| format!("Could not write the Codex auth file: {e}"))?;
            }
            (None, Some(cfg)) => {
                let config_path = get_codex_config_path();
                crate::config::write_text_file(&config_path, cfg)
                    .map_err(|e| format!("Could not write the Codex config file: {e}"))?;
            }
            (None, None) => {}
        }

        Ok(())
    }

    fn read_gemini_live(&self) -> Result<Value, String> {
        use crate::gemini_config::{env_to_json, get_gemini_env_path, read_gemini_env};

        let env_path = get_gemini_env_path();
        if !env_path.exists() {
            return Err("The Gemini .env file does not exist".to_string());
        }

        let env_map =
            read_gemini_env().map_err(|e| format!("Could not read the Gemini env file: {e}"))?;
        Ok(env_to_json(&env_map))
    }

    fn write_gemini_live(&self, config: &Value) -> Result<(), String> {
        use crate::gemini_config::{json_to_env, write_gemini_env_atomic};

        let env_map =
            json_to_env(config).map_err(|e| format!("Could not convert the Gemini config: {e}"))?;
        write_gemini_env_atomic(&env_map)
            .map_err(|e| format!("Could not write the Gemini env file: {e}"))?;
        Ok(())
    }

    // ==================== Existing methods ====================

    /// Get the server status
    pub async fn get_status(&self) -> Result<ProxyStatus, String> {
        if let Some(server) = self.server.read().await.as_ref() {
            Ok(server.get_status().await)
        } else {
            // Return the default status when the server is not running
            Ok(ProxyStatus {
                running: false,
                ..Default::default()
            })
        }
    }

    /// Get the proxy config
    pub async fn get_config(&self) -> Result<ProxyConfig, String> {
        self.db
            .get_proxy_config()
            .await
            .map_err(|e| format!("Could not read the proxy config: {e}"))
    }

    /// Update the proxy config
    pub async fn update_config(&self, config: &ProxyConfig) -> Result<(), String> {
        // Keep the old config to decide whether a restart is needed
        let previous = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("Could not read the proxy config: {e}"))?;

        // Save to the database (live_takeover_active is left unchanged)
        let mut new_config = config.clone();
        new_config.live_takeover_active = previous.live_takeover_active;

        self.db
            .update_proxy_config(new_config.clone())
            .await
            .map_err(|e| format!("Could not save the proxy config: {e}"))?;

        // Check the server's current state
        let mut server_guard = self.server.write().await;
        if server_guard.is_none() {
            return Ok(());
        }

        // Decide whether a restart is needed (address or port changed)
        let require_restart = new_config.listen_address != previous.listen_address
            || new_config.listen_port != previous.listen_port;

        if require_restart {
            if let Some(server) = server_guard.take() {
                server.stop().await.map_err(|e| {
                    format!("Could not stop the proxy server before restarting it: {e}")
                })?;
            }

            let app_handle = self.app_handle.read().await.clone();
            let new_server = ProxyServer::new(new_config, self.db.clone(), app_handle);
            new_server
                .start()
                .await
                .map_err(|e| format!("Could not restart the proxy server: {e}"))?;

            *server_guard = Some(new_server);
            log::info!("Proxy config updated; the server restarted automatically to apply it");

            // If any app's live config is taken over, update the proxy address in it too (otherwise clients still point at the old port)
            drop(server_guard);
            if let Ok(takeover) = self.get_takeover_status().await {
                let mut updated_any = false;

                if takeover.claude {
                    self.takeover_live_config_best_effort(&AppType::Claude)
                        .await?;
                    updated_any = true;
                }
                if takeover.codex {
                    self.takeover_live_config_best_effort(&AppType::Codex)
                        .await?;
                    updated_any = true;
                }
                if takeover.gemini {
                    self.takeover_live_config_best_effort(&AppType::Gemini)
                        .await?;
                    updated_any = true;
                }

                if updated_any {
                    log::info!("Updated the proxy address in the live config");
                }
            }

            return Ok(());
        } else if let Some(server) = server_guard.as_ref() {
            server.apply_runtime_config(&new_config).await;
            log::info!("Proxy config applied live; no proxy server restart needed");
        }

        Ok(())
    }

    /// Check whether the server is running
    pub async fn is_running(&self) -> bool {
        self.server.read().await.is_some()
    }

    /// Hot-update the circuit breaker config
    ///
    /// If the proxy server is running, apply the new config to every circuit breaker already created
    pub async fn update_circuit_breaker_configs(
        &self,
        config: crate::proxy::CircuitBreakerConfig,
    ) -> Result<(), String> {
        if let Some(server) = self.server.read().await.as_ref() {
            server.update_circuit_breaker_configs(config).await;
            log::info!("Hot-updated the running circuit breaker config");
        } else {
            log::debug!("Proxy server is not running; the circuit breaker config takes effect on the next start");
        }
        Ok(())
    }

    /// Reset a provider's circuit breaker
    ///
    /// If the proxy server is running, reset the in-memory circuit breaker immediately
    pub async fn reset_provider_circuit_breaker(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Result<(), String> {
        if let Some(server) = self.server.read().await.as_ref() {
            server
                .reset_provider_circuit_breaker(provider_id, app_type)
                .await;
            log::info!("Reset the circuit breaker of provider {provider_id} (app: {app_type})");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderMeta;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("SWITCHY_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("SWITCHY_TEST_HOME", dir.path());

            Self {
                dir,
                original_home,
                original_userprofile,
                original_test_home,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }

            match &self.original_test_home {
                Some(value) => env::set_var("SWITCHY_TEST_HOME", value),
                None => env::remove_var("SWITCHY_TEST_HOME"),
            }
        }
    }

    fn chatgpt_live(config: &str) -> Value {
        json!({
            "auth": {
                "tokens": {
                    "id_token": "x.e30.y",
                    "access_token": "AAA",
                    "refresh_token": "RRR",
                    "account_id": "acct-a"
                },
                "last_refresh": "2026-09-01T00:00:00Z"
            },
            "config": config
        })
    }

    #[test]
    fn claude_takeover_that_keeps_the_login_writes_no_token_placeholder() {
        let mut live = json!({ "env": { "ANTHROPIC_AUTH_TOKEN": "stale", "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000" } });
        ProxyService::apply_claude_takeover_fields(&mut live, "http://127.0.0.1:15721", true);
        let env = live["env"].as_object().unwrap();
        assert_eq!(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:15721");
        assert!(
            env.get("ANTHROPIC_AUTH_TOKEN").is_none(),
            "a token key would take Claude Code out of subscription mode"
        );
        assert!(env.get("ANTHROPIC_API_KEY").is_none());
        assert_eq!(env["CLAUDE_CODE_MAX_OUTPUT_TOKENS"], "64000");
        assert!(
            ProxyService::is_claude_live_taken_over(&live),
            "the local base URL marks the takeover"
        );
    }

    #[test]
    fn claude_takeover_for_an_api_key_provider_still_writes_the_placeholder() {
        let mut live = json!({ "env": { "ANTHROPIC_AUTH_TOKEN": "sk-real" } });
        ProxyService::apply_claude_takeover_fields(&mut live, "http://127.0.0.1:15721", false);
        assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], PROXY_TOKEN_PLACEHOLDER);
        let mut bare = json!({ "env": {} });
        ProxyService::apply_claude_takeover_fields(&mut bare, "http://127.0.0.1:15721", false);
        assert_eq!(bare["env"]["ANTHROPIC_AUTH_TOKEN"], PROXY_TOKEN_PLACEHOLDER);
    }

    #[test]
    fn codex_takeover_keeps_a_chatgpt_login_and_redirects_the_builtin_provider() {
        let mut live = chatgpt_live("model = \"gpt-5\"\n");
        ProxyService::apply_codex_takeover_fields(
            &mut live,
            "http://127.0.0.1:15721",
            "http://127.0.0.1:15721/v1",
        );

        assert!(
            live["auth"].get("OPENAI_API_KEY").is_none(),
            "an API key would switch Codex out of ChatGPT mode"
        );
        assert_eq!(live["auth"]["tokens"]["refresh_token"], "RRR");
        let parsed: toml::Value = toml::from_str(live["config"].as_str().unwrap()).unwrap();
        assert_eq!(
            parsed.get("openai_base_url").and_then(|v| v.as_str()),
            Some("http://127.0.0.1:15721/backend-api/codex")
        );
        assert_eq!(parsed.get("model").and_then(|v| v.as_str()), Some("gpt-5"));
        assert!(ProxyService::is_codex_live_taken_over(&live));
    }

    #[test]
    fn codex_takeover_of_an_api_key_provider_is_unchanged() {
        let mut live = json!({
            "auth": { "OPENAI_API_KEY": "sk-real" },
            "config": "model_provider = \"any\"\n[model_providers.any]\nbase_url = \"https://x/v1\"\n"
        });
        ProxyService::apply_codex_takeover_fields(
            &mut live,
            "http://127.0.0.1:15721",
            "http://127.0.0.1:15721/v1",
        );

        assert_eq!(live["auth"]["OPENAI_API_KEY"], PROXY_TOKEN_PLACEHOLDER);
        let parsed: toml::Value = toml::from_str(live["config"].as_str().unwrap()).unwrap();
        assert!(parsed.get("openai_base_url").is_none());
        assert_eq!(
            parsed["model_providers"]["any"]["base_url"].as_str(),
            Some("http://127.0.0.1:15721/v1")
        );
    }

    #[test]
    fn codex_chatgpt_login_with_a_third_party_provider_gets_the_api_key_takeover() {
        let mut live = chatgpt_live(
            "model_provider = \"any\"\n[model_providers.any]\nbase_url = \"https://x/v1\"\n",
        );
        ProxyService::apply_codex_takeover_fields(
            &mut live,
            "http://127.0.0.1:15721",
            "http://127.0.0.1:15721/v1",
        );
        assert_eq!(live["auth"]["OPENAI_API_KEY"], PROXY_TOKEN_PLACEHOLDER);
    }

    #[test]
    fn codex_openai_base_url_is_removed_only_when_it_points_at_the_proxy() {
        let local = ProxyService::set_codex_openai_base_url(
            "model = \"gpt-5\"\n",
            Some("http://127.0.0.1:15721/backend-api/codex"),
        );
        assert!(ProxyService::codex_openai_base_url_is_local(&local));
        let local = format!(
            "{local}chatgpt_base_url = \"https://127.0.0.1:15722/backend-api\"
"
        );
        let cleaned = ProxyService::set_codex_openai_base_url(&local, None);
        assert!(!cleaned.contains("openai_base_url"));
        assert!(!cleaned.contains("chatgpt_base_url"));
        assert!(cleaned.contains("model = \"gpt-5\""));

        let users_own = "openai_base_url = \"https://gateway.example/backend-api/codex\"\n";
        assert!(!ProxyService::codex_openai_base_url_is_local(users_own));
        assert!(!ProxyService::is_codex_live_taken_over(&chatgpt_live(
            users_own
        )));
    }

    #[test]
    fn update_toml_base_url_updates_active_model_provider_base_url() {
        let input = r#"
model_provider = "any"
model = "gpt-5.1-codex"
disable_response_storage = true

[model_providers.any]
name = "any"
base_url = "https://anyrouter.top/v1"
wire_api = "responses"
requires_openai_auth = true
"#;

        let new_url = "http://127.0.0.1:5000/v1";
        let output = ProxyService::update_toml_base_url(input, new_url);

        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        let base_url = parsed
            .get("model_providers")
            .and_then(|v| v.get("any"))
            .and_then(|v| v.get("base_url"))
            .and_then(|v| v.as_str())
            .expect("model_providers.any.base_url should exist");

        assert_eq!(base_url, new_url);
        assert!(
            parsed.get("base_url").is_none(),
            "should not write top-level base_url"
        );

        let wire_api = parsed
            .get("model_providers")
            .and_then(|v| v.get("any"))
            .and_then(|v| v.get("wire_api"))
            .and_then(|v| v.as_str())
            .expect("model_providers.any.wire_api should exist");
        assert_eq!(wire_api, "responses");
    }

    #[test]
    fn update_toml_base_url_falls_back_to_top_level_base_url() {
        let input = r#"
model = "gpt-5.1-codex"
"#;

        let new_url = "http://127.0.0.1:5000/v1";
        let output = ProxyService::update_toml_base_url(input, new_url);

        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        let base_url = parsed
            .get("base_url")
            .and_then(|v| v.as_str())
            .expect("base_url should exist");

        assert_eq!(base_url, new_url);
    }

    #[tokio::test]
    #[serial]
    async fn sync_claude_token_does_not_add_anthropic_api_key() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
                    "ANTHROPIC_AUTH_TOKEN": "stale"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");

        let live_config = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "fresh"
            }
        });

        service
            .sync_live_config_to_provider(&AppType::Claude, &live_config)
            .await
            .expect("sync");

        let updated = db
            .get_provider_by_id("p1", "claude")
            .expect("get provider")
            .expect("provider exists");
        let env = updated
            .settings_config
            .get("env")
            .and_then(|v| v.as_object())
            .expect("env object");

        assert_eq!(
            env.get("ANTHROPIC_AUTH_TOKEN").and_then(|v| v.as_str()),
            Some("fresh")
        );
        assert!(
            !env.contains_key("ANTHROPIC_API_KEY"),
            "should not add ANTHROPIC_API_KEY when absent"
        );
    }

    #[tokio::test]
    #[serial]
    async fn sync_claude_token_respects_existing_api_key_field() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
                    "ANTHROPIC_API_KEY": "stale"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");

        let live_config = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "fresh"
            }
        });

        service
            .sync_live_config_to_provider(&AppType::Claude, &live_config)
            .await
            .expect("sync");

        let updated = db
            .get_provider_by_id("p1", "claude")
            .expect("get provider")
            .expect("provider exists");
        let env = updated
            .settings_config
            .get("env")
            .and_then(|v| v.as_object())
            .expect("env object");

        assert_eq!(
            env.get("ANTHROPIC_API_KEY").and_then(|v| v.as_str()),
            Some("fresh")
        );
        assert!(
            !env.contains_key("ANTHROPIC_AUTH_TOKEN"),
            "should not add ANTHROPIC_AUTH_TOKEN when absent"
        );
    }

    #[tokio::test]
    #[serial]
    async fn switch_proxy_target_updates_live_backup_when_taken_over() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "a-key"
                }
            }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "b-key"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.set_current_provider("claude", "a")
            .expect("set current provider");

        // Simulate the "taken over" state: a live backup exists (its content does not matter; the hot switch updates it)
        db.save_live_backup("claude", "{\"env\":{}}")
            .await
            .expect("seed live backup");

        service
            .switch_proxy_target("claude", "b")
            .await
            .expect("switch proxy target");

        // Assert: the current provider in local settings is synced
        assert_eq!(
            crate::settings::get_current_provider(&AppType::Claude).as_deref(),
            Some("b")
        );

        // Assert: the live backup now holds the target provider's config (used by stop_with_restore)
        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let expected = serde_json::to_string(&provider_b.settings_config).expect("serialize");
        assert_eq!(backup.original_config, expected);
    }

    #[tokio::test]
    #[serial]
    async fn hot_switch_provider_updates_claude_live_while_preserving_takeover_fields() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "a-key",
                    "ANTHROPIC_BASE_URL": "https://api.a.example",
                    "ANTHROPIC_MODEL": "claude-old"
                },
                "permissions": { "allow": ["Bash"] }
            }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "b-key",
                    "ANTHROPIC_BASE_URL": "https://api.b.example",
                    "ANTHROPIC_MODEL": "claude-new"
                },
                "permissions": { "allow": ["Read"] }
            }),
            None,
        );

        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.set_current_provider("claude", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current provider");
        db.save_live_backup(
            "claude",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize provider a"),
        )
        .await
        .expect("seed live backup");
        service
            .write_claude_live(&json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721",
                    "ANTHROPIC_API_KEY": PROXY_TOKEN_PLACEHOLDER,
                    "ANTHROPIC_MODEL": "stale-model"
                },
                "permissions": { "allow": ["Bash"] }
            }))
            .expect("seed taken-over live file");

        service
            .hot_switch_provider("claude", "b")
            .await
            .expect("hot switch provider");

        let live = service.read_claude_live().expect("read live config");
        assert_eq!(
            live.get("permissions"),
            Some(&json!({ "allow": ["Bash"] })),
            "a switch writes only the provider's connection keys"
        );
        assert_eq!(
            live.get("env")
                .and_then(|env| env.get("ANTHROPIC_API_KEY"))
                .and_then(|v| v.as_str()),
            Some(PROXY_TOKEN_PLACEHOLDER),
            "takeover token placeholder should be preserved"
        );
        assert_eq!(
            live.get("env")
                .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
                .and_then(|v| v.as_str()),
            Some("http://127.0.0.1:15721"),
            "takeover proxy URL should remain active"
        );
        assert!(
            live.get("env")
                .and_then(|env| env.get("ANTHROPIC_MODEL"))
                .is_none(),
            "Claude model override fields should be removed in takeover mode"
        );

        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let expected = serde_json::to_string(&provider_b.settings_config).expect("serialize");
        assert_eq!(backup.original_config, expected);
    }

    #[tokio::test]
    #[serial]
    async fn hot_switch_provider_serializes_same_app_switches() {
        use tokio::time::{sleep, Duration};

        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "a-key" } }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "b-key" } }),
            None,
        );
        let provider_c = Provider::with_id(
            "c".to_string(),
            "C".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "c-key" } }),
            None,
        );

        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.save_provider("claude", &provider_c)
            .expect("save provider c");
        db.set_current_provider("claude", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current provider");
        db.save_live_backup("claude", "{\"env\":{}}")
            .await
            .expect("seed live backup");

        let guard = service.lock_switch_for_test("claude").await;
        let service_for_b = service.clone();
        let service_for_c = service.clone();

        let switch_b = tokio::spawn(async move {
            service_for_b
                .hot_switch_provider("claude", "b")
                .await
                .expect("switch to b")
        });
        sleep(Duration::from_millis(20)).await;
        let switch_c = tokio::spawn(async move {
            service_for_c
                .hot_switch_provider("claude", "c")
                .await
                .expect("switch to c")
        });

        sleep(Duration::from_millis(20)).await;
        drop(guard);

        let outcome_b = switch_b.await.expect("join switch b");
        let outcome_c = switch_c.await.expect("join switch c");
        assert!(outcome_b.logical_target_changed);
        assert!(outcome_c.logical_target_changed);

        assert_eq!(
            crate::settings::get_effective_current_provider(&db, &AppType::Claude)
                .expect("effective current"),
            Some("c".to_string())
        );
        assert_eq!(
            crate::settings::get_current_provider(&AppType::Claude).as_deref(),
            Some("c")
        );
        assert_eq!(
            db.get_current_provider("claude").expect("db current"),
            Some("c".to_string())
        );

        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let expected = serde_json::to_string(&provider_c.settings_config).expect("serialize");
        assert_eq!(backup.original_config, expected);
    }

    #[tokio::test]
    #[serial]
    async fn restore_waits_for_hot_switch_and_restores_latest_backup() {
        use tokio::time::{sleep, Duration};

        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "a-key" } }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "b-key" } }),
            None,
        );

        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.set_current_provider("claude", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current provider");
        db.save_live_backup(
            "claude",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize provider a"),
        )
        .await
        .expect("seed live backup");
        service
            .write_claude_live(&json!({ "env": { "ANTHROPIC_API_KEY": "stale" } }))
            .expect("seed live file");

        let guard = service.lock_switch_for_test("claude").await;
        let service_for_switch = service.clone();
        let service_for_restore = service.clone();

        let switch_to_b = tokio::spawn(async move {
            service_for_switch
                .hot_switch_provider("claude", "b")
                .await
                .expect("switch to b")
        });
        sleep(Duration::from_millis(20)).await;
        let restore = tokio::spawn(async move {
            service_for_restore
                .restore_live_config_for_app_with_fallback(&AppType::Claude)
                .await
                .expect("restore claude live")
        });

        sleep(Duration::from_millis(20)).await;
        drop(guard);

        let outcome = switch_to_b.await.expect("join switch");
        restore.await.expect("join restore");
        assert!(outcome.logical_target_changed);

        assert_eq!(
            crate::settings::get_effective_current_provider(&db, &AppType::Claude)
                .expect("effective current"),
            Some("b".to_string())
        );

        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let expected = serde_json::to_string(&provider_b.settings_config).expect("serialize");
        assert_eq!(backup.original_config, expected);
        assert_eq!(
            service.read_claude_live().expect("read live"),
            provider_b.settings_config
        );
    }

    #[tokio::test]
    #[serial]
    async fn update_live_backup_from_provider_applies_claude_common_config() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        db.set_config_snippet(
            "claude",
            Some(
                serde_json::json!({
                    "includeCoAuthoredBy": false
                })
                .to_string(),
            ),
        )
        .expect("set common config snippet");

        let service = ProxyService::new(db.clone());

        let mut provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token",
                    "ANTHROPIC_BASE_URL": "https://claude.example"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            common_config_enabled: Some(true),
            ..Default::default()
        });

        service
            .update_live_backup_from_provider("claude", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");

        assert_eq!(
            stored.get("includeCoAuthoredBy").and_then(|v| v.as_bool()),
            Some(false),
            "common config should be applied into Claude restore backup"
        );
    }

    #[tokio::test]
    #[serial]
    async fn update_live_backup_from_provider_applies_codex_common_config() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        db.set_config_snippet(
            "codex",
            Some("disable_response_storage = true\n".to_string()),
        )
        .expect("set common config snippet");

        let service = ProxyService::new(db.clone());

        let mut provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "token"
                },
                "config": r#"model_provider = "any"
model = "gpt-5"

[model_providers.any]
base_url = "https://codex.example/v1"
"#
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            common_config_enabled: Some(true),
            ..Default::default()
        });

        service
            .update_live_backup_from_provider("codex", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");
        let config = stored
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config string");

        assert!(
            config.contains("disable_response_storage = true"),
            "common config should be applied into Codex restore backup"
        );
    }

    #[tokio::test]
    #[serial]
    async fn update_live_backup_from_provider_preserves_codex_mcp_servers() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        db.save_live_backup(
            "codex",
            &serde_json::to_string(&json!({
                "auth": {
                    "OPENAI_API_KEY": "old-token"
                },
                "config": r#"model_provider = "any"
model = "gpt-4"

[model_providers.any]
base_url = "https://old.example/v1"

[mcp_servers.echo]
command = "npx"
args = ["echo-server"]
"#
            }))
            .expect("serialize seed backup"),
        )
        .await
        .expect("seed live backup");

        let provider = Provider::with_id(
            "p2".to_string(),
            "P2".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "new-token"
                },
                "config": r#"model_provider = "any"
model = "gpt-5"

[model_providers.any]
base_url = "https://new.example/v1"
"#
            }),
            None,
        );

        service
            .update_live_backup_from_provider("codex", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");
        let config = stored
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config string");

        assert!(
            config.contains("[mcp_servers.echo]"),
            "existing Codex MCP section should survive proxy hot-switch backup update"
        );
        assert!(
            config.contains("https://new.example/v1"),
            "provider-specific base_url should still update to the new provider"
        );
    }

    #[tokio::test]
    #[serial]
    async fn a_switch_leaves_the_mcp_servers_in_the_codex_backup_as_they_were() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        db.save_live_backup(
            "codex",
            &serde_json::to_string(&json!({
                "auth": {
                    "OPENAI_API_KEY": "old-token"
                },
                "config": r#"[mcp_servers.shared]
command = "old-command"

[mcp_servers.legacy]
command = "legacy-command"
"#
            }))
            .expect("serialize seed backup"),
        )
        .await
        .expect("seed live backup");

        let provider = Provider::with_id(
            "p2".to_string(),
            "P2".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "new-token"
                },
                "config": r#"[mcp_servers.shared]
command = "new-command"

[mcp_servers.latest]
command = "latest-command"
"#
            }),
            None,
        );

        service
            .update_live_backup_from_provider("codex", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");
        let config = stored
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config string");
        let parsed: toml::Value = toml::from_str(config).expect("parse merged codex config");

        let mcp_servers = parsed
            .get("mcp_servers")
            .expect("mcp_servers should be present");
        assert_eq!(
            mcp_servers
                .get("shared")
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str()),
            Some("old-command"),
            "the MCP servers are the user's; a switch does not write the card's"
        );
        assert!(mcp_servers.get("legacy").is_some());
        assert!(mcp_servers.get("latest").is_none());
    }

    /// Orca's status hooks, written into `settings.json` by another tool.
    fn orca_hooks() -> Value {
        json!({
            "Stop": [{
                "hooks": [{ "type": "command", "command": "%APPDATA%\\orca\\agent-hooks\\endpoint.cmd Stop" }]
            }],
            "SessionStart": [{
                "hooks": [{ "type": "command", "command": "%APPDATA%\\orca\\agent-hooks\\endpoint.cmd SessionStart" }]
            }]
        })
    }

    /// Adds Orca's hooks to the live `settings.json` the way Orca does: read
    /// the file, add its block, write it back.
    fn add_orca_hooks_to_live() {
        let path = get_claude_settings_path();
        let mut live: Value = read_json_file(&path).expect("read live settings");
        live["hooks"] = orca_hooks();
        write_json_file(&path, &live).expect("write live settings");
    }

    async fn take_over_claude(service: &ProxyService) {
        service
            .backup_live_config_strict(&AppType::Claude)
            .await
            .expect("back up Claude live");
        service
            .takeover_live_config_strict(&AppType::Claude)
            .await
            .expect("take over Claude live");
        assert!(ProxyService::is_claude_live_taken_over(
            &service.read_claude_live().expect("read live")
        ));
    }

    #[tokio::test]
    #[serial]
    async fn hooks_added_after_takeover_survive_a_clean_restore() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let original = json!({
            "env": { "ANTHROPIC_AUTH_TOKEN": "sk-real", "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000" },
            "model": "opus"
        });
        service.write_claude_live(&original).expect("seed live");

        take_over_claude(&service).await;
        add_orca_hooks_to_live();

        service
            .stop_with_restore()
            .await
            .expect("stop with restore");

        let mut expected = original.clone();
        expected["hooks"] = orca_hooks();
        assert_eq!(service.read_claude_live().expect("read live"), expected);
        assert!(db
            .get_live_backup("claude")
            .await
            .expect("read backup")
            .is_none());
    }

    #[tokio::test]
    #[serial]
    async fn hooks_added_after_takeover_survive_unclean_exit_recovery() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));

        // The proxy URL the user had before the takeover must come back.
        let original = json!({
            "env": { "ANTHROPIC_BASE_URL": "https://relay.example", "ANTHROPIC_AUTH_TOKEN": "sk-relay" }
        });
        {
            let service = ProxyService::new(db.clone());
            service.write_claude_live(&original).expect("seed live");
            take_over_claude(&service).await;
            // Killed here: no restore runs.
        }
        add_orca_hooks_to_live();

        let restarted = ProxyService::new(db.clone());
        assert!(restarted.detect_takeover_in_live_configs());
        restarted.recover_from_crash().await.expect("recover");

        let mut expected = original.clone();
        expected["hooks"] = orca_hooks();
        assert_eq!(restarted.read_claude_live().expect("read live"), expected);
        assert!(!db.has_any_live_backup().await.expect("read backups"));
    }

    #[tokio::test]
    #[serial]
    async fn recovery_from_a_backup_with_no_record_keeps_hooks_added_since() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // A backup taken before Orca's hooks were put back, by a build that
        // kept no record of what the takeover wrote.
        let backup =
            json!({ "env": { "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000" }, "model": "opus" });
        db.save_live_backup("claude", &backup.to_string())
            .await
            .expect("seed backup");
        let mut live = json!({
            "env": {
                "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000",
                "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721"
            },
            "model": "sonnet"
        });
        live["hooks"] = orca_hooks();
        service.write_claude_live(&live).expect("seed live");

        service.recover_from_crash().await.expect("recover");

        let restored = service.read_claude_live().expect("read live");
        assert_eq!(restored["hooks"], orca_hooks());
        assert_eq!(
            restored["model"], "sonnet",
            "a setting changed since the backup stays"
        );
        assert_eq!(
            restored["env"],
            json!({ "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000" })
        );
    }

    #[tokio::test]
    #[serial]
    async fn restore_without_a_backup_rebuilds_only_the_takeover_keys() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p".to_string(),
            "P".to_string(),
            json!({ "env": { "ANTHROPIC_AUTH_TOKEN": "sk-provider" } }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p").expect("set current");
        crate::settings::set_current_provider(&AppType::Claude, Some("p"))
            .expect("set local current");

        let mut live = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721",
                "ANTHROPIC_AUTH_TOKEN": PROXY_TOKEN_PLACEHOLDER
            },
            "model": "opus"
        });
        live["hooks"] = orca_hooks();
        service.write_claude_live(&live).expect("seed live");

        service.recover_from_crash().await.expect("recover");

        let mut expected = json!({
            "env": { "ANTHROPIC_AUTH_TOKEN": "sk-provider" },
            "model": "opus"
        });
        expected["hooks"] = orca_hooks();
        assert_eq!(service.read_claude_live().expect("read live"), expected);
    }

    #[tokio::test]
    #[serial]
    async fn hot_switch_and_restore_keep_hooks_added_during_the_takeover() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "a-key" }, "permissions": { "allow": ["Bash"] } }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "b-key" }, "permissions": { "allow": ["Read"] } }),
            None,
        );
        db.save_provider("claude", &provider_a).expect("save a");
        db.save_provider("claude", &provider_b).expect("save b");
        db.set_current_provider("claude", "a").expect("set current");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current");
        service
            .write_claude_live(&provider_a.settings_config)
            .expect("seed live");

        take_over_claude(&service).await;
        add_orca_hooks_to_live();

        service
            .hot_switch_provider("claude", "b")
            .await
            .expect("hot switch");
        let live = service.read_claude_live().expect("read live");
        assert_eq!(live["hooks"], orca_hooks(), "a hot switch keeps the hooks");
        assert_eq!(
            live["permissions"],
            json!({ "allow": ["Bash"] }),
            "the permissions are the user's"
        );
        assert!(ProxyService::is_claude_live_taken_over(&live));

        service
            .stop_with_restore()
            .await
            .expect("stop with restore");

        assert_eq!(
            service.read_claude_live().expect("read live"),
            json!({
                "env": { "ANTHROPIC_API_KEY": "b-key" },
                "permissions": { "allow": ["Bash"] },
                "hooks": orca_hooks(),
            })
        );
    }

    #[tokio::test]
    #[serial]
    async fn switching_away_from_an_account_that_stored_the_hooks_keeps_them() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // Two Official accounts. The outgoing one's stored settings are an old
        // copy of the whole file, hooks and model included; the incoming one
        // stored neither.
        let official = |id: &str, settings: Value| {
            let mut provider = Provider::with_id(id.to_string(), id.to_uppercase(), settings, None);
            provider.category = Some("official".to_string());
            provider.meta = Some(crate::provider::ProviderMeta {
                captured_claude_account: Some(crate::provider::CapturedClaudeAccountMeta {
                    account_uuid: format!("uuid-{id}"),
                    email_address: format!("{id}@example.com"),
                    captured_at: 1,
                }),
                ..Default::default()
            });
            provider
        };
        let provider_a = official(
            "a",
            json!({
                "model": "claude-fable-5-1[1m]",
                "permissions": { "allow": ["Bash"] },
                "hooks": orca_hooks(),
            }),
        );
        let provider_b = official("b", json!({ "env": {} }));
        db.save_provider("claude", &provider_a).expect("save a");
        db.save_provider("claude", &provider_b).expect("save b");
        db.set_current_provider("claude", "a").expect("set current");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current");
        service
            .write_claude_live(&provider_a.settings_config)
            .expect("seed live");
        take_over_claude(&service).await;

        service
            .hot_switch_provider("claude", "b")
            .await
            .expect("hot switch");

        let live = service.read_claude_live().expect("read live");
        assert_eq!(live["hooks"], orca_hooks());
        assert_eq!(live["model"], json!("claude-fable-5-1[1m]"));
        assert_eq!(live["permissions"], json!({ "allow": ["Bash"] }));
        assert!(ProxyService::is_claude_live_taken_over(&live));
    }

    #[tokio::test]
    #[serial]
    async fn codex_restore_keeps_config_toml_edits_made_during_the_takeover() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let original = chatgpt_live("# mine\nmodel = \"gpt-5\"\n");
        service.write_codex_live(&original).expect("seed live");
        service
            .backup_live_config_strict(&AppType::Codex)
            .await
            .expect("back up Codex live");
        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over Codex live");
        let config_path = crate::codex_config::get_codex_config_path();
        let taken_over = std::fs::read_to_string(&config_path).expect("read config");
        assert!(ProxyService::codex_openai_base_url_is_local(&taken_over));

        std::fs::write(
            &config_path,
            format!("{taken_over}\n[mcp_servers.orca]\ncommand = \"orca\"\n"),
        )
        .expect("add a table");

        service
            .stop_with_restore()
            .await
            .expect("stop with restore");

        let restored = std::fs::read_to_string(&config_path).expect("read config");
        let table: toml::Table = restored.parse().expect("valid toml");
        assert!(table.get("openai_base_url").is_none());
        assert_eq!(table["model"].as_str(), Some("gpt-5"));
        assert_eq!(
            table["mcp_servers"]["orca"]["command"].as_str(),
            Some("orca")
        );
        assert!(restored.starts_with("# mine"));
    }

    #[tokio::test]
    #[serial]
    async fn codex_restore_after_a_hot_switch_writes_the_new_providers_config() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let relay = |name: &str, key: &str| {
            json!({
                "auth": { "OPENAI_API_KEY": key },
                "config": format!(
                    "model_provider = \"{name}\"\nmodel = \"{name}-model\"\n\n[model_providers.{name}]\nbase_url = \"https://{name}.example/v1\"\n"
                )
            })
        };
        let provider_a = Provider::with_id("a".into(), "A".into(), relay("a", "a-key"), None);
        let provider_b = Provider::with_id("b".into(), "B".into(), relay("b", "b-key"), None);
        db.save_provider("codex", &provider_a).expect("save a");
        db.save_provider("codex", &provider_b).expect("save b");
        db.set_current_provider("codex", "a").expect("set current");
        crate::settings::set_current_provider(&AppType::Codex, Some("a"))
            .expect("set local current");
        service
            .write_codex_live(&provider_a.settings_config)
            .expect("seed live");

        service
            .backup_live_config_strict(&AppType::Codex)
            .await
            .expect("back up Codex live");
        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over Codex live");
        let config_path = crate::codex_config::get_codex_config_path();
        let taken_over = std::fs::read_to_string(&config_path).expect("read config");
        std::fs::write(
            &config_path,
            format!("{taken_over}\n[profiles.fast]\nmodel = \"x\"\n"),
        )
        .expect("add a table");

        service
            .hot_switch_provider("codex", "b")
            .await
            .expect("hot switch");
        service
            .stop_with_restore()
            .await
            .expect("stop with restore");

        let restored = std::fs::read_to_string(&config_path).expect("read config");
        let table: toml::Table = restored.parse().expect("valid toml");
        assert_eq!(table["model_provider"].as_str(), Some("b"));
        assert_eq!(table["model"].as_str(), Some("b-model"));
        assert_eq!(
            table["model_providers"]["b"]["base_url"].as_str(),
            Some("https://b.example/v1")
        );
        assert!(table["model_providers"].get("a").is_none());
        assert_eq!(table["profiles"]["fast"]["model"].as_str(), Some("x"));
        let auth: Value =
            read_json_file(&crate::codex_config::get_codex_auth_path()).expect("read auth");
        assert_eq!(auth["OPENAI_API_KEY"], "b-key");
    }

    /// A ChatGPT login of `account`, refreshed at `last_refresh`.
    fn chatgpt_login(account: &str, last_refresh: &str) -> Value {
        json!({
            "tokens": {
                "id_token": "x.e30.y",
                "access_token": format!("access-{account}"),
                "refresh_token": format!("refresh-{account}"),
                "account_id": account
            },
            "last_refresh": last_refresh
        })
    }

    fn set_codex_mirror_dir(dir: &std::path::Path) {
        let mut settings = crate::settings::get_settings();
        settings.codex_mirror_config_dir = Some(dir.to_string_lossy().to_string());
        crate::settings::update_settings(settings).expect("set codex mirror dir");
    }

    #[tokio::test]
    #[serial]
    async fn the_wsl_claude_install_is_routed_through_the_proxy_and_handed_back() {
        let home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        service
            .write_claude_live(&json!({ "env": { "ANTHROPIC_AUTH_TOKEN": "sk-real" } }))
            .expect("seed live");
        let mirror = home.dir.path().join("wsl").join(".claude");
        std::fs::create_dir_all(&mirror).expect("mirror dir");
        let mirror_settings = mirror.join("settings.json");
        let original = json!({
            "env": { "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000" },
            "model": "opus"
        });
        write_json_file(&mirror_settings, &original).expect("seed mirror");
        crate::settings::set_claude_mirror_config_dir(Some(mirror.clone()))
            .expect("set claude mirror dir");

        take_over_claude(&service).await;
        let taken_over: Value = read_json_file(&mirror_settings).expect("read mirror");
        assert!(ProxyService::is_claude_live_taken_over(&taken_over));

        let mut edited = taken_over.clone();
        edited["hooks"] = orca_hooks();
        write_json_file(&mirror_settings, &edited).expect("add hooks in WSL");

        service
            .stop_with_restore()
            .await
            .expect("stop with restore");

        let mut expected = original.clone();
        expected["hooks"] = orca_hooks();
        assert_eq!(
            read_json_file::<Value>(&mirror_settings).expect("read mirror"),
            expected
        );
        assert!(db
            .get_live_backup("claude_mirror")
            .await
            .expect("read")
            .is_none());
    }

    #[tokio::test]
    #[serial]
    async fn a_wsl_codex_install_left_on_the_proxy_is_recovered_after_an_unclean_exit() {
        let home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));

        let mirror = home.dir.path().join("wsl").join(".codex");
        std::fs::create_dir_all(&mirror).expect("mirror dir");
        let mirror_config = mirror.join("config.toml");
        std::fs::write(&mirror_config, "# wsl\nmodel = \"gpt-5\"\n").expect("seed config");
        write_json_file(
            &mirror.join("auth.json"),
            &chatgpt_login("acct-a", "2026-09-01T00:00:00Z"),
        )
        .expect("seed auth");
        set_codex_mirror_dir(&mirror);

        {
            let service = ProxyService::new(db.clone());
            service
                .write_codex_live(&chatgpt_live("model = \"gpt-5\"\n"))
                .expect("seed live");
            service
                .backup_live_config_strict(&AppType::Codex)
                .await
                .expect("back up");
            service
                .takeover_live_config_strict(&AppType::Codex)
                .await
                .expect("take over");
            // Killed here: no restore runs.
        }
        let taken_over = std::fs::read_to_string(&mirror_config).expect("read config");
        assert!(ProxyService::codex_openai_base_url_is_local(&taken_over));
        std::fs::write(
            &mirror_config,
            format!("{taken_over}\n[mcp_servers.orca]\ncommand = \"orca\"\n"),
        )
        .expect("add a table in WSL");

        let restarted = ProxyService::new(db.clone());
        assert!(restarted.detect_takeover_in_live_configs());
        restarted.recover_from_crash().await.expect("recover");

        let restored = std::fs::read_to_string(&mirror_config).expect("read config");
        let table: toml::Table = restored.parse().expect("valid toml");
        assert!(table.get("openai_base_url").is_none());
        assert_eq!(
            table["mcp_servers"]["orca"]["command"].as_str(),
            Some("orca")
        );
        assert!(restored.starts_with("# wsl"));
        assert!(!restarted.detect_takeover_in_live_configs());
    }

    #[tokio::test]
    #[serial]
    async fn codex_in_wsl_keeps_the_login_of_the_account_that_answered_when_handed_back() {
        let home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let official = |account: &str| json!({ "auth": chatgpt_login(account, "2026-09-01T00:00:00Z"), "config": "model = \"gpt-5\"\n" });
        let provider_a = Provider::with_id("a".into(), "A".into(), official("acct-a"), None);
        let provider_b = Provider::with_id("b".into(), "B".into(), official("acct-b"), None);
        db.save_provider("codex", &provider_a).expect("save a");
        db.save_provider("codex", &provider_b).expect("save b");
        db.set_current_provider("codex", "a").expect("set current");
        crate::settings::set_current_provider(&AppType::Codex, Some("a"))
            .expect("set local current");
        service
            .write_codex_live(&provider_a.settings_config)
            .expect("seed live");

        let mirror = home.dir.path().join("wsl").join(".codex");
        std::fs::create_dir_all(&mirror).expect("mirror dir");
        std::fs::write(mirror.join("config.toml"), "model = \"gpt-5\"\n").expect("seed config");
        write_json_file(
            &mirror.join("auth.json"),
            &chatgpt_login("acct-a", "2026-09-01T00:00:00Z"),
        )
        .expect("seed auth");
        set_codex_mirror_dir(&mirror);

        service
            .backup_live_config_strict(&AppType::Codex)
            .await
            .expect("back up");
        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over");
        service
            .hot_switch_provider("codex", "b")
            .await
            .expect("hot switch");
        let auth: Value = read_json_file(&mirror.join("auth.json")).expect("read auth");
        assert_eq!(
            auth["tokens"]["account_id"], "acct-a",
            "under the proxy the WSL login stays; the proxy presents b's"
        );

        // b has not answered a request, so Codex keeps a's working login
        // when the proxy lets go.
        service
            .stop_with_restore()
            .await
            .expect("stop with restore");
        let auth: Value = read_json_file(&mirror.join("auth.json")).expect("read auth");
        assert_eq!(auth["tokens"]["account_id"], "acct-a");

        // Once b has answered through the proxy, b's login is Codex's, and it
        // is what the hand-back leaves in place.
        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over again");
        crate::proxy::codex_pool::save_login_of_serving_account(db.as_ref(), &provider_b);
        service
            .stop_with_restore()
            .await
            .expect("stop with restore");
        let auth: Value = read_json_file(&mirror.join("auth.json")).expect("read auth");
        assert_eq!(auth["tokens"]["account_id"], "acct-b");
        let table: toml::Table = std::fs::read_to_string(mirror.join("config.toml"))
            .expect("read config")
            .parse()
            .expect("valid toml");
        assert!(table.get("openai_base_url").is_none());
    }

    #[test]
    #[serial]
    fn a_wsl_home_reaches_the_proxy_only_with_mirrored_networking() {
        let home = TempHome::new();
        let wsl = std::path::Path::new(r"\\wsl$\Ubuntu-22.04\home\agentcode\.codex");
        assert!(!ProxyService::mirror_reaches_proxy(wsl));
        std::fs::write(
            home.dir.path().join(".wslconfig"),
            "[wsl2]\nnetworkingMode = mirrored\n",
        )
        .expect("write .wslconfig");
        assert!(ProxyService::mirror_reaches_proxy(wsl));
        assert!(ProxyService::mirror_reaches_proxy(std::path::Path::new(
            r"D:\other\.codex"
        )));
    }

    #[tokio::test]
    #[serial]
    async fn hooks_in_the_file_before_the_takeover_survive_an_account_switch_and_the_restore() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "a-key" }, "model": "opus" }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "b-key" }, "model": "sonnet" }),
            None,
        );
        db.save_provider("claude", &provider_a).expect("save a");
        db.save_provider("claude", &provider_b).expect("save b");
        db.set_current_provider("claude", "a").expect("set current");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current");

        let mut original = provider_a.settings_config.clone();
        original["hooks"] = orca_hooks();
        service.write_claude_live(&original).expect("seed live");

        take_over_claude(&service).await;
        service
            .hot_switch_provider("claude", "b")
            .await
            .expect("hot switch");

        let live = service.read_claude_live().expect("read live");
        assert_eq!(live["hooks"], orca_hooks(), "the switch keeps the hooks");
        assert_eq!(
            live["model"], "opus",
            "the model is the user's, not the provider's"
        );
        assert!(ProxyService::is_claude_live_taken_over(&live));

        service
            .stop_with_restore()
            .await
            .expect("stop with restore");

        assert_eq!(
            service.read_claude_live().expect("read live"),
            json!({
                "env": { "ANTHROPIC_API_KEY": "b-key" },
                "model": "opus",
                "hooks": orca_hooks(),
            })
        );
    }

    #[tokio::test]
    #[serial]
    async fn codex_config_present_before_the_takeover_survives_an_account_switch() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let relay = |name: &str| {
            json!({
                "auth": { "OPENAI_API_KEY": format!("{name}-key") },
                "config": format!(
                    "model_provider = \"{name}\"\n\n[model_providers.{name}]\nbase_url = \"https://{name}.example/v1\"\n"
                )
            })
        };
        let provider_a = Provider::with_id("a".into(), "A".into(), relay("a"), None);
        let provider_b = Provider::with_id("b".into(), "B".into(), relay("b"), None);
        db.save_provider("codex", &provider_a).expect("save a");
        db.save_provider("codex", &provider_b).expect("save b");
        db.set_current_provider("codex", "a").expect("set current");
        crate::settings::set_current_provider(&AppType::Codex, Some("a"))
            .expect("set local current");

        let mut original = provider_a.settings_config.clone();
        original["config"] = json!(format!(
            "{}\n[projects.'C:\\Projects']\ntrust_level = \"trusted\"\n",
            provider_a.settings_config["config"].as_str().unwrap()
        ));
        service.write_codex_live(&original).expect("seed live");

        service
            .backup_live_config_strict(&AppType::Codex)
            .await
            .expect("back up");
        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over");
        service
            .hot_switch_provider("codex", "b")
            .await
            .expect("hot switch");
        service
            .stop_with_restore()
            .await
            .expect("stop with restore");

        let table: toml::Table =
            std::fs::read_to_string(crate::codex_config::get_codex_config_path())
                .expect("read config")
                .parse()
                .expect("valid toml");
        assert_eq!(table["model_provider"].as_str(), Some("b"));
        assert!(table["model_providers"].get("a").is_none());
        assert_eq!(
            table["projects"]["C:\\Projects"]["trust_level"].as_str(),
            Some("trusted")
        );
    }
}
