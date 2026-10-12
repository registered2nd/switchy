//! Global HTTP client
//!
//! Provides an HTTP client that honours the global proxy setting.
//! Every module that sends HTTP requests should use the client from this module.

use super::outbound_route::{require_route, OutboundRoute};
use crate::provider::ProviderProxyConfig;
use once_cell::sync::OnceCell;
use reqwest::Client;
use std::env;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::Duration;

static REQUIRE_PROXY: AtomicBool = AtomicBool::new(false);

pub fn set_require_proxy(required: bool) {
    REQUIRE_PROXY.store(required, Ordering::Relaxed);
}

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
    // Preserve route intent even when malformed saved configuration cannot
    // construct a client. No startup failure may silently select direct.
    let record = CURRENT_PROXY_URL.get_or_init(|| RwLock::new(None));
    *record
        .write()
        .map_err(|_| "Proxy configuration lock unavailable")? = effective_url.map(str::to_owned);
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
    get_for_provider(None).unwrap_or_else(|e| {
        // Older callers return Client rather than Result. Preserve fail-closed
        // behavior for them too; port zero is never an outbound direct route.
        log::error!("[GlobalProxy] outbound route unavailable: {e}");
        Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:0").expect("constant proxy URL"))
            .build()
            .expect("failed-route client")
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
    build_client_with(proxy_url, false)
}

fn build_client_with(proxy_url: Option<&str>, http1_only: bool) -> Result<Client, String> {
    let route = match proxy_url {
        Some(url) => OutboundRoute::Proxy(validate_proxy_url(url)?),
        None => OutboundRoute::Direct,
    };
    build_client_for_route(&route, http1_only)
}

pub(crate) fn build_client_for_route(
    route: &OutboundRoute,
    http1_only: bool,
) -> Result<Client, String> {
    install_rustls_provider();
    // Each resolved proxy is explicit, so NO_PROXY/system bypass lists cannot
    // turn an account's configured route into a direct connection.
    let mut builder = Client::builder().no_proxy();
    if http1_only {
        builder = builder.http1_only();
    }
    let mut builder = builder
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
    if let OutboundRoute::Proxy(url) = route {
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
    } else if let OutboundRoute::Tun(tun) = route {
        builder = builder.local_address(tun.source);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            builder = builder.interface(&tun.interface);
        }
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

#[cfg(test)]
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
    let host = config.proxy_host.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }
    let port = config.proxy_port?;
    if port == 0 {
        return None;
    }
    let mut url = url::Url::parse(&format!("{proxy_type}://{host}:{port}")).ok()?;
    if let Some(username) = &config.proxy_username {
        url.set_username(username).ok()?;
        if let Some(password) = &config.proxy_password {
            url.set_password(Some(password)).ok()?;
        }
    }
    Some(url.to_string())
}

/// Validate an explicit route before building any connection.
pub(crate) fn validate_proxy_url(value: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value).map_err(|e| format!("Invalid configured proxy: {e}"))?;
    if !["http", "https", "socks5", "socks5h"].contains(&parsed.scheme())
        || parsed.host_str().is_none()
        || parsed.port_or_known_default().filter(|p| *p != 0).is_none()
    {
        return Err(
            "Invalid configured proxy: a supported scheme, host and port are required".into(),
        );
    }
    if proxy_points_to_loopback(value) {
        return Err("Configured proxy points to Switchy's own listener".into());
    }
    Ok(parsed.to_string())
}

fn system_proxy_url() -> Result<Option<String>, String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        if let Ok(value) = env::var(key) {
            let value = value.trim();
            if value.is_empty() || proxy_points_to_loopback(value) {
                continue;
            }
            let normalized = if value.contains("://") {
                value.to_string()
            } else {
                format!("http://{value}")
            };
            return validate_proxy_url(&normalized).map(Some);
        }
    }
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        if let Ok(settings) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        {
            if settings.get_value::<u32, _>("ProxyEnable").unwrap_or(0) != 0 {
                if let Ok(value) = settings.get_value::<String, _>("ProxyServer") {
                    let raw = if value.contains('=') {
                        value
                            .split(';')
                            .find_map(|p| p.trim().strip_prefix("https="))
                            .or_else(|| {
                                value
                                    .split(';')
                                    .find_map(|p| p.trim().strip_prefix("socks="))
                            })
                    } else {
                        Some(value.as_str())
                    };
                    if let Some(raw) = raw.filter(|s| !s.trim().is_empty()) {
                        if !proxy_points_to_loopback(raw) {
                            let scheme = if value.contains("socks=") && !value.contains("https=") {
                                "socks5"
                            } else {
                                "http"
                            };
                            let normalized = if raw.contains("://") {
                                raw.into()
                            } else {
                                format!("{scheme}://{raw}")
                            };
                            return validate_proxy_url(&normalized).map(Some);
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

/// Account proxy > global proxy > explicit system proxy. Malformed enabled
/// account settings are an error, never permission to use another route.
pub fn effective_proxy_url(
    proxy_config: Option<&ProviderProxyConfig>,
) -> Result<Option<String>, String> {
    if let Some(config) = proxy_config.filter(|c| c.enabled) {
        let url = build_proxy_url_from_config(config)
            .ok_or("Configured account proxy is incomplete or malformed")?;
        return validate_proxy_url(&url).map(Some);
    }
    if let Some(url) = get_current_proxy_url() {
        return validate_proxy_url(&url).map(Some);
    }
    system_proxy_url()
}

pub(crate) fn resolve_route(
    proxy_config: Option<&ProviderProxyConfig>,
) -> Result<OutboundRoute, String> {
    require_route(
        effective_proxy_url(proxy_config)?,
        REQUIRE_PROXY.load(Ordering::Relaxed),
        super::outbound_route::detect_tun,
    )
}

pub fn route_cache_key(proxy_config: Option<&ProviderProxyConfig>) -> Result<String, String> {
    resolve_route(proxy_config).map(|route| route.cache_key())
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
pub fn get_for_provider(proxy_config: Option<&ProviderProxyConfig>) -> Result<Client, String> {
    let route = resolve_route(proxy_config)?;
    if !proxy_config.is_some_and(|c| c.enabled) {
        let global = get_current_proxy_url();
        let cached_route = match &route {
            OutboundRoute::Proxy(url) => global
                .as_deref()
                .is_some_and(|saved| validate_proxy_url(saved).is_ok_and(|saved| saved == *url)),
            OutboundRoute::Direct => global.is_none(),
            OutboundRoute::Tun(_) => false,
        };
        if cached_route {
            if let Some(client) = GLOBAL_CLIENT.get().and_then(|lock| lock.read().ok()) {
                return Ok(client.clone());
            }
        }
    }
    build_client_for_route(&route, false)
}

/// An HTTP/1.1-only client taking the same route as `get_for_provider`, for a
/// request upgraded to a WebSocket: an upgrade does not exist in HTTP/2.
pub fn get_http1_for_provider(
    proxy_config: Option<&ProviderProxyConfig>,
) -> Result<Client, String> {
    build_client_for_route(&resolve_route(proxy_config)?, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn account_proxy(scheme: &str, host: Option<&str>, port: Option<u16>) -> ProviderProxyConfig {
        ProviderProxyConfig {
            enabled: true,
            proxy_type: Some(scheme.into()),
            proxy_host: host.map(str::to_owned),
            proxy_port: port,
            proxy_username: None,
            proxy_password: None,
        }
    }

    #[tokio::test]
    async fn malformed_and_dead_account_proxies_never_reach_direct_target() {
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_url = format!("http://{}/credential-target", target.local_addr().unwrap());
        for config in [
            account_proxy("http", None, Some(1234)),
            account_proxy("unknown", Some("127.0.0.1"), Some(1234)),
            account_proxy("http", Some("127.0.0.1"), Some(0)),
        ] {
            assert!(get_for_provider(Some(&config)).is_err());
            assert!(get_http1_for_provider(Some(&config)).is_err());
        }
        let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_port = dead.local_addr().unwrap().port();
        drop(dead);
        for scheme in ["http", "socks5"] {
            let config = account_proxy(scheme, Some("127.0.0.1"), Some(dead_port));
            for client in [
                get_for_provider(Some(&config)).unwrap(),
                get_http1_for_provider(Some(&config)).unwrap(),
            ] {
                assert!(client
                    .get(&target_url)
                    .bearer_auth("test-secret")
                    .timeout(Duration::from_millis(500))
                    .send()
                    .await
                    .is_err());
            }
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(80), target.accept())
                .await
                .is_err(),
            "configured proxy failures must never touch the target directly"
        );
    }

    #[test]
    fn proxy_credentials_are_encoded_without_changing_route() {
        let mut config = account_proxy("socks5", Some("127.0.0.1"), Some(1080));
        config.proxy_username = Some("name@example".into());
        config.proxy_password = Some("password/@:#".into());
        let url = build_proxy_url_from_config(&config).unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert_eq!(
            percent_encoding::percent_decode_str(parsed.username()).decode_utf8_lossy(),
            "name@example"
        );
        assert_eq!(
            percent_encoding::percent_decode_str(parsed.password().unwrap()).decode_utf8_lossy(),
            "password/@:#"
        );
    }

    #[tokio::test]
    async fn tun_client_never_falls_back_to_ipv6_or_unbound_ipv4() {
        use super::super::outbound_route::TunRoute;
        let target4 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let route = OutboundRoute::Tun(TunRoute {
            interface: "missing_tun".into(),
            source: "192.0.2.1".parse().unwrap(),
            #[cfg(target_os = "macos")]
            index: u32::MAX,
        });
        let client = build_client_for_route(&route, false).unwrap();
        assert!(client
            .get(format!("http://{}/", target4.local_addr().unwrap()))
            .timeout(Duration::from_millis(400))
            .send()
            .await
            .is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(80), target4.accept())
                .await
                .is_err()
        );
        // A single-family source bind must filter the other family completely,
        // including literal addresses rather than only DNS-resolved names.
        if let Ok(target6) = tokio::net::TcpListener::bind("[::1]:0").await {
            assert!(client
                .get(format!("http://{}/", target6.local_addr().unwrap()))
                .timeout(Duration::from_millis(400))
                .send()
                .await
                .is_err());
            assert!(
                tokio::time::timeout(Duration::from_millis(80), target6.accept())
                    .await
                    .is_err()
            );
        }
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
