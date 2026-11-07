//! Global outbound proxy commands
//!
//! Tauri commands to get, set and test the global proxy.

use crate::proxy::http_client;
use crate::store::AppState;
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

/// Get the global proxy URL
///
/// Returns the configured proxy URL; null means a direct connection.
#[tauri::command]
pub fn get_global_proxy_url(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    let result = state.db.get_global_proxy_url().map_err(|e| e.to_string())?;
    log::debug!(
        "[GlobalProxy] [GP-010] Read from database: {}",
        result
            .as_ref()
            .map(|u| http_client::mask_url(u))
            .unwrap_or_else(|| "None".to_string())
    );
    Ok(result)
}

/// Set the global proxy URL
///
/// - Non-empty string: enable the proxy
/// - Empty string: clear the proxy (direct connection)
///
/// Order: validate -> write the DB -> apply
/// so a failed DB write never leaves the runtime state out of step with what is persisted
#[tauri::command]
pub fn set_global_proxy_url(state: tauri::State<'_, AppState>, url: String) -> Result<(), String> {
    // Debug: log what the received URL looks like (nothing sensitive)
    let has_auth = url.contains('@') && (url.starts_with("http://") || url.starts_with("socks"));
    log::debug!(
        "[GlobalProxy] [GP-011] Received URL: length={}, has_auth={}",
        url.len(),
        has_auth
    );

    let url_opt = if url.trim().is_empty() {
        None
    } else {
        Some(url.as_str())
    };

    // 1. Validate the proxy config first (without applying it)
    http_client::validate_proxy(url_opt)?;

    // 2. Once valid, save it to the database
    state
        .db
        .set_global_proxy_url(url_opt)
        .map_err(|e| e.to_string())?;

    // 3. Apply it to the runtime only after the DB write succeeds
    http_client::apply_proxy(url_opt)?;

    log::info!(
        "[GlobalProxy] [GP-009] Configuration updated: {}",
        url_opt
            .map(http_client::mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );

    Ok(())
}

/// Proxy test result
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    /// Whether the connection succeeded
    pub success: bool,
    /// Latency (ms)
    pub latency_ms: u64,
    /// Error message
    pub error: Option<String>,
}

/// Test the proxy connection
///
/// Sends a test request through the given proxy URL and returns the result and latency.
/// Tries several test targets; the proxy counts as working if any of them succeeds.
#[tauri::command]
pub async fn test_proxy_url(url: String) -> Result<ProxyTestResult, String> {
    if url.trim().is_empty() {
        return Err("Proxy URL is empty".to_string());
    }

    let start = Instant::now();

    // Build a temporary client that uses the proxy
    let proxy = reqwest::Proxy::all(&url).map_err(|e| format!("Invalid proxy URL: {e}"))?;

    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build client: {e}"))?;

    // Several test targets for better compatibility
    // Prefer httpbin (built for HTTP testing), then fall back to other public endpoints
    let test_urls = [
        "https://httpbin.org/get",
        "https://www.google.com",
        "https://api.anthropic.com",
    ];

    let mut last_error = None;

    for test_url in test_urls {
        match client.head(test_url).send().await {
            Ok(resp) => {
                let latency = start.elapsed().as_millis() as u64;
                log::debug!(
                    "[GlobalProxy] Test successful: {} -> {} via {} ({}ms)",
                    http_client::mask_url(&url),
                    test_url,
                    resp.status(),
                    latency
                );
                return Ok(ProxyTestResult {
                    success: true,
                    latency_ms: latency,
                    error: None,
                });
            }
            Err(e) => {
                log::debug!("[GlobalProxy] Test to {test_url} failed: {e}");
                last_error = Some(e);
            }
        }
    }

    // Every test target failed
    let latency = start.elapsed().as_millis() as u64;
    let error_msg = last_error
        .map(|e| e.to_string())
        .unwrap_or_else(|| "All test targets failed".to_string());

    log::debug!(
        "[GlobalProxy] Test failed: {} -> {} ({}ms)",
        http_client::mask_url(&url),
        error_msg,
        latency
    );

    Ok(ProxyTestResult {
        success: false,
        latency_ms: latency,
        error: Some(error_msg),
    })
}

/// Get the current outbound proxy status
///
/// Returns whether an outbound proxy is enabled and its URL.
#[tauri::command]
pub fn get_upstream_proxy_status() -> UpstreamProxyStatus {
    let url = http_client::get_current_proxy_url();
    UpstreamProxyStatus {
        enabled: url.is_some(),
        proxy_url: url,
    }
}

/// Outbound proxy status
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamProxyStatus {
    /// Whether the proxy is enabled
    pub enabled: bool,
    /// Proxy URL
    pub proxy_url: Option<String>,
}

/// A detected proxy
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedProxy {
    /// Proxy URL
    pub url: String,
    /// Proxy type (http/socks5)
    pub proxy_type: String,
    /// Port
    pub port: u16,
}

/// Common proxy ports
/// Format: (port, primary type, whether it serves both http and socks5)
/// For mixed ports both protocols are returned so the user can choose
const PROXY_PORTS: &[(u16, &str, bool)] = &[
    (7890, "http", true),     // Clash (mixed mode)
    (7891, "socks5", false),  // Clash SOCKS only
    (1080, "socks5", false),  // Generic SOCKS5
    (8080, "http", false),    // Generic HTTP
    (8888, "http", false),    // Charles/Fiddler
    (3128, "http", false),    // Squid
    (10808, "socks5", false), // V2Ray SOCKS
    (10809, "http", false),   // V2Ray HTTP
];

/// Scan for local proxies
///
/// Checks whether a proxy service is running on common ports.
/// Runs as a blocking task so it does not block the UI thread.
#[tauri::command]
pub async fn scan_local_proxies() -> Vec<DetectedProxy> {
    // spawn_blocking keeps the main thread free
    tokio::task::spawn_blocking(|| {
        let mut found = Vec::new();

        for &(port, primary_type, is_mixed) in PROXY_PORTS {
            let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
            if TcpStream::connect_timeout(&addr.into(), Duration::from_millis(100)).is_ok() {
                // Add the primary type
                found.push(DetectedProxy {
                    url: format!("{primary_type}://127.0.0.1:{port}"),
                    proxy_type: primary_type.to_string(),
                    port,
                });
                // For mixed ports, also add the other protocol
                if is_mixed {
                    let alt_type = if primary_type == "http" {
                        "socks5"
                    } else {
                        "http"
                    };
                    found.push(DetectedProxy {
                        url: format!("{alt_type}://127.0.0.1:{port}"),
                        proxy_type: alt_type.to_string(),
                        port,
                    });
                }
            }
        }

        found
    })
    .await
    .unwrap_or_default()
}
