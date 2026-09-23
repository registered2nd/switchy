//! Proxy server module
//!
//! Local HTTP proxy with multi-provider failover and request passthrough

pub mod account_pool;
pub mod body_filter;
pub mod cache_injector;
pub mod circuit_breaker;
pub mod claude_pool;
pub mod codex_pool;
pub mod copilot_optimizer;
pub mod error;
pub mod error_mapper;
pub(crate) mod failover_switch;
mod forwarder;
pub mod handler_config;
pub mod handler_context;
mod handlers;
mod health;
pub mod http_client;
pub mod hyper_client;
pub mod keep_warm;
pub mod local_tls;
pub mod log_codes;
pub mod manual_hold;
pub mod model_mapper;
pub mod provider_router;
pub mod providers;
pub mod response_handler;
pub mod response_processor;
pub(crate) mod server;
pub mod session;
pub(crate) mod sse;
pub(crate) mod switch_lock;
pub mod thinking_budget_rectifier;
pub mod thinking_optimizer;
pub mod thinking_rectifier;
pub(crate) mod types;
pub mod usage;

// Public exports for other modules (commands, services, etc.)
#[allow(unused_imports)]
pub use circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerStats, CircuitState,
};
#[allow(unused_imports)]
pub use error::ProxyError;
#[allow(unused_imports)]
pub use provider_router::ProviderRouter;
#[allow(unused_imports)]
pub use response_handler::{NonStreamHandler, ResponseType, StreamHandler};
#[allow(unused_imports)]
pub use session::{
    extract_session_id, ClientFormat, ProxySession, SessionIdResult, SessionIdSource,
};
#[allow(unused_imports)]
pub use types::{ProxyConfig, ProxyServerInfo, ProxyStatus};

// Shared between submodules
// The compiler may flag this as unused, but submodules do use it
#[allow(unused_imports)]
pub(crate) use types::*;
