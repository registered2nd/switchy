use rquickjs::{Context, Function, Runtime};
use serde_json::Value;
use std::collections::HashMap;
use url::{Host, Url};

use crate::error::AppError;

/// Run a usage query script
pub async fn execute_usage_script(
    script_code: &str,
    api_key: &str,
    base_url: &str,
    timeout_secs: u64,
    access_token: Option<&str>,
    user_id: Option<&str>,
    template_type: Option<&str>,
) -> Result<Value, AppError> {
    // Detect custom template mode
    // The template_type passed by the frontend takes precedence
    let is_custom_template = template_type.map(|t| t == "custom").unwrap_or(false);

    // 1. Substitute template variables, so secrets are not exposed
    let script_with_vars =
        build_script_with_vars(script_code, api_key, base_url, access_token, user_id);

    // 2. Validate base_url (only when one is given)
    // In custom template mode the user may skip template variables and write the full URL in the script
    if !base_url.is_empty() {
        validate_base_url(base_url)?;
    }

    // 3. Extract the request config in its own scope (so Runtime/Context are dropped before the await)
    let request_config = {
        let runtime = Runtime::new().map_err(|e| {
            AppError::localized(
                "usage_script.runtime_create_failed",
                format!("Failed to create JS runtime: {e}"),
            )
        })?;
        let context = Context::full(&runtime).map_err(|e| {
            AppError::localized(
                "usage_script.context_create_failed",
                format!("Failed to create JS context: {e}"),
            )
        })?;

        context.with(|ctx| {
            // Run the user code to get the config object
            let config: rquickjs::Object = ctx.eval(script_with_vars.clone()).map_err(|e| {
                AppError::localized(
                    "usage_script.config_parse_failed",
                    format!("Failed to parse config: {e}"),
                )
            })?;

            // Extract the request config
            let request: rquickjs::Object = config.get("request").map_err(|e| {
                AppError::localized(
                    "usage_script.request_missing",
                    format!("Missing request config: {e}"),
                )
            })?;

            // Convert the request to a JSON string
            let request_json: String = ctx
                .json_stringify(request)
                .map_err(|e| {
                    AppError::localized(
                        "usage_script.request_serialize_failed",
                        format!("Failed to serialize request: {e}"),
                    )
                })?
                .ok_or_else(|| {
                    AppError::localized(
                        "usage_script.serialize_none",
                        "Serialization returned None",
                    )
                })?
                .get()
                .map_err(|e| {
                    AppError::localized(
                        "usage_script.get_string_failed",
                        format!("Failed to get string: {e}"),
                    )
                })?;

            Ok::<_, AppError>(request_json)
        })?
    }; // Runtime and Context are dropped here

    // 4. Parse the request config
    let request: RequestConfig = serde_json::from_str(&request_config).map_err(|e| {
        AppError::localized(
            "usage_script.request_format_invalid",
            format!("Invalid request config format: {e}"),
        )
    })?;

    // 5. Validate the request URL (SSRF protection)
    // With a base_url, require the same origin; otherwise only basic checks
    validate_request_url(&request.url, base_url, is_custom_template)?;

    // 6. Send the HTTP request
    let response_data = send_http_request(&request, timeout_secs).await?;

    // 7. Run the extractor in its own scope (so Runtime/Context are dropped before the function ends)
    let result: Value = {
        let runtime = Runtime::new().map_err(|e| {
            AppError::localized(
                "usage_script.runtime_create_failed",
                format!("Failed to create JS runtime: {e}"),
            )
        })?;
        let context = Context::full(&runtime).map_err(|e| {
            AppError::localized(
                "usage_script.context_create_failed",
                format!("Failed to create JS context: {e}"),
            )
        })?;

        context.with(|ctx| {
            // Eval again to get the config object
            let config: rquickjs::Object = ctx.eval(script_with_vars.clone()).map_err(|e| {
                AppError::localized(
                    "usage_script.config_reparse_failed",
                    format!("Failed to re-parse config: {e}"),
                )
            })?;

            // Extract the extractor function
            let extractor: Function = config.get("extractor").map_err(|e| {
                AppError::localized(
                    "usage_script.extractor_missing",
                    format!("Missing extractor function: {e}"),
                )
            })?;

            // Convert the response data to a JS value
            let response_js: rquickjs::Value =
                ctx.json_parse(response_data.as_str()).map_err(|e| {
                    AppError::localized(
                        "usage_script.response_parse_failed",
                        format!("Failed to parse response JSON: {e}"),
                    )
                })?;

            // Call extractor(response)
            let result_js: rquickjs::Value = extractor.call((response_js,)).map_err(|e| {
                AppError::localized(
                    "usage_script.extractor_exec_failed",
                    format!("Failed to execute extractor: {e}"),
                )
            })?;

            // Convert to a JSON string
            let result_json: String = ctx
                .json_stringify(result_js)
                .map_err(|e| {
                    AppError::localized(
                        "usage_script.result_serialize_failed",
                        format!("Failed to serialize result: {e}"),
                    )
                })?
                .ok_or_else(|| {
                    AppError::localized(
                        "usage_script.serialize_none",
                        "Serialization returned None",
                    )
                })?
                .get()
                .map_err(|e| {
                    AppError::localized(
                        "usage_script.get_string_failed",
                        format!("Failed to get string: {e}"),
                    )
                })?;

            // Parse into serde_json::Value
            serde_json::from_str(&result_json).map_err(|e| {
                AppError::localized(
                    "usage_script.json_parse_failed",
                    format!("JSON parse failed: {e}"),
                )
            })
        })?
    }; // Runtime and Context are dropped here

    // 8. Validate the return value format
    validate_result(&result)?;

    Ok(result)
}

/// Request config
#[derive(Debug, serde::Deserialize)]
struct RequestConfig {
    url: String,
    method: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: Option<String>,
}

/// Send an HTTP request
async fn send_http_request(config: &RequestConfig, timeout_secs: u64) -> Result<String, AppError> {
    // Use the global HTTP client (already carries the proxy config)
    let client = crate::proxy::http_client::get();
    // Clamp the timeout so a bad config cannot block for long (2 s minimum, 30 s maximum)
    let request_timeout = std::time::Duration::from_secs(timeout_secs.clamp(2, 30));

    // Validate the HTTP method strictly; an invalid value does not fall back to GET
    let method: reqwest::Method = config.method.parse().map_err(|_| {
        AppError::localized(
            "usage_script.invalid_http_method",
            format!("Unsupported HTTP method: {}", config.method),
        )
    })?;

    let mut req = client
        .request(method.clone(), &config.url)
        .timeout(request_timeout);

    // Add headers
    for (k, v) in &config.headers {
        req = req.header(k, v);
    }

    // Add the body
    if let Some(body) = &config.body {
        req = req.body(body.clone());
    }

    // Send the request
    let resp = req.send().await.map_err(|e| {
        AppError::localized(
            "usage_script.request_failed",
            format!("Request failed: {e}"),
        )
    })?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| {
        AppError::localized(
            "usage_script.read_response_failed",
            format!("Failed to read response: {e}"),
        )
    })?;

    if !status.is_success() {
        let preview = if text.len() > 200 {
            let mut safe_cut = 200usize;
            while !text.is_char_boundary(safe_cut) {
                safe_cut = safe_cut.saturating_sub(1);
            }
            format!("{}...", &text[..safe_cut])
        } else {
            text.clone()
        };
        return Err(AppError::localized(
            "usage_script.http_error",
            format!("HTTP {status} : {preview}"),
        ));
    }

    Ok(text)
}

/// Validate the script's return value (a single object or an array)
fn validate_result(result: &Value) -> Result<(), AppError> {
    // For an array, validate each element
    if let Some(arr) = result.as_array() {
        if arr.is_empty() {
            return Err(AppError::localized(
                "usage_script.empty_array",
                "Script returned empty array",
            ));
        }
        for (idx, item) in arr.iter().enumerate() {
            validate_single_usage(item).map_err(|e| {
                AppError::localized(
                    "usage_script.array_validation_failed",
                    format!("Validation failed at index [{idx}]: {e}"),
                )
            })?;
        }
        return Ok(());
    }

    // For a single object, validate it directly (backward compatible)
    validate_single_usage(result)
}

/// Validate a single usage data object
fn validate_single_usage(result: &Value) -> Result<(), AppError> {
    let obj = result.as_object().ok_or_else(|| {
        AppError::localized(
            "usage_script.must_return_object",
            "Script must return object or array of objects",
        )
    })?;

    // Every field is optional; only types are checked
    if obj.contains_key("isValid")
        && !result["isValid"].is_null()
        && !result["isValid"].is_boolean()
    {
        return Err(AppError::localized(
            "usage_script.isvalid_type_error",
            "isValid must be boolean or null",
        ));
    }
    if obj.contains_key("invalidMessage")
        && !result["invalidMessage"].is_null()
        && !result["invalidMessage"].is_string()
    {
        return Err(AppError::localized(
            "usage_script.invalidmessage_type_error",
            "invalidMessage must be string or null",
        ));
    }
    if obj.contains_key("remaining")
        && !result["remaining"].is_null()
        && !result["remaining"].is_number()
    {
        return Err(AppError::localized(
            "usage_script.remaining_type_error",
            "remaining must be number or null",
        ));
    }
    if obj.contains_key("unit") && !result["unit"].is_null() && !result["unit"].is_string() {
        return Err(AppError::localized(
            "usage_script.unit_type_error",
            "unit must be string or null",
        ));
    }
    if obj.contains_key("total") && !result["total"].is_null() && !result["total"].is_number() {
        return Err(AppError::localized(
            "usage_script.total_type_error",
            "total must be number or null",
        ));
    }
    if obj.contains_key("used") && !result["used"].is_null() && !result["used"].is_number() {
        return Err(AppError::localized(
            "usage_script.used_type_error",
            "used must be number or null",
        ));
    }
    if obj.contains_key("planName")
        && !result["planName"].is_null()
        && !result["planName"].is_string()
    {
        return Err(AppError::localized(
            "usage_script.planname_type_error",
            "planName must be string or null",
        ));
    }
    if obj.contains_key("extra") && !result["extra"].is_null() && !result["extra"].is_string() {
        return Err(AppError::localized(
            "usage_script.extra_type_error",
            "extra must be string or null",
        ));
    }

    Ok(())
}

/// Build the script with variables substituted, staying compatible with old scripts
fn build_script_with_vars(
    script_code: &str,
    api_key: &str,
    base_url: &str,
    access_token: Option<&str>,
    user_id: Option<&str>,
) -> String {
    let mut replaced = script_code
        .replace("{{apiKey}}", api_key)
        .replace("{{baseUrl}}", base_url);

    if let Some(token) = access_token {
        replaced = replaced.replace("{{accessToken}}", token);
    }
    if let Some(uid) = user_id {
        replaced = replaced.replace("{{userId}}", uid);
    }

    replaced
}

/// Basic safety checks on base_url
fn validate_base_url(base_url: &str) -> Result<(), AppError> {
    if base_url.is_empty() {
        return Err(AppError::localized(
            "usage_script.base_url_empty",
            "base_url cannot be empty",
        ));
    }

    // Parse the URL
    let parsed_url = Url::parse(base_url).map_err(|e| {
        AppError::localized(
            "usage_script.base_url_invalid",
            format!("Invalid base_url: {e}"),
        )
    })?;

    let is_loopback = is_loopback_host(&parsed_url);

    // Must be HTTPS (localhost allowed for development)
    if parsed_url.scheme() != "https" && !is_loopback {
        return Err(AppError::localized(
            "usage_script.base_url_https_required",
            "base_url must use HTTPS (localhost allowed)",
        ));
    }

    // Check the hostname format is valid
    let hostname = parsed_url.host_str().ok_or_else(|| {
        AppError::localized(
            "usage_script.base_url_hostname_missing",
            "base_url must include a valid hostname",
        )
    })?;

    // Basic hostname format check
    if hostname.is_empty() {
        return Err(AppError::localized(
            "usage_script.base_url_hostname_empty",
            "base_url hostname cannot be empty",
        ));
    }

    // Check for an obvious private IP (lenient at the base_url stage; the request_url stage does the main check)
    if is_suspicious_hostname(hostname) {
        return Err(AppError::localized(
            "usage_script.base_url_suspicious",
            "base_url contains a suspicious hostname",
        ));
    }

    Ok(())
}

/// Validate that a request URL is safe (SSRF protection)
fn validate_request_url(
    request_url: &str,
    base_url: &str,
    is_custom_template: bool,
) -> Result<(), AppError> {
    // Parse the request URL
    let parsed_request = Url::parse(request_url).map_err(|e| {
        AppError::localized(
            "usage_script.request_url_invalid",
            format!("Invalid request URL: {e}"),
        )
    })?;

    let is_request_loopback = is_loopback_host(&parsed_request);

    // Must use HTTPS (localhost allowed for development)
    // In custom template mode the user may choose HTTP (at their own risk)
    if !is_custom_template && parsed_request.scheme() != "https" && !is_request_loopback {
        return Err(AppError::localized(
            "usage_script.request_https_required",
            "Request URL must use HTTPS (localhost allowed)",
        ));
    }

    // With a non-empty base_url, check same origin
    // 🔧 In custom template mode the user may reach any HTTPS domain, so the same-origin check is skipped
    if !base_url.is_empty() && !is_custom_template {
        // Parse the base URL
        let parsed_base = Url::parse(base_url).map_err(|e| {
            AppError::localized(
                "usage_script.base_url_invalid",
                format!("Invalid base_url: {e}"),
            )
        })?;

        // Core safety check: must share base_url's origin (same host and port)
        if parsed_request.host_str() != parsed_base.host_str() {
            return Err(AppError::localized(
                "usage_script.request_host_mismatch",
                format!(
                    "Request host {} must match base_url host {} (same-origin required)",
                    parsed_request.host_str().unwrap_or("unknown"),
                    parsed_base.host_str().unwrap_or("unknown")
                ),
            ));
        }

        // Check the ports match (accounting for default ports)
        // port_or_known_default() fills in default ports (http->80, https->443)
        match (
            parsed_request.port_or_known_default(),
            parsed_base.port_or_known_default(),
        ) {
            (Some(request_port), Some(base_port)) if request_port == base_port => {
                // Ports match; carry on
            }
            (Some(request_port), Some(base_port)) => {
                return Err(AppError::localized(
                    "usage_script.request_port_mismatch",
                    format!("Request port {request_port} must match base_url port {base_port}"),
                ));
            }
            _ => {
                // Should not happen: port_or_known_default() should always return Some
                return Err(AppError::localized(
                    "usage_script.request_port_unknown",
                    "Unable to determine port number",
                ));
            }
        }

        // Block private IP addresses (unless base_url is itself private, for development)
        if let Some(host) = parsed_request.host_str() {
            let base_host = parsed_base.host_str().unwrap_or("");

            // If base_url is not private, block private IPs
            if !is_private_ip(base_host) && is_private_ip(host) {
                return Err(AppError::localized(
                    "usage_script.private_ip_blocked",
                    "Access to private IP addresses is blocked",
                ));
            }
        }
    } else {
        // Custom template mode: no base_url, so extra checks are needed
        // Block private IP addresses (SSRF protection)
        if let Some(host) = parsed_request.host_str() {
            if is_private_ip(host) && !is_request_loopback {
                return Err(AppError::localized(
                    "usage_script.private_ip_blocked",
                    "Access to private IP addresses is blocked (localhost allowed)",
                ));
            }
        }
    }

    Ok(())
}

/// Check whether a host is a private IP address
fn is_private_ip(host: &str) -> bool {
    // localhost check
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    // Try to parse as an IP address
    if let Ok(ip_addr) = host.parse::<std::net::IpAddr>() {
        return is_private_ip_addr(ip_addr);
    }

    // Not an IP address, so not a private IP
    false
}

/// Check whether an IP address is private, using the standard library API
fn is_private_ip_addr(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();

            // 0.0.0.0/8 (including the unspecified address)
            if octets[0] == 0 {
                return true;
            }

            // RFC1918 private ranges
            // 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }

            // 172.16.0.0/12 (172.16.0.0 - 172.31.255.255)
            if octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31 {
                return true;
            }

            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }

            // Other special addresses
            // 169.254.0.0/16 (link-local)
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }

            // 127.0.0.0/8 (loopback)
            if octets[0] == 127 {
                return true;
            }

            false
        }
        std::net::IpAddr::V6(ipv6) => {
            // IPv6 private address checks, using standard library methods

            // ::1 (loopback)
            if ipv6.is_loopback() {
                return true;
            }

            // Unique local addresses (fc00::/7)
            // Rust 1.70+ has ipv6.is_unique_local(),
            // but we check by hand for compatibility
            let first_segment = ipv6.segments()[0];
            if (first_segment & 0xfe00) == 0xfc00 {
                return true;
            }

            // Link-local addresses (fe80::/10)
            if (first_segment & 0xffc0) == 0xfe80 {
                return true;
            }

            // Unspecified address ::
            if ipv6.is_unspecified() {
                return true;
            }

            false
        }
    }
}

/// Check for a suspicious hostname (only obviously unsafe patterns)
fn is_suspicious_hostname(hostname: &str) -> bool {
    // Empty hostname
    if hostname.is_empty() {
        return true;
    }

    // Obvious hostname format problems
    if hostname.contains("..") || hostname.starts_with(".") || hostname.ends_with(".") {
        return true;
    }

    // A bare IP address (lenient here; the later same-origin check does the main work)
    if hostname.parse::<std::net::IpAddr>().is_ok() {
        // Do not reject IP addresses here; leave them to the same-origin check
        return false;
    }

    // Obviously invalid characters
    let suspicious_chars = ['<', '>', '"', '\'', '\n', '\r', '\t', '\0'];
    if hostname.chars().any(|c| suspicious_chars.contains(&c)) {
        return true;
    }

    false
}

/// Whether a URL points at this machine (localhost / loopback)
fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_private_ip_validation() {
        // IPv4 private addresses

        // RFC1918 private addresses: true
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("10.255.255.254"));
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("192.168.255.255"));

        // Link-local addresses: true
        assert!(is_private_ip("169.254.0.1"));
        assert!(is_private_ip("169.254.255.255"));

        // Loopback addresses: true
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("localhost"));

        // Public 172.x.x.x addresses: false (the point of the fix)
        assert!(!is_private_ip("172.0.0.1"));
        assert!(!is_private_ip("172.15.255.255"));
        assert!(!is_private_ip("172.32.0.1"));
        assert!(!is_private_ip("172.64.0.1"));
        assert!(!is_private_ip("172.67.0.1")); // Cloudflare CDN
        assert!(!is_private_ip("172.68.0.1"));
        assert!(!is_private_ip("172.100.50.25"));
        assert!(!is_private_ip("172.255.255.255"));

        // Other public addresses: false
        assert!(!is_private_ip("8.8.8.8")); // Google DNS
        assert!(!is_private_ip("1.1.1.1")); // Cloudflare DNS
        assert!(!is_private_ip("208.67.222.222")); // OpenDNS
        assert!(!is_private_ip("180.76.76.76")); // Baidu DNS

        // Domain names: false
        assert!(!is_private_ip("api.example.com"));
        assert!(!is_private_ip("www.google.com"));
    }

    #[test]
    fn test_ipv6_private_validation() {
        // IPv6 private addresses
        assert!(is_private_ip("::1")); // loopback
        assert!(is_private_ip("fc00::1")); // unique local
        assert!(is_private_ip("fd00::1")); // unique local
        assert!(is_private_ip("fe80::1")); // link-local
        assert!(is_private_ip("::")); // unspecified

        // IPv6 public addresses: false (the point of the fix)
        assert!(!is_private_ip("2001:4860:4860::8888")); // Google DNS IPv6
        assert!(!is_private_ip("2606:4700:4700::1111")); // Cloudflare DNS IPv6
        assert!(!is_private_ip("2404:6800:4001:c01::67")); // Google DNS IPv6 (another form)
        assert!(!is_private_ip("2001:db8::1")); // documentation address (not private)

        // Public addresses containing the substring ::1 that are not loopback
        assert!(!is_private_ip("2001:db8::1abc")); // contains ::1abc but is not loopback
        assert!(!is_private_ip("2606:4700::1")); // contains ::1 but is not loopback
    }

    #[test]
    fn test_hostname_bypass_prevention() {
        // Looks local but is a domain name
        assert!(!is_private_ip("127.0.0.1.evil.com"));
        assert!(!is_private_ip("localhost.evil.com"));

        // 0.0.0.0 counts as local and is blocked
        assert!(is_private_ip("0.0.0.0"));
    }

    #[test]
    fn test_https_bypass_prevention() {
        // HTTP to a non-local domain is rejected
        let result = validate_base_url("http://127.0.0.1.evil.com/api");
        assert!(
            result.is_err(),
            "Should reject HTTP for non-localhost domains"
        );
    }

    #[test]
    fn test_edge_cases() {
        // Edge cases
        assert!(is_private_ip("172.16.0.0")); // RFC1918 start
        assert!(is_private_ip("172.31.255.255")); // RFC1918 end
        assert!(is_private_ip("10.0.0.0")); // 10.0.0.0/8 start
        assert!(is_private_ip("10.255.255.255")); // 10.0.0.0/8 end
        assert!(is_private_ip("192.168.0.0")); // 192.168.0.0/16 start
        assert!(is_private_ip("192.168.255.255")); // 192.168.0.0/16 end

        // Public addresses right next to RFC1918 ranges: false
        assert!(!is_private_ip("172.15.255.255")); // just before 172.16.0.0
        assert!(!is_private_ip("172.32.0.0")); // just after 172.31.255.255
    }

    #[test]
    fn test_ip_addr_parsing() {
        // IP address parsing
        let ipv4_private = "10.0.0.1".parse::<std::net::IpAddr>().unwrap();
        assert!(is_private_ip_addr(ipv4_private));

        let ipv4_public = "172.67.0.1".parse::<std::net::IpAddr>().unwrap();
        assert!(!is_private_ip_addr(ipv4_public));

        let ipv6_private = "fc00::1".parse::<std::net::IpAddr>().unwrap();
        assert!(is_private_ip_addr(ipv6_private));

        let ipv6_public = "2001:4860:4860::8888".parse::<std::net::IpAddr>().unwrap();
        assert!(!is_private_ip_addr(ipv6_public));
    }

    #[test]
    fn test_port_comparison() {
        // Port comparison handles default and explicit ports

        // Cases: (base_url, request_url, should_match)
        let test_cases = vec![
            // HTTPS default port
            (
                "https://api.example.com",
                "https://api.example.com/v1/test",
                true,
            ),
            (
                "https://api.example.com",
                "https://api.example.com:443/v1/test",
                true,
            ),
            (
                "https://api.example.com:443",
                "https://api.example.com/v1/test",
                true,
            ),
            (
                "https://api.example.com:443",
                "https://api.example.com:443/v1/test",
                true,
            ),
            // Port mismatch
            (
                "https://api.example.com",
                "https://api.example.com:8443/v1/test",
                false,
            ),
            (
                "https://api.example.com:443",
                "https://api.example.com:8443/v1/test",
                false,
            ),
        ];

        for (base_url, request_url, should_match) in test_cases {
            let result = validate_request_url(request_url, base_url, false);

            if should_match {
                assert!(
                    result.is_ok(),
                    "URL that should match was rejected: base_url={}, request_url={}, error={}",
                    base_url,
                    request_url,
                    result.unwrap_err()
                );
            } else {
                assert!(
                    result.is_err(),
                    "URL that should not match was allowed: base_url={}, request_url={}",
                    base_url,
                    request_url
                );
            }
        }
    }
}
