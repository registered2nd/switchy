//! HTTP proxy server
//!
//! Axum-based HTTP server that handles proxy requests
//!
//! Uses a manual hyper HTTP/1.1 accept loop with `preserve_header_case(true)` so
//! that the original header-name casing from the CLI client is captured in a
//! `HeaderCaseMap` extension.  This map is later forwarded to the upstream via
//! the hyper-based HTTP client, producing wire-level header casing identical to
//! a direct (non-proxied) CLI request.

use super::{
    failover_switch::FailoverSwitchManager, handlers, log_codes::srv as log_srv,
    provider_router::ProviderRouter, types::*, ProxyError,
};
use crate::database::Database;
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};
use tokio::task::JoinHandle;
use tower_http::cors::{Any, CorsLayer};

/// Proxy server state (shared)
#[derive(Clone)]
pub struct ProxyState {
    pub db: Arc<Database>,
    pub config: Arc<RwLock<ProxyConfig>>,
    pub status: Arc<RwLock<ProxyStatus>>,
    pub start_time: Arc<RwLock<Option<std::time::Instant>>>,
    /// Provider currently in use per app type (app_type -> (provider_id, provider_name))
    pub current_providers: Arc<RwLock<std::collections::HashMap<String, (String, String)>>>,
    /// Shared ProviderRouter (holds circuit breaker state across requests)
    pub provider_router: Arc<ProviderRouter>,
    /// AppHandle, for emitting events and updating the tray menu
    pub app_handle: Option<tauri::AppHandle>,
    /// Failover switch manager
    pub failover_manager: Arc<FailoverSwitchManager>,
}

/// Proxy HTTP server
pub struct ProxyServer {
    config: ProxyConfig,
    state: ProxyState,
    shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
    /// Server task handle, used to wait for the server to actually shut down
    server_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl ProxyServer {
    pub fn new(
        config: ProxyConfig,
        db: Arc<Database>,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        // Create the shared ProviderRouter (circuit breaker state persists across all requests)
        let provider_router = Arc::new(ProviderRouter::new(db.clone()));
        // Create the failover switch manager
        let failover_manager = Arc::new(FailoverSwitchManager::new(db.clone()));

        let state = ProxyState {
            db,
            config: Arc::new(RwLock::new(config.clone())),
            status: Arc::new(RwLock::new(ProxyStatus::default())),
            start_time: Arc::new(RwLock::new(None)),
            current_providers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            provider_router,
            app_handle,
            failover_manager,
        };

        Self {
            config,
            state,
            shutdown_tx: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self) -> Result<ProxyServerInfo, ProxyError> {
        // Check whether it is already running
        if self.shutdown_tx.read().await.is_some() {
            return Err(ProxyError::AlreadyRunning);
        }

        let addr: SocketAddr =
            format!("{}:{}", self.config.listen_address, self.config.listen_port)
                .parse()
                .map_err(|e| ProxyError::BindFailed(format!("Invalid address: {e}")))?;

        // Create the shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        // Build the router
        let app = self.build_router();

        // Bind the listener
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| ProxyError::BindFailed(e.to_string()))?;

        log::info!("[{}] Proxy server started on {addr}", log_srv::STARTED);

        // Update the global proxy port, used for system proxy detection
        crate::proxy::http_client::set_proxy_port(self.config.listen_port);

        // Save the shutdown handle
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        // Update status
        let mut status = self.state.status.write().await;
        status.running = true;
        status.address = self.config.listen_address.clone();
        status.port = self.config.listen_port;
        drop(status);

        // Record the start time
        *self.state.start_time.write().await = Some(std::time::Instant::now());

        // Start the server with a manual hyper HTTP/1.1 accept loop
        // preserve_header_case keeps the client's original header casing
        let state = self.state.clone();
        let handle = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _remote_addr) = match result {
                            Ok(v) => v,
                            Err(e) => {
                                log::error!("[{SRV}] accept failed: {e}", SRV = log_srv::ACCEPT_ERR);
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                continue;
                            }
                        };

                        let app = app.clone();
                        tokio::spawn(async move {
                            // Peek raw TCP bytes to capture original header casing
                            // before hyper parses (and lowercases) the header names.
                            let original_cases = {
                                let mut peek_buf = vec![0u8; 8192];
                                match stream.peek(&mut peek_buf).await {
                                    Ok(n) => {
                                        let cases = super::hyper_client::OriginalHeaderCases::from_raw_bytes(&peek_buf[..n]);
                                        log::debug!(
                                            "[ProxyServer] Peeked {} bytes, captured {} header casings",
                                            n, cases.cases.len()
                                        );
                                        cases
                                    }
                                    Err(e) => {
                                        log::debug!("[ProxyServer] peek failed (non-fatal): {e}");
                                        super::hyper_client::OriginalHeaderCases::default()
                                    }
                                }
                            };

                            // service_fn bridges the axum Router (tower::Service) to hyper
                            let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                                let mut router = app.clone();
                                let cases = original_cases.clone();
                                async move {
                                    // Convert hyper::body::Incoming to axum::body::Body, keeping extensions
                                    let (mut parts, body) = req.into_parts();

                                    // Insert our own header case map alongside hyper's internal one
                                    parts.extensions.insert(cases);

                                    let body = axum::body::Body::new(body);
                                    let axum_req = http::Request::from_parts(parts, body);
                                    <Router as tower::Service<http::Request<axum::body::Body>>>::call(&mut router, axum_req).await
                                }
                            });

                            if let Err(e) = hyper::server::conn::http1::Builder::new()
                                .preserve_header_case(true)
                                .serve_connection(TokioIo::new(stream), service)
                                .await
                            {
                                // Connection reset / broken pipe etc. are common when proxying; log at debug
                                log::debug!("[{SRV}] connection error: {e}", SRV = log_srv::CONN_ERR);
                            }
                        });
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }

            // Update status once the server stops
            state.status.write().await.running = false;
            *state.start_time.write().await = None;
        });

        // Save the server task handle
        *self.server_handle.write().await = Some(handle);

        Ok(ProxyServerInfo {
            address: self.config.listen_address.clone(),
            port: self.config.listen_port,
            started_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn stop(&self) -> Result<(), ProxyError> {
        // 1. Send the shutdown signal
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        } else {
            return Err(ProxyError::NotRunning);
        }

        // 2. Wait for the server task to finish (5s timeout)
        if let Some(handle) = self.server_handle.write().await.take() {
            match tokio::time::timeout(std::time::Duration::from_secs(5), handle).await {
                Ok(Ok(())) => {
                    log::info!("[{}] Proxy server fully stopped", log_srv::STOPPED);
                    Ok(())
                }
                Ok(Err(e)) => {
                    log::warn!(
                        "[{}] Proxy server task terminated abnormally: {e}",
                        log_srv::TASK_ERROR
                    );
                    Err(ProxyError::StopFailed(e.to_string()))
                }
                Err(_) => {
                    log::warn!(
                        "[{}] Proxy server stop timed out (5s), continuing anyway",
                        log_srv::STOP_TIMEOUT
                    );
                    Err(ProxyError::StopTimeout)
                }
            }
        } else {
            Ok(())
        }
    }

    pub async fn get_status(&self) -> ProxyStatus {
        let mut status = self.state.status.read().await.clone();

        // Compute uptime
        if let Some(start) = *self.state.start_time.read().await {
            status.uptime_seconds = start.elapsed().as_secs();
        }

        // Get each app type's current provider from the current_providers HashMap
        let current_providers = self.state.current_providers.read().await;
        status.active_targets = current_providers
            .iter()
            .map(|(app_type, (provider_id, provider_name))| ActiveTarget {
                app_type: app_type.clone(),
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
            })
            .collect();

        status
    }

    /// Update an app type's current target provider (for the UI's active_targets)
    ///
    /// This does not mean the provider has handled a request yet; it lets the UI reflect the latest target right away
    /// in cases like a hot switch, or enabling failover and switching to P1 immediately.
    pub async fn set_active_target(&self, app_type: &str, provider_id: &str, provider_name: &str) {
        let mut current_providers = self.state.current_providers.write().await;
        current_providers.insert(
            app_type.to_string(),
            (provider_id.to_string(), provider_name.to_string()),
        );
    }

    fn build_router(&self) -> Router {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);

        Router::new()
            // Health check
            .route("/health", get(handlers::health_check))
            .route("/status", get(handlers::get_status))
            // Claude API (with or without prefix)
            .route("/v1/messages", post(handlers::handle_messages))
            .route("/claude/v1/messages", post(handlers::handle_messages))
            // OpenAI Chat Completions API (Codex CLI, with or without prefix)
            .route("/chat/completions", post(handlers::handle_chat_completions))
            .route(
                "/v1/chat/completions",
                post(handlers::handle_chat_completions),
            )
            .route(
                "/v1/v1/chat/completions",
                post(handlers::handle_chat_completions),
            )
            .route(
                "/codex/v1/chat/completions",
                post(handlers::handle_chat_completions),
            )
            // OpenAI Responses API (Codex CLI, with or without prefix)
            .route("/responses", post(handlers::handle_responses))
            .route("/v1/responses", post(handlers::handle_responses))
            .route("/v1/v1/responses", post(handlers::handle_responses))
            .route("/codex/v1/responses", post(handlers::handle_responses))
            // OpenAI Responses Compact API (Codex CLI remote compaction, passthrough)
            .route(
                "/responses/compact",
                post(handlers::handle_responses_compact),
            )
            .route(
                "/v1/responses/compact",
                post(handlers::handle_responses_compact),
            )
            .route(
                "/v1/v1/responses/compact",
                post(handlers::handle_responses_compact),
            )
            .route(
                "/codex/v1/responses/compact",
                post(handlers::handle_responses_compact),
            )
            // Codex in ChatGPT mode, its built-in provider pointed here with
            // `openai_base_url`. Fixed routes win over the wildcard.
            .route(
                "/backend-api/codex/responses",
                post(handlers::handle_responses).get(handlers::handle_codex_websocket_refusal),
            )
            .route(
                "/backend-api/codex/responses/compact",
                post(handlers::handle_responses_compact),
            )
            .route(
                "/backend-api/codex/*path",
                get(handlers::handle_codex_backend_get),
            )
            // Gemini API (with or without prefix)
            .route("/v1beta/*path", post(handlers::handle_gemini))
            .route("/gemini/v1beta/*path", post(handlers::handle_gemini))
            // Claude Code's identity-plane calls while an Official account is
            // served through the proxy; a 404 otherwise, as before.
            .fallback(handlers::handle_claude_passthrough)
            // Raise the default request body size limit (avoids 413 Payload Too Large)
            .layer(DefaultBodyLimit::max(200 * 1024 * 1024))
            .layer(cors)
            .with_state(self.state.clone())
    }

    /// Update runtime config without restarting the server
    pub async fn apply_runtime_config(&self, config: &ProxyConfig) {
        *self.state.config.write().await = config.clone();
    }

    /// Hot-reload circuit breaker config
    ///
    /// Apply the new config to every existing circuit breaker
    pub async fn update_circuit_breaker_configs(
        &self,
        config: super::circuit_breaker::CircuitBreakerConfig,
    ) {
        self.state.provider_router.update_all_configs(config).await;
    }

    /// Reset the circuit breaker for the given provider
    pub async fn reset_provider_circuit_breaker(&self, provider_id: &str, app_type: &str) {
        self.state
            .provider_router
            .reset_provider_breaker(provider_id, app_type)
            .await;
    }
}
