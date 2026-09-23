//! Request forwarder
//!
//! Forwards requests to the upstream provider, with failover

use super::hyper_client::ProxyResponse;
use super::{
    body_filter::filter_private_params_with_whitelist,
    error::*,
    failover_switch::FailoverSwitchManager,
    log_codes::fwd as log_fwd,
    provider_router::ProviderRouter,
    providers::{get_adapter, AuthInfo, AuthStrategy, ProviderAdapter, ProviderType},
    thinking_budget_rectifier::{rectify_thinking_budget, should_rectify_thinking_budget},
    thinking_rectifier::{
        normalize_thinking_type, rectify_anthropic_request, should_rectify_thinking_signature,
    },
    types::{CopilotOptimizerConfig, OptimizerConfig, ProxyStatus, RectifierConfig},
    ProxyError,
};
use crate::commands::CopilotAuthState;
use crate::database::SwitchReason;
use crate::proxy::providers::copilot_auth::CopilotAuthManager;
use crate::{app_config::AppType, provider::Provider};
use http::Extensions;
use serde_json::Value;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

pub struct ForwardResult {
    pub response: ProxyResponse,
    pub provider: Provider,
    pub claude_api_format: Option<String>,
}

pub struct ForwardError {
    pub error: ProxyError,
    pub provider: Option<Provider>,
}

pub struct RequestForwarder {
    /// Shared ProviderRouter (holds circuit breaker state)
    router: Arc<ProviderRouter>,
    status: Arc<RwLock<ProxyStatus>>,
    current_providers: Arc<RwLock<std::collections::HashMap<String, (String, String)>>>,
    /// Failover switch manager
    failover_manager: Arc<FailoverSwitchManager>,
    /// AppHandle, for emitting events and updating the tray
    app_handle: Option<tauri::AppHandle>,
    /// The "current provider ID" when the request started (decides whether the UI/tray need syncing)
    current_provider_id_at_start: String,
    /// Rectifier configuration
    rectifier_config: RectifierConfig,
    /// Optimizer configuration
    optimizer_config: OptimizerConfig,
    /// Copilot optimizer configuration
    copilot_optimizer_config: CopilotOptimizerConfig,
    /// Non-streaming request timeout (seconds)
    non_streaming_timeout: std::time::Duration,
}

impl RequestForwarder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        router: Arc<ProviderRouter>,
        non_streaming_timeout: u64,
        status: Arc<RwLock<ProxyStatus>>,
        current_providers: Arc<RwLock<std::collections::HashMap<String, (String, String)>>>,
        failover_manager: Arc<FailoverSwitchManager>,
        app_handle: Option<tauri::AppHandle>,
        current_provider_id_at_start: String,
        _streaming_first_byte_timeout: u64,
        _streaming_idle_timeout: u64,
        rectifier_config: RectifierConfig,
        optimizer_config: OptimizerConfig,
        copilot_optimizer_config: CopilotOptimizerConfig,
    ) -> Self {
        Self {
            router,
            status,
            current_providers,
            failover_manager,
            app_handle,
            current_provider_id_at_start,
            rectifier_config,
            optimizer_config,
            copilot_optimizer_config,
            non_streaming_timeout: std::time::Duration::from_secs(non_streaming_timeout),
        }
    }

    /// Forwards a request (with failover)
    ///
    /// # Arguments
    /// * `app_type` - app type
    /// * `endpoint` - API endpoint
    /// * `body` - request body
    /// * `headers` - request headers
    /// * `providers` - the selected providers (supplied by RequestContext, so select_providers is not called again)
    pub async fn forward_with_retry(
        &self,
        app_type: &AppType,
        endpoint: &str,
        body: Value,
        headers: axum::http::HeaderMap,
        extensions: Extensions,
        providers: Vec<Provider>,
    ) -> Result<ForwardResult, ForwardError> {
        // Get the adapter
        let adapter = get_adapter(app_type);
        let app_type_str = app_type.as_str();

        if providers.is_empty() {
            return Err(ForwardError {
                error: ProxyError::NoAvailableProvider,
                provider: None,
            });
        }

        let mut last_error = None;
        let mut last_provider = None;
        let mut attempted_providers = 0usize;
        // Why the provider the app had selected was passed over, for the
        // switch history.
        let mut passed_over: Option<(SwitchReason, String)> = None;
        if !self.current_provider_id_at_start.is_empty()
            && providers.first().map(|p| p.id.as_str())
                != Some(self.current_provider_id_at_start.as_str())
        {
            let detail = if providers
                .iter()
                .any(|p| p.id == self.current_provider_id_at_start)
            {
                "Near its usage limit"
            } else {
                "Taken out of rotation after repeated failures"
            };
            passed_over = Some((SwitchReason::Rotation, detail.to_string()));
        }

        // Rectifier retry flags: rectification fires at most once
        let mut rectifier_retried = false;
        let mut budget_rectifier_retried = false;

        // With a single provider, skip the circuit breaker check (failover off)
        let bypass_circuit_breaker = providers.len() == 1;

        // Try each provider in turn
        for provider in providers.iter() {
            // Take a circuit breaker permit before sending (HalfOpen uses up a probe slot)
            // Skipped with a single provider so the breaker cannot block every request
            let (allowed, used_half_open_permit) = if bypass_circuit_breaker {
                (true, false)
            } else {
                let permit = self
                    .router
                    .allow_provider_request(&provider.id, app_type_str)
                    .await;
                (permit.allowed, permit.used_half_open_permit)
            };

            if !allowed {
                if provider.id == self.current_provider_id_at_start && passed_over.is_none() {
                    passed_over = Some((
                        SwitchReason::Rotation,
                        "Circuit breaker open after repeated failures".to_string(),
                    ));
                }
                continue;
            }

            // PRE-SEND optimizer: each provider decides independently whether to optimize
            // Clone the body so Bedrock optimization fields do not leak to non-Bedrock providers (failover)
            let mut provider_body =
                if self.optimizer_config.enabled && is_bedrock_provider(provider) {
                    let mut b = body.clone();
                    if self.optimizer_config.thinking_optimizer {
                        super::thinking_optimizer::optimize(&mut b, &self.optimizer_config);
                    }
                    if self.optimizer_config.cache_injection {
                        super::cache_injector::inject(&mut b, &self.optimizer_config);
                    }
                    b
                } else {
                    body.clone()
                };

            attempted_providers += 1;

            // Update the current provider in the status
            {
                let mut status = self.status.write().await;
                status.current_provider = Some(provider.name.clone());
                status.current_provider_id = Some(provider.id.clone());
                status.total_requests += 1;
                status.last_request_at = Some(chrono::Utc::now().to_rfc3339());
            }

            // Forward the request (one attempt per provider; the client controls retries)
            match self
                .forward(
                    provider,
                    endpoint,
                    &provider_body,
                    &headers,
                    &extensions,
                    adapter.as_ref(),
                )
                .await
            {
                Ok((response, claude_api_format)) => {
                    // Success: record it and update the circuit breaker
                    let _ = self
                        .router
                        .record_result(
                            &provider.id,
                            app_type_str,
                            used_half_open_permit,
                            true,
                            None,
                        )
                        .await;

                    // Update the provider used by this app type
                    {
                        let mut current_providers = self.current_providers.write().await;
                        current_providers.insert(
                            app_type_str.to_string(),
                            (provider.id.clone(), provider.name.clone()),
                        );
                    }

                    // Update success statistics
                    {
                        let mut status = self.status.write().await;
                        status.success_requests += 1;
                        status.last_error = None;
                        let should_switch =
                            self.current_provider_id_at_start.as_str() != provider.id.as_str();
                        if should_switch {
                            status.failover_count += 1;

                            // Trigger the provider switch asynchronously: update the UI/tray and sync the "current provider" to the one actually used
                            let fm = self.failover_manager.clone();
                            let ah = self.app_handle.clone();
                            let pid = provider.id.clone();
                            let pname = provider.name.clone();
                            let at = app_type_str.to_string();
                            let reason = passed_over.clone();

                            tokio::spawn(async move {
                                let _ = fm.try_switch(ah.as_ref(), &at, &pid, &pname, reason).await;
                            });
                        }
                        // Recompute the success rate
                        if status.total_requests > 0 {
                            status.success_rate = (status.success_requests as f32
                                / status.total_requests as f32)
                                * 100.0;
                        }
                    }

                    return Ok(ForwardResult {
                        response,
                        provider: provider.clone(),
                        claude_api_format,
                    });
                }
                Err(e) => {
                    // Check whether to trigger the rectifier (Claude/ClaudeAuth providers only)
                    let provider_type = ProviderType::from_app_type_and_config(app_type, provider);
                    let is_anthropic_provider = matches!(
                        provider_type,
                        ProviderType::Claude | ProviderType::ClaudeAuth
                    );
                    let mut signature_rectifier_non_retryable_client_error = false;

                    if is_anthropic_provider {
                        let error_message = extract_error_message(&e);
                        if should_rectify_thinking_signature(
                            error_message.as_deref(),
                            &self.rectifier_config,
                        ) {
                            // Already retried: return the error (non-retryable client error)
                            if rectifier_retried {
                                log::warn!("[{app_type_str}] [RECT-005] Rectifier already fired; not retrying again");
                                // Release the HalfOpen permit (not recorded in the breaker; this is a client compatibility issue)
                                self.router
                                    .release_permit_neutral(
                                        &provider.id,
                                        app_type_str,
                                        used_half_open_permit,
                                    )
                                    .await;
                                let mut status = self.status.write().await;
                                status.failed_requests += 1;
                                status.last_error = Some(e.to_string());
                                if status.total_requests > 0 {
                                    status.success_rate = (status.success_requests as f32
                                        / status.total_requests as f32)
                                        * 100.0;
                                }
                                return Err(ForwardError {
                                    error: e,
                                    provider: Some(provider.clone()),
                                });
                            }

                            // First trigger: rectify the request body
                            let rectified = rectify_anthropic_request(&mut provider_body);

                            // Rectification changed nothing: go on to the budget rectifier path rather than short-circuiting on a misjudgement
                            if !rectified.applied {
                                log::warn!(
                                    "[{app_type_str}] [RECT-006] thinking signature rectifier fired but found nothing to rectify; checking budget next, and returning a client error if budget does not match either"
                                );
                                signature_rectifier_non_retryable_client_error = true;
                            } else {
                                log::info!(
                                    "[{}] [RECT-001] thinking signature rectifier fired, removed {} thinking blocks, {} redacted_thinking blocks, {} signature fields",
                                    app_type_str,
                                    rectified.removed_thinking_blocks,
                                    rectified.removed_redacted_thinking_blocks,
                                    rectified.removed_signature_fields
                                );

                                // Mark as retried (the retry always returns under the current logic; the flag is kept for future use)
                                let _ = std::mem::replace(&mut rectifier_retried, true);

                                // Retry with the same provider (not counted by the circuit breaker)
                                match self
                                    .forward(
                                        provider,
                                        endpoint,
                                        &provider_body,
                                        &headers,
                                        &extensions,
                                        adapter.as_ref(),
                                    )
                                    .await
                                {
                                    Ok((response, claude_api_format)) => {
                                        log::info!(
                                            "[{app_type_str}] [RECT-002] Rectified retry succeeded"
                                        );
                                        // Record success
                                        let _ = self
                                            .router
                                            .record_result(
                                                &provider.id,
                                                app_type_str,
                                                used_half_open_permit,
                                                true,
                                                None,
                                            )
                                            .await;

                                        // Update the provider used by this app type
                                        {
                                            let mut current_providers =
                                                self.current_providers.write().await;
                                            current_providers.insert(
                                                app_type_str.to_string(),
                                                (provider.id.clone(), provider.name.clone()),
                                            );
                                        }

                                        // Update success statistics
                                        {
                                            let mut status = self.status.write().await;
                                            status.success_requests += 1;
                                            status.last_error = None;
                                            let should_switch =
                                                self.current_provider_id_at_start.as_str()
                                                    != provider.id.as_str();
                                            if should_switch {
                                                status.failover_count += 1;

                                                // Trigger the provider switch asynchronously and update the UI/tray
                                                let fm = self.failover_manager.clone();
                                                let ah = self.app_handle.clone();
                                                let pid = provider.id.clone();
                                                let pname = provider.name.clone();
                                                let at = app_type_str.to_string();
                                                let reason = passed_over.clone();

                                                tokio::spawn(async move {
                                                    let _ = fm
                                                        .try_switch(
                                                            ah.as_ref(),
                                                            &at,
                                                            &pid,
                                                            &pname,
                                                            reason,
                                                        )
                                                        .await;
                                                });
                                            }
                                            if status.total_requests > 0 {
                                                status.success_rate = (status.success_requests
                                                    as f32
                                                    / status.total_requests as f32)
                                                    * 100.0;
                                            }
                                        }

                                        return Ok(ForwardResult {
                                            response,
                                            provider: provider.clone(),
                                            claude_api_format,
                                        });
                                    }
                                    Err(retry_err) => {
                                        // Rectified retry still failed: the error type decides whether the breaker records it
                                        log::warn!(
                                            "[{app_type_str}] [RECT-003] Rectified retry still failed: {retry_err}"
                                        );

                                        // By error type: provider problems are recorded as failures; client problems only release the permit
                                        let is_provider_error = counts_against_provider(&retry_err);

                                        if is_provider_error {
                                            // Provider problem: record the failure in the circuit breaker
                                            let _ = self
                                                .router
                                                .record_result(
                                                    &provider.id,
                                                    app_type_str,
                                                    used_half_open_permit,
                                                    false,
                                                    Some(retry_err.to_string()),
                                                )
                                                .await;
                                        } else {
                                            // Client problem: only release the permit; not recorded in the breaker
                                            self.router
                                                .release_permit_neutral(
                                                    &provider.id,
                                                    app_type_str,
                                                    used_half_open_permit,
                                                )
                                                .await;
                                        }

                                        let mut status = self.status.write().await;
                                        status.failed_requests += 1;
                                        status.last_error = Some(retry_err.to_string());
                                        if status.total_requests > 0 {
                                            status.success_rate = (status.success_requests as f32
                                                / status.total_requests as f32)
                                                * 100.0;
                                        }
                                        return Err(ForwardError {
                                            error: retry_err,
                                            provider: Some(provider.clone()),
                                        });
                                    }
                                }
                            }
                        }
                    }

                    // Check whether to trigger the budget rectifier (Claude/ClaudeAuth providers only)
                    if is_anthropic_provider {
                        let error_message = extract_error_message(&e);
                        if should_rectify_thinking_budget(
                            error_message.as_deref(),
                            &self.rectifier_config,
                        ) {
                            // Already retried: return the error (non-retryable client error)
                            if budget_rectifier_retried {
                                log::warn!(
                                    "[{app_type_str}] [RECT-013] budget rectifier already fired; not retrying again"
                                );
                                self.router
                                    .release_permit_neutral(
                                        &provider.id,
                                        app_type_str,
                                        used_half_open_permit,
                                    )
                                    .await;
                                let mut status = self.status.write().await;
                                status.failed_requests += 1;
                                status.last_error = Some(e.to_string());
                                if status.total_requests > 0 {
                                    status.success_rate = (status.success_requests as f32
                                        / status.total_requests as f32)
                                        * 100.0;
                                }
                                return Err(ForwardError {
                                    error: e,
                                    provider: Some(provider.clone()),
                                });
                            }

                            let budget_rectified = rectify_thinking_budget(&mut provider_body);
                            if !budget_rectified.applied {
                                log::warn!(
                                    "[{app_type_str}] [RECT-014] budget rectifier fired but found nothing to rectify; skipping a pointless retry"
                                );
                                self.router
                                    .release_permit_neutral(
                                        &provider.id,
                                        app_type_str,
                                        used_half_open_permit,
                                    )
                                    .await;
                                let mut status = self.status.write().await;
                                status.failed_requests += 1;
                                status.last_error = Some(e.to_string());
                                if status.total_requests > 0 {
                                    status.success_rate = (status.success_requests as f32
                                        / status.total_requests as f32)
                                        * 100.0;
                                }
                                return Err(ForwardError {
                                    error: e,
                                    provider: Some(provider.clone()),
                                });
                            }

                            log::info!(
                                "[{}] [RECT-010] thinking budget rectifier fired, before={:?}, after={:?}",
                                app_type_str,
                                budget_rectified.before,
                                budget_rectified.after
                            );

                            let _ = std::mem::replace(&mut budget_rectifier_retried, true);

                            // Retry with the same provider (not counted by the circuit breaker)
                            match self
                                .forward(
                                    provider,
                                    endpoint,
                                    &provider_body,
                                    &headers,
                                    &extensions,
                                    adapter.as_ref(),
                                )
                                .await
                            {
                                Ok((response, claude_api_format)) => {
                                    log::info!("[{app_type_str}] [RECT-011] budget rectified retry succeeded");
                                    let _ = self
                                        .router
                                        .record_result(
                                            &provider.id,
                                            app_type_str,
                                            used_half_open_permit,
                                            true,
                                            None,
                                        )
                                        .await;

                                    {
                                        let mut current_providers =
                                            self.current_providers.write().await;
                                        current_providers.insert(
                                            app_type_str.to_string(),
                                            (provider.id.clone(), provider.name.clone()),
                                        );
                                    }

                                    {
                                        let mut status = self.status.write().await;
                                        status.success_requests += 1;
                                        status.last_error = None;
                                        let should_switch =
                                            self.current_provider_id_at_start.as_str()
                                                != provider.id.as_str();
                                        if should_switch {
                                            status.failover_count += 1;
                                            let fm = self.failover_manager.clone();
                                            let ah = self.app_handle.clone();
                                            let pid = provider.id.clone();
                                            let pname = provider.name.clone();
                                            let at = app_type_str.to_string();
                                            let reason = passed_over.clone();
                                            tokio::spawn(async move {
                                                let _ = fm
                                                    .try_switch(
                                                        ah.as_ref(),
                                                        &at,
                                                        &pid,
                                                        &pname,
                                                        reason,
                                                    )
                                                    .await;
                                            });
                                        }
                                        if status.total_requests > 0 {
                                            status.success_rate = (status.success_requests as f32
                                                / status.total_requests as f32)
                                                * 100.0;
                                        }
                                    }

                                    return Ok(ForwardResult {
                                        response,
                                        provider: provider.clone(),
                                        claude_api_format,
                                    });
                                }
                                Err(retry_err) => {
                                    log::warn!(
                                        "[{app_type_str}] [RECT-012] budget rectified retry still failed: {retry_err}"
                                    );

                                    let is_provider_error = counts_against_provider(&retry_err);

                                    if is_provider_error {
                                        let _ = self
                                            .router
                                            .record_result(
                                                &provider.id,
                                                app_type_str,
                                                used_half_open_permit,
                                                false,
                                                Some(retry_err.to_string()),
                                            )
                                            .await;
                                    } else {
                                        self.router
                                            .release_permit_neutral(
                                                &provider.id,
                                                app_type_str,
                                                used_half_open_permit,
                                            )
                                            .await;
                                    }

                                    let mut status = self.status.write().await;
                                    status.failed_requests += 1;
                                    status.last_error = Some(retry_err.to_string());
                                    if status.total_requests > 0 {
                                        status.success_rate = (status.success_requests as f32
                                            / status.total_requests as f32)
                                            * 100.0;
                                    }
                                    return Err(ForwardError {
                                        error: retry_err,
                                        provider: Some(provider.clone()),
                                    });
                                }
                            }
                        }
                    }

                    if signature_rectifier_non_retryable_client_error {
                        self.router
                            .release_permit_neutral(
                                &provider.id,
                                app_type_str,
                                used_half_open_permit,
                            )
                            .await;
                        let mut status = self.status.write().await;
                        status.failed_requests += 1;
                        status.last_error = Some(e.to_string());
                        if status.total_requests > 0 {
                            status.success_rate = (status.success_requests as f32
                                / status.total_requests as f32)
                                * 100.0;
                        }
                        return Err(ForwardError {
                            error: e,
                            provider: Some(provider.clone()),
                        });
                    }

                    // Failure: count it against the provider only when it says
                    // something about the provider; otherwise just free the permit.
                    if counts_against_provider(&e) {
                        let _ = self
                            .router
                            .record_result(
                                &provider.id,
                                app_type_str,
                                used_half_open_permit,
                                false,
                                Some(e.to_string()),
                            )
                            .await;
                    } else {
                        self.router
                            .release_permit_neutral(
                                &provider.id,
                                app_type_str,
                                used_half_open_permit,
                            )
                            .await;
                    }

                    // Classify the error
                    let category = self.categorize_proxy_error(&e);

                    match category {
                        ErrorCategory::Retryable => {
                            // Retryable: update the error info and try the next provider
                            {
                                let mut status = self.status.write().await;
                                status.last_error =
                                    Some(format!("Provider {} failed: {}", provider.name, e));
                            }

                            let (log_code, log_message) = build_retryable_failure_log(
                                &provider.name,
                                attempted_providers,
                                providers.len(),
                                &e,
                            );
                            log::warn!("[{app_type_str}] [{log_code}] {log_message}");

                            if provider.id == self.current_provider_id_at_start
                                && passed_over.is_none()
                            {
                                passed_over = Some(switch_reason_for(provider, &e));
                            }
                            last_error = Some(e);
                            last_provider = Some(provider.clone());
                            // Try the next provider
                            continue;
                        }
                        ErrorCategory::NonRetryable | ErrorCategory::ClientAbort => {
                            // Not retryable: return the error
                            {
                                let mut status = self.status.write().await;
                                status.failed_requests += 1;
                                status.last_error = Some(e.to_string());
                                if status.total_requests > 0 {
                                    status.success_rate = (status.success_requests as f32
                                        / status.total_requests as f32)
                                        * 100.0;
                                }
                            }
                            return Err(ForwardError {
                                error: e,
                                provider: Some(provider.clone()),
                            });
                        }
                    }
                }
            }
        }

        if attempted_providers == 0 {
            // The provider list is not empty but the circuit breaker rejected them all (typically: the HalfOpen probe slot is taken)
            {
                let mut status = self.status.write().await;
                status.failed_requests += 1;
                status.last_error =
                    Some("No provider is available right now (all circuits are open)".to_string());
                if status.total_requests > 0 {
                    status.success_rate =
                        (status.success_requests as f32 / status.total_requests as f32) * 100.0;
                }
            }
            return Err(ForwardError {
                error: ProxyError::NoAvailableProvider,
                provider: None,
            });
        }

        // Every provider failed
        {
            let mut status = self.status.write().await;
            status.failed_requests += 1;
            status.last_error = Some("Every provider failed".to_string());
            if status.total_requests > 0 {
                status.success_rate =
                    (status.success_requests as f32 / status.total_requests as f32) * 100.0;
            }
        }

        if let Some((log_code, log_message)) =
            build_terminal_failure_log(attempted_providers, providers.len(), last_error.as_ref())
        {
            log::warn!("[{app_type_str}] [{log_code}] {log_message}");
        }

        Err(ForwardError {
            error: last_error.unwrap_or(ProxyError::MaxRetriesExceeded),
            provider: last_provider,
        })
    }

    /// Forwards a single request (through the adapter)
    ///
    /// A ChatGPT-login Codex account gets one more try after a 401, with its
    /// login refreshed first; a usage-limit refusal is noted so the account is
    /// passed over until it resets.
    async fn forward(
        &self,
        provider: &Provider,
        endpoint: &str,
        body: &Value,
        headers: &axum::http::HeaderMap,
        extensions: &Extensions,
        adapter: &dyn ProviderAdapter,
    ) -> Result<(ProxyResponse, Option<String>), ProxyError> {
        let first = self
            .forward_once(
                provider, endpoint, body, headers, extensions, adapter, false,
            )
            .await;
        let pooled_codex =
            adapter.name() == "Codex" && super::codex_pool::is_chatgpt_provider(provider);
        let pooled_claude = adapter.name() == "Claude"
            && super::providers::ClaudeAdapter::serves_captured_login(provider);
        if !pooled_codex && !pooled_claude {
            return first;
        }
        let result = match first {
            Err(ProxyError::UpstreamError { status: 401, .. }) => {
                log::info!(
                    "[{}] provider={} answered 401; refreshing its login and retrying once",
                    adapter.name(),
                    provider.id
                );
                self.forward_once(provider, endpoint, body, headers, extensions, adapter, true)
                    .await
            }
            other => other,
        };
        if pooled_codex {
            if let Err(ProxyError::UpstreamError { status, body }) = &result {
                super::codex_pool::record_limit_refusal(&provider.id, *status, body.as_deref());
            }
        }
        if result.is_err() {
            self.announce_if_signed_out(provider, pooled_codex);
        } else if pooled_codex {
            super::codex_pool::save_login_of_serving_account(self.router.db(), provider);
        }
        result
    }

    /// Tells the window when a pooled account has been refused for good
    /// (its refresh token was rejected), naming the account, so the user
    /// learns why the pool moved off it and that it needs signing in again.
    fn announce_if_signed_out(&self, provider: &Provider, codex: bool) {
        let signed_out = if codex {
            super::codex_pool::needs_sign_in(provider)
        } else {
            super::claude_pool::needs_sign_in(provider)
        };
        if !signed_out {
            return;
        }
        let account = provider.account_email();
        if let Some(app) = self.app_handle.as_ref() {
            let payload = serde_json::json!({
                "appType": if codex { "codex" } else { "claude" },
                "providerId": provider.id,
                "providerName": provider.name,
                "account": account,
            });
            if let Err(e) = tauri::Emitter::emit(app, "account-needs-sign-in", payload) {
                log::warn!("Could not announce a signed-out account: {e}");
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn forward_once(
        &self,
        provider: &Provider,
        endpoint: &str,
        body: &Value,
        headers: &axum::http::HeaderMap,
        extensions: &Extensions,
        adapter: &dyn ProviderAdapter,
        force_login_refresh: bool,
    ) -> Result<(ProxyResponse, Option<String>), ProxyError> {
        // Extract base_url via the adapter
        let mut base_url = adapter.extract_base_url(provider)?;

        let is_full_url = provider
            .meta
            .as_ref()
            .and_then(|meta| meta.is_full_url)
            .unwrap_or(false);

        // Apply model mapping (independent of format conversion)
        let (mapped_body, _original_model, _mapped_model) =
            super::model_mapper::apply_model_mapping(body.clone(), provider);

        // Aligned with CCH: no proactive thinking rewrite before the request (only the compatibility entry point is kept)
        let mut mapped_body = normalize_thinking_type(mapped_body);

        let claude_oauth = adapter.name() == "Claude"
            && super::providers::ClaudeAdapter::serves_captured_login(provider);
        if claude_oauth {
            if let Some(uuid) = super::claude_pool::account_uuid(provider) {
                super::claude_pool::patch_account_uuid(&mut mapped_body, uuid);
            }
        }

        // Determine the effective endpoint
        // The GitHub Copilot API uses /chat/completions (no /v1 prefix)
        let is_copilot = provider
            .meta
            .as_ref()
            .and_then(|m| m.provider_type.as_deref())
            == Some("github_copilot")
            || base_url.contains("githubcopilot.com");

        // --- Copilot optimizer: body optimization + classification (before format conversion) ---
        // Note: the deterministic ID is also computed here because mapped_body is moved during format conversion
        let copilot_optimization = if is_copilot && self.copilot_optimizer_config.enabled {
            // 1. Tool result merging — must run before classification
            //    Merging turns [tool_result, text] into [tool_result(with text)],
            //    so classification sees agent (all tool_result) rather than user (has a text block)
            if self.copilot_optimizer_config.tool_result_merging {
                mapped_body = super::copilot_optimizer::merge_tool_results(mapped_body);
            }

            // 2. Classify the merged body
            let has_anthropic_beta = headers.contains_key("anthropic-beta");
            let classification = super::copilot_optimizer::classify_request(
                &mapped_body,
                has_anthropic_beta,
                self.copilot_optimizer_config.compact_detection,
            );

            log::debug!(
                "[Copilot] Optimizer classification: initiator={}, is_warmup={}, is_compact={}",
                classification.initiator,
                classification.is_warmup,
                classification.is_compact
            );

            // 3. Warmup downgrade to a small model
            if self.copilot_optimizer_config.warmup_downgrade && classification.is_warmup {
                log::info!(
                    "[Copilot] Warmup request downgraded to model: {}",
                    self.copilot_optimizer_config.warmup_model
                );
                mapped_body["model"] =
                    serde_json::json!(&self.copilot_optimizer_config.warmup_model);
            }

            // Precompute the deterministic request ID (before the body is moved)
            // The session_id comes from body.metadata.user_id or the request headers
            let session_id = body
                .pointer("/metadata/user_id")
                .and_then(|v| v.as_str())
                .or_else(|| headers.get("x-session-id").and_then(|v| v.to_str().ok()))
                .unwrap_or("");
            let det_request_id = if self.copilot_optimizer_config.deterministic_request_id {
                Some(super::copilot_optimizer::deterministic_request_id(
                    &mapped_body,
                    session_id,
                ))
            } else {
                None
            };

            Some((classification, det_request_id))
        } else {
            None
        };

        // GitHub Copilot dynamic endpoint routing
        // Get the cached API endpoint from CopilotAuthManager (supports enterprise and other non-default endpoints)
        if is_copilot && !is_full_url {
            if let Some(app_handle) = &self.app_handle {
                let copilot_state = app_handle.state::<CopilotAuthState>();
                let copilot_auth = copilot_state.0.read().await;

                // Get the linked GitHub account ID from provider.meta
                let account_id = provider
                    .meta
                    .as_ref()
                    .and_then(|m| m.managed_account_id_for("github_copilot"));

                let dynamic_endpoint = match &account_id {
                    Some(id) => copilot_auth.get_api_endpoint(id).await,
                    None => copilot_auth.get_default_api_endpoint().await,
                };

                // Replace only when the dynamic endpoint differs from the current base_url
                if dynamic_endpoint != base_url {
                    log::debug!(
                        "[Copilot] Using dynamic API endpoint: {} (was: {})",
                        dynamic_endpoint,
                        base_url
                    );
                    base_url = dynamic_endpoint;
                }
            }
        }
        let resolved_claude_api_format = if adapter.name() == "Claude" {
            Some(
                self.resolve_claude_api_format(provider, &mapped_body, is_copilot)
                    .await,
            )
        } else {
            None
        };
        let needs_transform = match resolved_claude_api_format.as_deref() {
            Some(api_format) => super::providers::claude_api_format_needs_transform(api_format),
            None => adapter.needs_transform(provider),
        };
        let (effective_endpoint, passthrough_query) =
            if needs_transform && adapter.name() == "Claude" {
                let api_format = resolved_claude_api_format
                    .as_deref()
                    .unwrap_or_else(|| super::providers::get_claude_api_format(provider));
                rewrite_claude_transform_endpoint(endpoint, api_format, is_copilot)
            } else {
                (
                    endpoint.to_string(),
                    split_endpoint_and_query(endpoint)
                        .1
                        .map(ToString::to_string),
                )
            };

        let url = if is_full_url {
            append_query_to_full_url(&base_url, passthrough_query.as_deref())
        } else {
            adapter.build_url(&base_url, &effective_endpoint)
        };

        // Transform the request body (if needed)
        let request_body = if needs_transform {
            if adapter.name() == "Claude" {
                let api_format = resolved_claude_api_format
                    .as_deref()
                    .unwrap_or_else(|| super::providers::get_claude_api_format(provider));
                super::providers::transform_claude_request_for_api_format(
                    mapped_body,
                    provider,
                    api_format,
                )?
            } else {
                adapter.transform_request(mapped_body, provider)?
            }
        } else {
            mapped_body
        };

        // Remove private parameters (fields starting with `_`) so internal data does not leak upstream
        // Uses an empty whitelist by default, removing every _-prefixed field
        let filtered_body = filter_private_params_with_whitelist(request_body, &[]);
        let force_identity_encoding = needs_transform
            || should_force_identity_encoding(&effective_endpoint, &filtered_body, headers);

        // Get the auth headers (prepared up front for in-place replacement)
        let mut auth_headers = if let Some(mut auth) = adapter.extract_auth(provider) {
            // GitHub Copilot special case: get the real token from CopilotAuthManager
            if auth.strategy == AuthStrategy::GitHubCopilot {
                if let Some(app_handle) = &self.app_handle {
                    let copilot_state = app_handle.state::<CopilotAuthState>();
                    let copilot_auth: tokio::sync::RwLockReadGuard<'_, CopilotAuthManager> =
                        copilot_state.0.read().await;

                    // Get the linked GitHub account ID from provider.meta (multi-account support)
                    let account_id = provider
                        .meta
                        .as_ref()
                        .and_then(|m| m.managed_account_id_for("github_copilot"));

                    // Get the token for that account ID (backward compatible: without an account ID, use the first account)
                    let token_result = match &account_id {
                        Some(id) => {
                            log::debug!("[Copilot] Getting token for account {id}");
                            copilot_auth.get_valid_token_for_account(id).await
                        }
                        None => {
                            log::debug!("[Copilot] Getting token for the default account");
                            copilot_auth.get_valid_token().await
                        }
                    };

                    match token_result {
                        Ok(token) => {
                            auth = AuthInfo::new(token, AuthStrategy::GitHubCopilot);
                            log::debug!(
                                "[Copilot] Got Copilot token (account={})",
                                account_id.as_deref().unwrap_or("default")
                            );
                        }
                        Err(e) => {
                            log::error!(
                                "[Copilot] Failed to get Copilot token (account={}): {e}",
                                account_id.as_deref().unwrap_or("default")
                            );
                            return Err(ProxyError::AuthError(format!(
                                "GitHub Copilot authentication failed: {e}"
                            )));
                        }
                    }
                } else {
                    log::error!("[Copilot] AppHandle unavailable");
                    return Err(ProxyError::AuthError(
                        "GitHub Copilot authentication is unavailable (no AppHandle)".to_string(),
                    ));
                }
            }
            if auth.strategy == AuthStrategy::ChatGpt {
                super::account_pool::ensure_exit_allowed(
                    self.router.db(),
                    provider,
                    super::codex_pool::EXIT_TRACE_URL,
                )
                .await?;
                let credentials = super::codex_pool::credentials_for(
                    self.router.db(),
                    provider,
                    force_login_refresh,
                )
                .await?;
                let mut chatgpt_headers = vec![(
                    http::header::AUTHORIZATION,
                    http::HeaderValue::from_str(&format!("Bearer {}", credentials.access_token))
                        .map_err(|e| ProxyError::AuthError(format!("invalid access token: {e}")))?,
                )];
                if let Some(account_id) = credentials.account_id.as_deref() {
                    if let Ok(value) = http::HeaderValue::from_str(account_id) {
                        chatgpt_headers
                            .push((http::HeaderName::from_static("chatgpt-account-id"), value));
                    }
                }
                chatgpt_headers
            } else if auth.strategy == AuthStrategy::ClaudeOAuth {
                super::account_pool::ensure_exit_allowed(
                    self.router.db(),
                    provider,
                    super::claude_pool::EXIT_TRACE_URL,
                )
                .await?;
                let token =
                    super::claude_pool::access_token_for(provider, force_login_refresh).await?;
                vec![(
                    http::header::AUTHORIZATION,
                    http::HeaderValue::from_str(&format!("Bearer {token}"))
                        .map_err(|e| ProxyError::AuthError(format!("invalid access token: {e}")))?,
                )]
            } else {
                adapter.get_auth_headers(&auth)
            }
        } else {
            Vec::new()
        };
        // The account id Codex sent names its own login, not the one presented.
        let replaces_account_id = auth_headers
            .iter()
            .any(|(name, _)| name.as_str() == "chatgpt-account-id");

        // --- Copilot optimizer: dynamic header injection ---
        if let Some((ref classification, ref det_request_id)) = copilot_optimization {
            for (name, value) in auth_headers.iter_mut() {
                match name.as_str() {
                    "x-initiator" if self.copilot_optimizer_config.request_classification => {
                        *value = http::HeaderValue::from_static(classification.initiator);
                    }
                    "x-request-id" | "x-agent-task-id" => {
                        if let Some(ref det_id) = det_request_id {
                            if let Ok(hv) = http::HeaderValue::from_str(det_id) {
                                *value = hv;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Copilot fingerprint header names (injected by get_auth_headers; must be deduplicated against the original headers)
        let copilot_fingerprint_headers: &[&str] = if is_copilot {
            &[
                "user-agent",
                "editor-version",
                "editor-plugin-version",
                "copilot-integration-id",
                "x-github-api-version",
                "openai-intent",
                // Newer headers
                "x-initiator",
                "x-interaction-type",
                "x-vscode-user-agent-library-version",
                "x-request-id",
                "x-agent-task-id",
            ]
        } else {
            &[]
        };

        // Precompute the upstream host value (to replace the host header in place)
        let upstream_host = url
            .parse::<http::Uri>()
            .ok()
            .and_then(|u| u.authority().map(|a| a.to_string()));

        // Precompute the anthropic-beta value (Claude only)
        let anthropic_beta_value = if adapter.name() == "Claude" {
            const CLAUDE_CODE_BETA: &str = "claude-code-20250219";
            let mut required = vec![CLAUDE_CODE_BETA];
            if claude_oauth {
                required.push(super::claude_pool::OAUTH_BETA);
            }
            let mut value = headers
                .get("anthropic-beta")
                .and_then(|b| b.to_str().ok())
                .map(str::to_string)
                .unwrap_or_default();
            for beta in required {
                if !value.split(',').any(|b| b.trim() == beta) {
                    value = if value.is_empty() {
                        beta.to_string()
                    } else {
                        format!("{beta},{value}")
                    };
                }
            }
            Some(value)
        } else {
            None
        };

        // ============================================================
        // Build an ordered HeaderMap — replace in place, keeping the client's original order
        // ============================================================
        let mut ordered_headers = http::HeaderMap::new();
        let mut saw_auth = false;
        let mut saw_accept_encoding = false;
        let mut saw_anthropic_beta = false;
        let mut saw_anthropic_version = false;

        for (key, value) in headers {
            let key_str = key.as_str();

            // --- host — replaced in place with the upstream host (keeps the client's original position) ---
            if key_str.eq_ignore_ascii_case("host") {
                if let Some(ref host_val) = upstream_host {
                    if let Ok(hv) = http::HeaderValue::from_str(host_val) {
                        ordered_headers.append(key.clone(), hv);
                    }
                }
                continue;
            }

            if replaces_account_id && key_str.eq_ignore_ascii_case("chatgpt-account-id") {
                continue;
            }

            // --- connection / tracing / CDN headers — always skipped ---
            // content-encoding: the body below is always re-serialized plain JSON,
            // whatever encoding the client sent it in.
            if matches!(
                key_str,
                "content-length"
                    | "content-encoding"
                    | "transfer-encoding"
                    | "x-forwarded-host"
                    | "x-forwarded-port"
                    | "x-forwarded-proto"
                    | "forwarded"
                    | "cf-connecting-ip"
                    | "cf-ipcountry"
                    | "cf-ray"
                    | "cf-visitor"
                    | "true-client-ip"
                    | "fastly-client-ip"
                    | "x-azure-clientip"
                    | "x-azure-fdid"
                    | "x-azure-ref"
                    | "akamai-origin-hop"
                    | "x-akamai-config-log-detail"
                    | "x-request-id"
                    | "x-correlation-id"
                    | "x-trace-id"
                    | "x-amzn-trace-id"
                    | "x-b3-traceid"
                    | "x-b3-spanid"
                    | "x-b3-parentspanid"
                    | "x-b3-sampled"
                    | "traceparent"
                    | "tracestate"
            ) {
                continue;
            }

            // --- auth headers — replaced with the adapter's auth headers (in the original position) ---
            if key_str.eq_ignore_ascii_case("authorization")
                || key_str.eq_ignore_ascii_case("x-api-key")
                || key_str.eq_ignore_ascii_case("x-goog-api-key")
            {
                if !saw_auth {
                    saw_auth = true;
                    for (ah_name, ah_value) in &auth_headers {
                        ordered_headers.append(ah_name.clone(), ah_value.clone());
                    }
                }
                continue;
            }

            // --- accept-encoding — forced to identity on the transform / SSE path, otherwise kept ---
            if key_str.eq_ignore_ascii_case("accept-encoding") {
                if !saw_accept_encoding {
                    saw_accept_encoding = true;
                    if force_identity_encoding {
                        ordered_headers.append(
                            http::header::ACCEPT_ENCODING,
                            http::HeaderValue::from_static("identity"),
                        );
                    } else {
                        ordered_headers.append(key.clone(), value.clone());
                    }
                }
                continue;
            }

            // --- anthropic-beta — replaced with the rebuilt value (always includes the claude-code marker) ---
            if key_str.eq_ignore_ascii_case("anthropic-beta") {
                if !saw_anthropic_beta {
                    saw_anthropic_beta = true;
                    if let Some(ref beta_val) = anthropic_beta_value {
                        if let Ok(hv) = http::HeaderValue::from_str(beta_val) {
                            ordered_headers.append("anthropic-beta", hv);
                        }
                    }
                }
                continue;
            }

            // --- anthropic-version — pass the client's value through ---
            if key_str.eq_ignore_ascii_case("anthropic-version") {
                saw_anthropic_version = true;
                ordered_headers.append(key.clone(), value.clone());
                continue;
            }

            // --- Copilot fingerprint headers — skipped (supplied by auth_headers) ---
            if copilot_fingerprint_headers
                .iter()
                .any(|h| key_str.eq_ignore_ascii_case(h))
            {
                continue;
            }

            // --- default: pass through ---
            ordered_headers.append(key.clone(), value.clone());
        }

        // If the original request had no auth header, append it at the end
        if !saw_auth && !auth_headers.is_empty() {
            for (ah_name, ah_value) in &auth_headers {
                ordered_headers.append(ah_name.clone(), ah_value.clone());
            }
        }

        // The transform / SSE path adds identity when missing; plain passthrough does not add accept-encoding
        if !saw_accept_encoding && force_identity_encoding {
            ordered_headers.append(
                http::header::ACCEPT_ENCODING,
                http::HeaderValue::from_static("identity"),
            );
        }

        // If the original request had no anthropic-beta and there is a value to add, append it
        if !saw_anthropic_beta {
            if let Some(ref beta_val) = anthropic_beta_value {
                if let Ok(hv) = http::HeaderValue::from_str(beta_val) {
                    ordered_headers.append("anthropic-beta", hv);
                }
            }
        }

        // anthropic-version: add the default only when missing
        if adapter.name() == "Claude" && !saw_anthropic_version {
            ordered_headers.append(
                "anthropic-version",
                http::HeaderValue::from_static("2023-06-01"),
            );
        }

        // Serialize the request body
        let body_bytes = serde_json::to_vec(&filtered_body)
            .map_err(|e| ProxyError::Internal(format!("Failed to serialize request body: {e}")))?;

        // Make sure content-type is present
        if !ordered_headers.contains_key(http::header::CONTENT_TYPE) {
            ordered_headers.insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            );
        }

        // Log the request
        let tag = adapter.name();
        let request_model = filtered_body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("<none>");
        log::info!("[{tag}] >>> Request URL: {url} (model={request_model})");
        if let Ok(body_str) = serde_json::to_string(&filtered_body) {
            log::debug!(
                "[{tag}] >>> Request body ({} bytes): {}",
                body_str.len(),
                body_str
            );
        }

        // Determine the timeout
        let timeout = if self.non_streaming_timeout.is_zero() {
            std::time::Duration::from_secs(600) // default 600 seconds
        } else {
            self.non_streaming_timeout
        };

        // Resolve the upstream proxy URL (provider's own proxy > global proxy > none)
        let proxy_config = provider.meta.as_ref().and_then(|m| m.proxy_config.as_ref());
        let upstream_proxy_url: Option<String> = proxy_config
            .filter(|c| c.enabled)
            .and_then(super::http_client::build_proxy_url_from_config)
            .or_else(super::http_client::get_current_proxy_url);

        // SOCKS5 proxies do not support CONNECT tunnels, so reqwest is needed
        let is_socks_proxy = upstream_proxy_url
            .as_deref()
            .map(|u| u.starts_with("socks5"))
            .unwrap_or(false);

        let uri: http::Uri = url
            .parse()
            .map_err(|e| ProxyError::ForwardFailed(format!("Invalid URL '{url}': {e}")))?;

        // Send the request
        let response = if is_socks_proxy {
            // SOCKS5 proxy: reqwest only (header case is not preserved)
            log::debug!("[Forwarder] Using reqwest for SOCKS5 proxy");
            let client = super::http_client::get_for_provider(proxy_config);
            let mut request = client.post(&url);
            if !self.non_streaming_timeout.is_zero() {
                request = request.timeout(self.non_streaming_timeout);
            }
            for (key, value) in &ordered_headers {
                request = request.header(key, value);
            }
            let reqwest_resp = request.body(body_bytes).send().await.map_err(|e| {
                if e.is_timeout() {
                    ProxyError::Timeout(format!("The request timed out: {e}"))
                } else if e.is_connect() {
                    ProxyError::ForwardFailed(format!("Could not connect: {e}"))
                } else {
                    ProxyError::ForwardFailed(e.to_string())
                }
            })?;
            ProxyResponse::Reqwest(reqwest_resp)
        } else {
            // HTTP proxy or direct: hyper raw write (preserves header case)
            // With an HTTP proxy, hyper_client tunnels through it with CONNECT
            super::hyper_client::send_request(
                uri,
                http::Method::POST,
                ordered_headers,
                extensions.clone(),
                body_bytes,
                timeout,
                upstream_proxy_url.as_deref(),
            )
            .await?
        };

        // Check the response status
        let status = response.status();

        // A pooled account reports its quota on every answer, refusals included.
        if claude_oauth {
            let model = Some(request_model).filter(|m| *m != "<none>");
            super::claude_pool::record_quota(&provider.id, model, response.headers());
        } else if adapter.name() == "Codex" && super::codex_pool::is_chatgpt_provider(provider) {
            super::codex_pool::record_quota(&provider.id, response.headers());
        }

        if status.is_success() {
            Ok((response, resolved_claude_api_format))
        } else {
            let status_code = status.as_u16();
            let body_text = String::from_utf8(response.bytes().await?.to_vec()).ok();

            Err(ProxyError::UpstreamError {
                status: status_code,
                body: body_text,
            })
        }
    }

    async fn resolve_claude_api_format(
        &self,
        provider: &Provider,
        body: &Value,
        is_copilot: bool,
    ) -> String {
        if !is_copilot {
            return super::providers::get_claude_api_format(provider).to_string();
        }

        let model = body.get("model").and_then(|value| value.as_str());
        if let Some(model_id) = model {
            if self
                .is_copilot_openai_vendor_model(provider, model_id)
                .await
            {
                return "openai_responses".to_string();
            }
        }

        "openai_chat".to_string()
    }

    async fn is_copilot_openai_vendor_model(&self, provider: &Provider, model_id: &str) -> bool {
        let Some(app_handle) = &self.app_handle else {
            log::debug!("[Copilot] AppHandle unavailable, fallback to chat/completions");
            return false;
        };

        let copilot_state = app_handle.state::<CopilotAuthState>();
        let copilot_auth = copilot_state.0.read().await;
        let account_id = provider
            .meta
            .as_ref()
            .and_then(|m| m.managed_account_id_for("github_copilot"));

        let vendor_result = match account_id.as_deref() {
            Some(id) => {
                copilot_auth
                    .get_model_vendor_for_account(id, model_id)
                    .await
            }
            None => copilot_auth.get_model_vendor(model_id).await,
        };

        match vendor_result {
            Ok(Some(vendor)) => vendor.eq_ignore_ascii_case("openai"),
            Ok(None) => {
                log::debug!(
                    "[Copilot] Model vendor unavailable for {model_id}, fallback to chat/completions"
                );
                false
            }
            Err(err) => {
                log::warn!(
                    "[Copilot] Failed to resolve model vendor for {model_id}, fallback to chat/completions: {err}"
                );
                false
            }
        }
    }

    fn categorize_proxy_error(&self, error: &ProxyError) -> ErrorCategory {
        match error {
            // Network and upstream errors: always try the next provider
            ProxyError::Timeout(_) => ErrorCategory::Retryable,
            ProxyError::ForwardFailed(_) => ErrorCategory::Retryable,
            ProxyError::ProviderUnhealthy(_) => ErrorCategory::Retryable,
            // Upstream HTTP error: try the next provider whatever the status code
            // Reason: providers have different limits and auth, so a 4xx from one provider
            // does not mean the others will fail
            ProxyError::UpstreamError { .. } => ErrorCategory::Retryable,
            // Provider-level config/transform problem: another provider may well succeed
            ProxyError::ConfigError(_) => ErrorCategory::Retryable,
            ProxyError::TransformError(_) => ErrorCategory::Retryable,
            ProxyError::AuthError(_) => ErrorCategory::Retryable,
            ProxyError::StreamIdleTimeout(_) => ErrorCategory::Retryable,
            // No available provider: every provider has been tried; cannot retry
            ProxyError::NoAvailableProvider => ErrorCategory::NonRetryable,
            // Other errors (database/internal, etc.): switching provider will not fix them
            _ => ErrorCategory::NonRetryable,
        }
    }
}

/// Extracts the error message from a ProxyError
fn extract_error_message(error: &ProxyError) -> Option<String> {
    match error {
        ProxyError::UpstreamError { body, .. } => body.clone(),
        _ => Some(error.to_string()),
    }
}

/// Whether the provider is Bedrock (via the CLAUDE_CODE_USE_BEDROCK environment variable)
fn is_bedrock_provider(provider: &Provider) -> bool {
    provider
        .settings_config
        .get("env")
        .and_then(|e| e.get("CLAUDE_CODE_USE_BEDROCK"))
        .and_then(|v| v.as_str())
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Whether a failed request says something about the provider itself, so it
/// counts toward its health and circuit breaker: a refused login, a server
/// error, a timeout or a dropped connection. Errors the request caused (400,
/// 404, 413, ...) and rate limits (429, which also hit every account at once
/// when the request itself is too large) still move on to the next provider
/// but leave this one's health alone.
fn counts_against_provider(error: &ProxyError) -> bool {
    match error {
        ProxyError::Timeout(_)
        | ProxyError::ForwardFailed(_)
        | ProxyError::StreamIdleTimeout(_)
        | ProxyError::ProviderUnhealthy(_)
        | ProxyError::AuthError(_) => true,
        ProxyError::UpstreamError { status, .. } => {
            *status >= 500 || *status == 401 || *status == 403
        }
        _ => false,
    }
}

/// Why a failed request moved the pool off `provider`, for the switch history.
fn switch_reason_for(provider: &Provider, error: &ProxyError) -> (SwitchReason, String) {
    let summary = summarize_proxy_error(error);
    let signed_out =
        super::codex_pool::needs_sign_in(provider) || super::claude_pool::needs_sign_in(provider);
    let reason = if signed_out {
        SwitchReason::SignedOut
    } else if matches!(error, ProxyError::UpstreamError { status: 429, .. }) {
        SwitchReason::Limit
    } else {
        SwitchReason::Failover
    };
    (reason, summary)
}

fn build_retryable_failure_log(
    provider_name: &str,
    attempted_providers: usize,
    total_providers: usize,
    error: &ProxyError,
) -> (&'static str, String) {
    let error_summary = summarize_proxy_error(error);

    if total_providers <= 1 {
        (
            log_fwd::SINGLE_PROVIDER_FAILED,
            format!("Provider {provider_name} request failed: {error_summary}"),
        )
    } else {
        (
            log_fwd::PROVIDER_FAILED_RETRY,
            format!(
                "Provider {provider_name} failed, trying the next one ({attempted_providers}/{total_providers}): {error_summary}"
            ),
        )
    }
}

fn build_terminal_failure_log(
    attempted_providers: usize,
    total_providers: usize,
    last_error: Option<&ProxyError>,
) -> Option<(&'static str, String)> {
    if total_providers <= 1 {
        return None;
    }

    let error_summary = last_error
        .map(summarize_proxy_error)
        .unwrap_or_else(|| "unknown error".to_string());

    Some((
        log_fwd::ALL_PROVIDERS_FAILED,
        format!(
            "Tried {attempted_providers}/{total_providers} providers, all failed. Last error: {error_summary}"
        ),
    ))
}

fn summarize_proxy_error(error: &ProxyError) -> String {
    match error {
        ProxyError::UpstreamError { status, body } => {
            let body_summary = body
                .as_deref()
                .map(summarize_upstream_body)
                .filter(|summary| !summary.is_empty());

            match body_summary {
                Some(summary) => format!("upstream HTTP {status}: {summary}"),
                None => format!("upstream HTTP {status}"),
            }
        }
        ProxyError::Timeout(message) => {
            format!(
                "request timed out: {}",
                summarize_text_for_log(message, 180)
            )
        }
        ProxyError::ForwardFailed(message) => {
            format!(
                "request forwarding failed: {}",
                summarize_text_for_log(message, 180)
            )
        }
        ProxyError::TransformError(message) => {
            format!(
                "response conversion failed: {}",
                summarize_text_for_log(message, 180)
            )
        }
        ProxyError::ConfigError(message) => {
            format!(
                "configuration error: {}",
                summarize_text_for_log(message, 180)
            )
        }
        ProxyError::AuthError(message) => {
            format!(
                "authentication failed: {}",
                summarize_text_for_log(message, 180)
            )
        }
        _ => summarize_text_for_log(&error.to_string(), 180),
    }
}

fn summarize_upstream_body(body: &str) -> String {
    if let Ok(json_body) = serde_json::from_str::<Value>(body) {
        if let Some(message) = extract_json_error_message(&json_body) {
            return summarize_text_for_log(&message, 180);
        }

        if let Ok(compact_json) = serde_json::to_string(&json_body) {
            return summarize_text_for_log(&compact_json, 180);
        }
    }

    summarize_text_for_log(body, 180)
}

fn extract_json_error_message(body: &Value) -> Option<String> {
    let candidates = [
        body.pointer("/error/message"),
        body.pointer("/message"),
        body.pointer("/detail"),
        body.pointer("/error"),
    ];

    candidates
        .into_iter()
        .flatten()
        .find_map(|value| value.as_str().map(ToString::to_string))
}

fn split_endpoint_and_query(endpoint: &str) -> (&str, Option<&str>) {
    endpoint
        .split_once('?')
        .map_or((endpoint, None), |(path, query)| (path, Some(query)))
}

fn strip_beta_query(query: Option<&str>) -> Option<String> {
    let filtered = query.map(|query| {
        query
            .split('&')
            .filter(|pair| !pair.is_empty() && !pair.starts_with("beta="))
            .collect::<Vec<_>>()
            .join("&")
    });

    match filtered.as_deref() {
        Some("") | None => None,
        Some(_) => filtered,
    }
}

fn is_claude_messages_path(path: &str) -> bool {
    matches!(path, "/v1/messages" | "/claude/v1/messages")
}

fn rewrite_claude_transform_endpoint(
    endpoint: &str,
    api_format: &str,
    is_copilot: bool,
) -> (String, Option<String>) {
    let (path, query) = split_endpoint_and_query(endpoint);
    let passthrough_query = if is_claude_messages_path(path) {
        strip_beta_query(query)
    } else {
        query.map(ToString::to_string)
    };

    if !is_claude_messages_path(path) {
        return (endpoint.to_string(), passthrough_query);
    }

    let target_path = if is_copilot && api_format == "openai_responses" {
        "/v1/responses"
    } else if is_copilot {
        "/chat/completions"
    } else if api_format == "openai_responses" {
        "/v1/responses"
    } else {
        "/v1/chat/completions"
    };

    let rewritten = match passthrough_query.as_deref() {
        Some(query) if !query.is_empty() => format!("{target_path}?{query}"),
        _ => target_path.to_string(),
    };

    (rewritten, passthrough_query)
}

fn append_query_to_full_url(base_url: &str, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => {
            if base_url.contains('?') {
                format!("{base_url}&{query}")
            } else {
                format!("{base_url}?{query}")
            }
        }
        _ => base_url.to_string(),
    }
}

fn should_force_identity_encoding(
    endpoint: &str,
    body: &Value,
    headers: &axum::http::HeaderMap,
) -> bool {
    if body
        .get("stream")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return true;
    }

    if endpoint.contains("streamGenerateContent") || endpoint.contains("alt=sse") {
        return true;
    }

    headers
        .get(axum::http::header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| accept.contains("text/event-stream"))
        .unwrap_or(false)
}

fn summarize_text_for_log(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();

    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let truncated: String = trimmed.chars().take(max_chars).collect();
    let truncated = truncated.trim_end();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_about_the_account_count_against_it() {
        let upstream = |status| ProxyError::UpstreamError { status, body: None };
        assert!(counts_against_provider(&upstream(401)));
        assert!(counts_against_provider(&upstream(403)));
        assert!(counts_against_provider(&upstream(503)));
        assert!(counts_against_provider(&ProxyError::Timeout("t".into())));
        assert!(
            !counts_against_provider(&upstream(400)),
            "a request that is too long"
        );
        assert!(!counts_against_provider(&upstream(413)));
        assert!(
            !counts_against_provider(&upstream(429)),
            "rate limits hit every account"
        );
    }
    use axum::http::header::{HeaderValue, ACCEPT};
    use axum::http::HeaderMap;
    use serde_json::json;

    #[test]
    fn single_provider_retryable_log_uses_single_provider_code() {
        let error = ProxyError::UpstreamError {
            status: 429,
            body: Some(r#"{"error":{"message":"rate limit exceeded"}}"#.to_string()),
        };

        let (code, message) = build_retryable_failure_log("PackyCode-response", 1, 1, &error);

        assert_eq!(code, log_fwd::SINGLE_PROVIDER_FAILED);
        assert!(message.contains("Provider PackyCode-response request failed"));
        assert!(message.contains("upstream HTTP 429"));
        assert!(message.contains("rate limit exceeded"));
        assert!(!message.contains("trying the next one"));
    }

    #[test]
    fn multi_provider_retryable_log_keeps_failover_wording() {
        let error = ProxyError::Timeout("upstream timed out after 30s".to_string());

        let (code, message) = build_retryable_failure_log("primary", 1, 3, &error);

        assert_eq!(code, log_fwd::PROVIDER_FAILED_RETRY);
        assert!(message.contains("trying the next one (1/3)"));
        assert!(message.contains("request timed out"));
    }

    #[test]
    fn single_provider_has_no_terminal_all_failed_log() {
        assert!(build_terminal_failure_log(1, 1, None).is_none());
    }

    #[test]
    fn multi_provider_terminal_log_contains_last_error_summary() {
        let error = ProxyError::ForwardFailed("connection reset by peer".to_string());

        let (code, message) =
            build_terminal_failure_log(2, 2, Some(&error)).expect("expected terminal log");

        assert_eq!(code, log_fwd::ALL_PROVIDERS_FAILED);
        assert!(message.contains("Tried 2/2 providers, all failed"));
        assert!(message.contains("connection reset by peer"));
    }

    #[test]
    fn summarize_upstream_body_prefers_json_message() {
        let body = json!({
            "error": {
                "message": "invalid_request_error: unsupported field"
            },
            "request_id": "req_123"
        });

        let summary = summarize_upstream_body(&body.to_string());

        assert_eq!(summary, "invalid_request_error: unsupported field");
    }

    #[test]
    fn summarize_text_for_log_collapses_whitespace_and_truncates() {
        let summary = summarize_text_for_log("line1\n\n line2   line3", 12);

        assert_eq!(summary, "line1 line2...");
    }

    #[test]
    fn rewrite_claude_transform_endpoint_strips_beta_for_chat_completions() {
        let (endpoint, passthrough_query) = rewrite_claude_transform_endpoint(
            "/v1/messages?beta=true&foo=bar",
            "openai_chat",
            false,
        );

        assert_eq!(endpoint, "/v1/chat/completions?foo=bar");
        assert_eq!(passthrough_query.as_deref(), Some("foo=bar"));
    }

    #[test]
    fn rewrite_claude_transform_endpoint_strips_beta_for_responses() {
        let (endpoint, passthrough_query) = rewrite_claude_transform_endpoint(
            "/claude/v1/messages?beta=true&x-id=1",
            "openai_responses",
            false,
        );

        assert_eq!(endpoint, "/v1/responses?x-id=1");
        assert_eq!(passthrough_query.as_deref(), Some("x-id=1"));
    }

    #[test]
    fn rewrite_claude_transform_endpoint_uses_copilot_path() {
        let (endpoint, passthrough_query) =
            rewrite_claude_transform_endpoint("/v1/messages?beta=true&x-id=1", "anthropic", true);

        assert_eq!(endpoint, "/chat/completions?x-id=1");
        assert_eq!(passthrough_query.as_deref(), Some("x-id=1"));
    }

    #[test]
    fn rewrite_claude_transform_endpoint_uses_copilot_responses_path() {
        let (endpoint, passthrough_query) = rewrite_claude_transform_endpoint(
            "/v1/messages?beta=true&x-id=1",
            "openai_responses",
            true,
        );

        assert_eq!(endpoint, "/v1/responses?x-id=1");
        assert_eq!(passthrough_query.as_deref(), Some("x-id=1"));
    }

    #[test]
    fn append_query_to_full_url_preserves_existing_query_string() {
        let url = append_query_to_full_url("https://relay.example/api?foo=bar", Some("x-id=1"));

        assert_eq!(url, "https://relay.example/api?foo=bar&x-id=1");
    }

    #[test]
    fn force_identity_for_stream_flag_requests() {
        let headers = HeaderMap::new();

        assert!(should_force_identity_encoding(
            "/v1/responses",
            &json!({ "stream": true }),
            &headers
        ));
    }

    #[test]
    fn force_identity_for_gemini_stream_endpoints() {
        let headers = HeaderMap::new();

        assert!(should_force_identity_encoding(
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse",
            &json!({ "model": "gemini-2.5-pro" }),
            &headers
        ));
    }

    #[test]
    fn force_identity_for_sse_accept_header() {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));

        assert!(should_force_identity_encoding(
            "/v1/responses",
            &json!({ "model": "gpt-5" }),
            &headers
        ));
    }

    #[test]
    fn non_streaming_requests_allow_automatic_compression() {
        let headers = HeaderMap::new();

        assert!(!should_force_identity_encoding(
            "/v1/responses",
            &json!({ "model": "gpt-5" }),
            &headers
        ));
    }

    // ==================== Copilot dynamic endpoint routing tests ====================

    /// Checks is_copilot detection via provider_type
    #[test]
    fn copilot_detection_via_provider_type() {
        use crate::provider::{Provider, ProviderMeta};

        let provider = Provider {
            id: "test".to_string(),
            name: "Test Copilot".to_string(),
            settings_config: serde_json::json!({}),
            website_url: None,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: Some(ProviderMeta {
                provider_type: Some("github_copilot".to_string()),
                ..Default::default()
            }),
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        };

        let is_copilot = provider
            .meta
            .as_ref()
            .and_then(|m| m.provider_type.as_deref())
            == Some("github_copilot");

        assert!(
            is_copilot,
            "should be detected as Copilot via provider_type"
        );
    }

    /// Checks is_copilot detection via base_url
    #[test]
    fn copilot_detection_via_base_url() {
        let base_url = "https://api.githubcopilot.com";
        let is_copilot = base_url.contains("githubcopilot.com");
        assert!(is_copilot, "should be detected as Copilot via base_url");

        let non_copilot_url = "https://api.anthropic.com";
        let is_not_copilot = non_copilot_url.contains("githubcopilot.com");
        assert!(
            !is_not_copilot,
            "a non-Copilot URL must not be detected as Copilot"
        );
    }

    /// Checks is_copilot is still correct for an enterprise endpoint (without githubcopilot.com)
    #[test]
    fn copilot_detection_for_enterprise_endpoint() {
        use crate::provider::{Provider, ProviderMeta};

        // Enterprise case: provider_type is github_copilot but base_url may be an internal corporate domain
        let provider = Provider {
            id: "enterprise".to_string(),
            name: "Enterprise Copilot".to_string(),
            settings_config: serde_json::json!({}),
            website_url: None,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: Some(ProviderMeta {
                provider_type: Some("github_copilot".to_string()),
                ..Default::default()
            }),
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        };

        let enterprise_base_url = "https://copilot-api.corp.example.com";

        // is_copilot must be detected via provider_type even when base_url lacks githubcopilot.com
        let is_copilot = provider
            .meta
            .as_ref()
            .and_then(|m| m.provider_type.as_deref())
            == Some("github_copilot")
            || enterprise_base_url.contains("githubcopilot.com");

        assert!(
            is_copilot,
            "enterprise Copilot should be detected via provider_type"
        );
    }

    /// Checks the dynamic endpoint replacement condition
    #[test]
    fn dynamic_endpoint_replacement_conditions() {
        // Condition: is_copilot && !is_full_url
        let test_cases = [
            (true, false, true, "Copilot + not full_url should replace"),
            (true, true, false, "Copilot + full_url should not replace"),
            (false, false, false, "non-Copilot should not replace"),
            (
                false,
                true,
                false,
                "non-Copilot + full_url should not replace",
            ),
        ];

        for (is_copilot, is_full_url, should_replace, desc) in test_cases {
            let will_replace = is_copilot && !is_full_url;
            assert_eq!(will_replace, should_replace, "{desc}");
        }
    }
}
