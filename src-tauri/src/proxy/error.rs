use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("The proxy server is already running")]
    AlreadyRunning,

    #[error("The proxy server is not running")]
    NotRunning,

    #[error("Could not bind the address: {0}")]
    BindFailed(String),

    #[error("Timed out stopping the proxy server")]
    StopTimeout,

    #[error("Could not stop the proxy server: {0}")]
    StopFailed(String),

    #[error("Could not forward the request: {0}")]
    ForwardFailed(String),

    #[error("No provider is available")]
    NoAvailableProvider,

    #[error("Every provider's circuit is open; no channel is available")]
    AllProvidersCircuitOpen,

    #[error("No provider is configured")]
    NoProvidersConfigured,

    #[allow(dead_code)]
    #[error("The provider is unhealthy: {0}")]
    ProviderUnhealthy(String),

    #[error("Upstream error (status {status}): {body:?}")]
    UpstreamError { status: u16, body: Option<String> },

    #[error("Out of retries")]
    MaxRetriesExceeded,

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[allow(dead_code)]
    #[error("Format conversion error: {0}")]
    TransformError(String),

    #[allow(dead_code)]
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Timed out: {0}")]
    Timeout(String),

    /// Streamed response idle timeout
    #[allow(dead_code)]
    #[error("The streamed response went idle: no data for {0}s")]
    StreamIdleTimeout(u64),

    /// Authentication error
    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[allow(dead_code)]
    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, body) = match &self {
            ProxyError::UpstreamError {
                status: upstream_status,
                body: upstream_body,
            } => {
                let http_status =
                    StatusCode::from_u16(*upstream_status).unwrap_or(StatusCode::BAD_GATEWAY);

                // Parse the upstream body as JSON; if that fails, wrap it as a string
                let error_body = if let Some(body_str) = upstream_body {
                    if let Ok(json_body) = serde_json::from_str::<serde_json::Value>(body_str) {
                        // Upstream returned JSON: pass it through
                        json_body
                    } else {
                        // Upstream did not return JSON: wrap it as an error message
                        json!({
                            "error": {
                                "message": body_str,
                                "type": "upstream_error",
                            }
                        })
                    }
                } else {
                    json!({
                        "error": {
                            "message": format!("Upstream error (status {})", upstream_status),
                            "type": "upstream_error",
                        }
                    })
                };

                (http_status, error_body)
            }
            _ => {
                let (http_status, message) = match &self {
                    ProxyError::AlreadyRunning => (StatusCode::CONFLICT, self.to_string()),
                    ProxyError::NotRunning => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
                    ProxyError::BindFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopTimeout => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::StopFailed(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ForwardFailed(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
                    ProxyError::NoAvailableProvider => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::AllProvidersCircuitOpen => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::NoProvidersConfigured => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::ProviderUnhealthy(_) => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::MaxRetriesExceeded => {
                        (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
                    }
                    ProxyError::DatabaseError(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::ConfigError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::TransformError(_) => {
                        (StatusCode::UNPROCESSABLE_ENTITY, self.to_string())
                    }
                    ProxyError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
                    ProxyError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, self.to_string()),
                    ProxyError::StreamIdleTimeout(_) => {
                        (StatusCode::GATEWAY_TIMEOUT, self.to_string())
                    }
                    ProxyError::AuthError(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
                    ProxyError::Internal(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
                    }
                    ProxyError::UpstreamError { .. } => unreachable!(),
                };

                let error_body = json!({
                    "error": {
                        "message": message,
                        "type": "proxy_error",
                    }
                });

                (http_status, error_body)
            }
        };

        (status, Json(body)).into_response()
    }
}

/// Error category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Retryable (network problems, 5xx)
    Retryable, // network timeout, 5xx
    /// Not retryable (4xx, authentication failure)
    NonRetryable, // auth failure, bad parameters, 4xx
    #[allow(dead_code)]
    ClientAbort, // client aborted
}

/// Whether an error is retryable
#[allow(dead_code)]
pub fn categorize_error(error: &reqwest::Error) -> ErrorCategory {
    if error.is_timeout() || error.is_connect() {
        return ErrorCategory::Retryable;
    }

    if let Some(status) = error.status() {
        if status.is_server_error() {
            ErrorCategory::Retryable
        } else if status.is_client_error() {
            ErrorCategory::NonRetryable
        } else {
            ErrorCategory::Retryable
        }
    } else {
        ErrorCategory::Retryable
    }
}
