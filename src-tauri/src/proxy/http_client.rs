//! Global HTTP client
//!
//! Provides an HTTP client that honours the global proxy setting.
//! Every module that sends HTTP requests should use the client from this module.

use crate::provider::ProviderProxyConfig;
use once_cell::sync::OnceCell;
use reqwest::Client;
use std::env;
use std::net::IpAddr;
use std::sync::RwLock;
use std::time::Duration;

/// rustls 0.23 will not choose a process-wide crypto provider when both
/// `ring` (hyper-rustls) and `aws-lc-rs` (reqwest) are linked. The first
/// HTTPS call then panics, and the proxy drops the client connection.
pub fn install_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Global HTTP client instance
static GLOBAL_CLIENT: OnceCell<RwLock<Client>> = OnceCell::new();

/// Current proxy URL (for logging and status)
static CURRENT_PROXY_URL: OnceCell<RwLock<Option<String>>> = OnceCell::new();

/// Port the Switchy proxy server is listening on
static SWITCHY_PROXY_PORT: OnceCell<RwLock<u16>> = OnceCell::new();

/// Sets the Switchy proxy server's listening port
///
/// Call when the proxy server starts so system proxy detection can recognise its own port
pub fn set_proxy_port(port: u16) {
    if let Some(lock) = SWITCHY_PROXY_PORT.get() {
        if let Ok(mut current_port) = lock.write() {
            *current_port = port;
            log::debug!("[GlobalProxy] Updated Switchy proxy port to {port}");
        }
    } else {
        let _ = SWITCHY_PROXY_PORT.set(RwLock::new(port));
        log::debug!("[GlobalProxy] Initialized Switchy proxy port to {port}");
    }
}

/// The Switchy proxy server's listening port
fn get_proxy_port() -> u16 {
    SWITCHY_PROXY_PORT
        .get()
        .and_then(|lock| lock.read().ok())
        .map(|port| *port)
        .unwrap_or(15721) // default port as fallback
}

/// Initializes the global HTTP client
///
/// Call once at app startup.
///
/// # Arguments
/// * `proxy_url` - proxy URL, e.g. `http://127.0.0.1:7890` or `socks5://127.0.0.1:1080`;
///   None or an empty string means a direct connection
pub fn init(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let client = build_client(effective_url)?;

    // Try to initialize the global client; if it already exists, warn and update via apply_proxy
    if GLOBAL_CLIENT.set(RwLock::new(client.clone())).is_err() {
        log::warn!(
            "[GlobalProxy] [GP-003] Already initialized, updating instead: {}",
            effective_url
                .map(mask_url)
                .unwrap_or_else(|| "direct connection".to_string())
        );
        // Already initialized: update via apply_proxy instead
        return apply_proxy(proxy_url);
    }

    // Record the proxy URL
    let _ = CURRENT_PROXY_URL.set(RwLock::new(effective_url.map(|s| s.to_string())));

    log::info!(
        "[GlobalProxy] Initialized: {}",
        effective_url
            .map(mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );

    Ok(())
}

/// Validates a proxy configuration without applying it
///
/// Only checks that the proxy URL is valid; the global client is not updated.
/// Used to validate the configuration before persisting it.
///
/// # Arguments
/// * `proxy_url` - proxy URL; None or an empty string means a direct connection
///
/// # Returns
/// Ok(()) if valid, otherwise the error message
pub fn validate_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    // Call build_client only to validate; do not apply
    build_client(effective_url)?;
    Ok(())
}

/// Applies a proxy configuration (assumed already validated)
///
/// Applies the proxy configuration to the global client without further validation.
/// Call after validate_proxy succeeds.
///
/// # Arguments
/// * `proxy_url` - proxy URL; None or an empty string means a direct connection
pub fn apply_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let new_client = build_client(effective_url)?;

    // Update the client
    if let Some(lock) = GLOBAL_CLIENT.get() {
        let mut client = lock.write().map_err(|e| {
            log::error!("[GlobalProxy] [GP-001] Failed to acquire write lock: {e}");
            "Failed to update proxy: lock poisoned".to_string()
        })?;
        *client = new_client;
    } else {
        // Not initialized yet: initialize
        return init(proxy_url);
    }

    // Update the proxy URL record
    if let Some(lock) = CURRENT_PROXY_URL.get() {
        let mut url = lock.write().map_err(|e| {
            log::error!("[GlobalProxy] [GP-002] Failed to acquire URL write lock: {e}");
            "Failed to update proxy URL record: lock poisoned".to_string()
        })?;
        *url = effective_url.map(|s| s.to_string());
    }

    log::info!(
        "[GlobalProxy] Applied: {}",
        effective_url
            .map(mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );

    Ok(())
}

/// Updates the proxy configuration (hot reload)
///
/// Can be called at runtime to change the proxy setting without restarting the app.
/// Note: this validates and applies in one step; to validate, persist and then apply,
/// use validate_proxy + apply_proxy.
///
/// # Arguments
/// * `proxy_url` - the new proxy URL; None or an empty string means a direct connection
#[allow(dead_code)]
pub fn update_proxy(proxy_url: Option<&str>) -> Result<(), String> {
    let effective_url = proxy_url.filter(|s| !s.trim().is_empty());
    let new_client = build_client(effective_url)?;

    // Update the client
    if let Some(lock) = GLOBAL_CLIENT.get() {
        let mut client = lock.write().map_err(|e| {
            log::error!("[GlobalProxy] [GP-001] Failed to acquire write lock: {e}");
            "Failed to update proxy: lock poisoned".to_string()
        })?;
        *client = new_client;
    } else {
        // Not initialized yet: initialize
        return init(proxy_url);
    }

    // Update the proxy URL record
    if let Some(lock) = CURRENT_PROXY_URL.get() {
        let mut url = lock.write().map_err(|e| {
            log::error!("[GlobalProxy] [GP-002] Failed to acquire URL write lock: {e}");
            "Failed to update proxy URL record: lock poisoned".to_string()
        })?;
        *url = effective_url.map(|s| s.to_string());
    }

    log::info!(
        "[GlobalProxy] Updated: {}",
        effective_url
            .map(mask_url)
            .unwrap_or_else(|| "direct connection".to_string())
    );

    Ok(())
}

/// Returns the global HTTP client
///
/// Returns the client configured with the proxy (if one is set), otherwise a client that follows the system proxy.
pub fn get() -> Client {
    GLOBAL_CLIENT
        .get()
        .and_then(|lock| lock.read().ok())
        .map(|c| c.clone())
        .unwrap_or_else(|| {
            log::warn!("[GlobalProxy] [GP-004] Client not initialized, using fallback");
            build_client(None).unwrap_or_default()
        })
}

/// Returns the current proxy URL
///
/// The currently configured proxy URL; None means a direct connection.
pub fn get_current_proxy_url() -> Option<String> {
    CURRENT_PROXY_URL
        .get()
        .and_then(|lock| lock.read().ok())
        .and_then(|url| url.clone())
}

/// Whether a proxy is in use
#[allow(dead_code)]
pub fn is_proxy_enabled() -> bool {
    get_current_proxy_url().is_some()
}

/// Builds the HTTP client
fn build_client(proxy_url: Option<&str>) -> Result<Client, String> {
    install_rustls_provider();
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Duration::from_secs(60))
        // Disable reqwest's automatic decompression so it does not overwrite the client's original accept-encoding header.
        // response_processor decompresses responses by content-encoding itself.
        .no_gzip()
        .no_brotli()
        .no_deflate();

    // Use the proxy address if there is one; otherwise follow the system proxy
    if let Some(url) = proxy_url {
        // Validate the URL format and scheme first
        let parsed = url::Url::parse(url)
            .map_err(|e| format!("Invalid proxy URL '{}': {}", mask_url(url), e))?;

        let scheme = parsed.scheme();
        if !["http", "https", "socks5", "socks5h"].contains(&scheme) {
            return Err(format!(
                "Invalid proxy scheme '{}' in URL '{}'. Supported: http, https, socks5, socks5h",
                scheme,
                mask_url(url)
            ));
        }

        let proxy = reqwest::Proxy::all(url)
            .map_err(|e| format!("Invalid proxy URL '{}': {}", mask_url(url), e))?;
        builder = builder.proxy(proxy);
        log::debug!("[GlobalProxy] Proxy configured: {}", mask_url(url));
    } else {
        // No global proxy: let reqwest detect the system proxy (environment variables)
        // If the system proxy points at this machine, disable it to avoid a loop
        if system_proxy_points_to_loopback() {
            builder = builder.no_proxy();
            log::warn!(
                "[GlobalProxy] System proxy points to localhost, bypassing to avoid recursion"
            );
        } else {
            log::debug!("[GlobalProxy] Following system proxy (no explicit proxy configured)");
        }
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

fn system_proxy_points_to_loopback() -> bool {
    const KEYS: [&str; 6] = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ];

    KEYS.iter()
        .filter_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .any(|value| proxy_points_to_loopback(&value))
}

fn proxy_points_to_loopback(value: &str) -> bool {
    fn host_is_loopback(host: &str) -> bool {
        if host.eq_ignore_ascii_case("localhost") {
            return true;
        }
        host.parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
    }

    // Whether this is Switchy's own proxy port
    // Only a proxy pointing at ourselves needs skipping, to avoid recursion
    fn is_switchy_proxy_port(port: Option<u16>) -> bool {
        let switchy_port = get_proxy_port();
        port == Some(switchy_port)
    }

    if let Ok(parsed) = url::Url::parse(value) {
        if let Some(host) = parsed.host_str() {
            // True only when the host is loopback and the port is Switchy's
            return host_is_loopback(host) && is_switchy_proxy_port(parsed.port());
        }
        return false;
    }

    let with_scheme = format!("http://{value}");
    if let Ok(parsed) = url::Url::parse(&with_scheme) {
        if let Some(host) = parsed.host_str() {
            return host_is_loopback(host) && is_switchy_proxy_port(parsed.port());
        }
    }

    false
}

/// Hides sensitive parts of a URL (for logging)
pub fn mask_url(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        // Hide username and password; keep scheme, host and port
        let host = parsed.host_str().unwrap_or("?");
        match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        }
    } else {
        // URL failed to parse: return part of it
        if url.len() > 20 {
            format!("{}...", &url[..20])
        } else {
            url.to_string()
        }
    }
}

/// Builds a proxy URL from a provider's own proxy configuration
///
/// Converts ProviderProxyConfig into a proxy URL string
pub fn build_proxy_url_from_config(config: &ProviderProxyConfig) -> Option<String> {
    let proxy_type = config.proxy_type.as_deref().unwrap_or("http");
    let host = config.proxy_host.as_deref()?;
    let port = config.proxy_port?;

    // Build a proxy URL with credentials
    if let (Some(username), Some(password)) = (&config.proxy_username, &config.proxy_password) {
        if !username.is_empty() && !password.is_empty() {
            return Some(format!(
                "{proxy_type}://{username}:{password}@{host}:{port}"
            ));
        }
    }

    Some(format!("{proxy_type}://{host}:{port}"))
}

/// Builds an HTTP client from a provider's own proxy configuration
///
/// If the provider has its own proxy (enabled = true), builds a client with it;
/// otherwise returns None and the caller should use the global client.
///
/// # Arguments
/// * `proxy_config` - the provider's proxy configuration
///
/// # Returns
/// Some(Client) if the configuration is valid, otherwise None
pub fn build_client_for_provider(proxy_config: Option<&ProviderProxyConfig>) -> Option<Client> {
    let config = proxy_config.filter(|c| c.enabled)?;

    let proxy_url = build_proxy_url_from_config(config)?;

    log::debug!(
        "[ProviderProxy] Building client with proxy: {}",
        mask_url(&proxy_url)
    );

    // Build a client with the proxy
    let proxy = match reqwest::Proxy::all(&proxy_url) {
        Ok(p) => p,
        Err(e) => {
            log::error!(
                "[ProviderProxy] Failed to create proxy from '{}': {}",
                mask_url(&proxy_url),
                e
            );
            return None;
        }
    };

    match Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Duration::from_secs(60))
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .proxy(proxy)
        .build()
    {
        Ok(client) => {
            log::info!(
                "[ProviderProxy] Client built with proxy: {}",
                mask_url(&proxy_url)
            );
            Some(client)
        }
        Err(e) => {
            log::error!("[ProviderProxy] Failed to build client: {e}");
            None
        }
    }
}

/// Returns the HTTP client for a provider
///
/// Uses the provider's own proxy if enabled, otherwise the global client.
///
/// # Arguments
/// * `proxy_config` - the provider's proxy configuration
///
/// # Returns
/// The HTTP client suited to this provider
pub fn get_for_provider(proxy_config: Option<&ProviderProxyConfig>) -> Client {
    // Prefer the provider's own proxy
    if let Some(client) = build_client_for_provider(proxy_config) {
        return client;
    }

    // Fall back to the global client
    get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn test_mask_url() {
        assert_eq!(mask_url("http://127.0.0.1:7890"), "http://127.0.0.1:7890");
        assert_eq!(
            mask_url("http://user:pass@127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            mask_url("socks5://admin:secret@proxy.example.com:1080"),
            "socks5://proxy.example.com:1080"
        );
        // A URL without a port must not show ":?"
        assert_eq!(
            mask_url("http://proxy.example.com"),
            "http://proxy.example.com"
        );
        assert_eq!(
            mask_url("https://user:pass@proxy.example.com"),
            "https://proxy.example.com"
        );
    }

    #[test]
    fn test_build_client_direct() {
        let result = build_client(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_with_http_proxy() {
        let result = build_client(Some("http://127.0.0.1:7890"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_with_socks5_proxy() {
        let result = build_client(Some("socks5://127.0.0.1:1080"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_client_invalid_url() {
        // reqwest::Proxy::all does not reject some invalid URLs immediately
        // so use a clearly invalid scheme to trigger the error
        let result = build_client(Some("invalid-scheme://127.0.0.1:7890"));
        assert!(result.is_err(), "Should reject invalid proxy scheme");
    }

    #[test]
    fn test_proxy_points_to_loopback() {
        // Set the Switchy proxy port to 15721 (the default)
        set_proxy_port(15721);

        // Only loopback addresses on Switchy's own port return true
        assert!(proxy_points_to_loopback("http://127.0.0.1:15721"));
        assert!(proxy_points_to_loopback("socks5://localhost:15721"));
        assert!(proxy_points_to_loopback("127.0.0.1:15721"));

        // Other loopback ports are not skipped (other local proxy tools are allowed)
        assert!(!proxy_points_to_loopback("http://127.0.0.1:7890"));
        assert!(!proxy_points_to_loopback("socks5://localhost:1080"));

        // Non-loopback addresses are not skipped
        assert!(!proxy_points_to_loopback("http://192.168.1.10:7890"));
        assert!(!proxy_points_to_loopback("http://192.168.1.10:15721"));
    }

    #[test]
    fn test_system_proxy_points_to_loopback() {
        let _guard = env_lock().lock().unwrap();

        // Set the Switchy proxy port
        set_proxy_port(15721);

        let keys = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ];

        for key in &keys {
            std::env::remove_var(key);
        }

        // A proxy pointing at Switchy's port is skipped
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:15721");
        assert!(system_proxy_points_to_loopback());

        // A local proxy on another port is not skipped
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:7890");
        assert!(!system_proxy_points_to_loopback());

        // Non-loopback addresses are not skipped
        std::env::set_var("HTTP_PROXY", "http://10.0.0.2:7890");
        assert!(!system_proxy_points_to_loopback());

        for key in &keys {
            std::env::remove_var(key);
        }
    }
}
