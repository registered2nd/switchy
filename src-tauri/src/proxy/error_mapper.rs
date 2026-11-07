//! Maps error types to HTTP status codes
//!
//! Maps ProxyError to a suitable HTTP status code for logging

use super::ProxyError;

/// Maps ProxyError to an HTTP status code
///
/// Mapping:
/// - Upstream error: the status code upstream returned
/// - Timeout: 504 Gateway Timeout
/// - Connection failure: 502 Bad Gateway
/// - No available provider: 503 Service Unavailable
/// - Retries exhausted: 503 Service Unavailable
/// - Anything else: 500 Internal Server Error
pub fn map_proxy_error_to_status(error: &ProxyError) -> u16 {
    match error {
        // Upstream error: use the actual status code
        ProxyError::UpstreamError { status, .. } => *status,

        // Timeout: 504 Gateway Timeout
        ProxyError::Timeout(_) => 504,

        // Forwarding/connection failure: 502 Bad Gateway
        ProxyError::ForwardFailed(_) => 502,

        // No available provider: 503 Service Unavailable
        ProxyError::NoAvailableProvider => 503,

        // All providers circuit-broken: 503 Service Unavailable
        ProxyError::AllProvidersCircuitOpen => 503,

        // No provider configured: 503 Service Unavailable
        ProxyError::NoProvidersConfigured => 503,

        // Retries exhausted: 503 Service Unavailable
        ProxyError::MaxRetriesExceeded => 503,

        // Provider unhealthy: 503 Service Unavailable
        ProxyError::ProviderUnhealthy(_) => 503,

        // Database error: 500 Internal Server Error
        ProxyError::DatabaseError(_) => 500,

        // Transform error: 500 Internal Server Error
        ProxyError::TransformError(_) => 500,

        // Other unknown error: 500 Internal Server Error
        _ => 500,
    }
}

/// Converts ProxyError into a user-friendly error message
pub fn get_error_message(error: &ProxyError) -> String {
    match error {
        ProxyError::UpstreamError { status, body } => {
            if let Some(body) = body {
                format!("Upstream error ({status}): {body}")
            } else {
                format!("Upstream error ({status})")
            }
        }
        ProxyError::Timeout(msg) => format!("Request timed out: {msg}"),
        ProxyError::ForwardFailed(msg) => format!("Could not forward the request: {msg}"),
        ProxyError::NoAvailableProvider => "No provider is available".to_string(),
        ProxyError::AllProvidersCircuitOpen => {
            "Every provider's circuit is open; no channel is available".to_string()
        }
        ProxyError::NoProvidersConfigured => "No provider is configured".to_string(),
        ProxyError::MaxRetriesExceeded => "Every provider failed; out of retries".to_string(),
        ProxyError::ProviderUnhealthy(msg) => format!("The provider is unhealthy: {msg}"),
        ProxyError::DatabaseError(msg) => format!("Database error: {msg}"),
        ProxyError::TransformError(msg) => format!("Request/response conversion error: {msg}"),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_upstream_error() {
        let error = ProxyError::UpstreamError {
            status: 401,
            body: Some("Unauthorized".to_string()),
        };
        assert_eq!(map_proxy_error_to_status(&error), 401);
    }

    #[test]
    fn test_map_timeout_error() {
        let error = ProxyError::Timeout("Request timeout".to_string());
        assert_eq!(map_proxy_error_to_status(&error), 504);
    }

    #[test]
    fn test_map_connection_error() {
        let error = ProxyError::ForwardFailed("Connection refused".to_string());
        assert_eq!(map_proxy_error_to_status(&error), 502);
    }

    #[test]
    fn test_map_no_provider_error() {
        let error = ProxyError::NoAvailableProvider;
        assert_eq!(map_proxy_error_to_status(&error), 503);
    }

    #[test]
    fn test_get_error_message() {
        let error = ProxyError::UpstreamError {
            status: 500,
            body: Some("Internal Server Error".to_string()),
        };
        let msg = get_error_message(&error);
        assert!(msg.contains("Upstream error"));
        assert!(msg.contains("500"));
        assert!(msg.contains("Internal Server Error"));
    }
}
