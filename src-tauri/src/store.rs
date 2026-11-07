use crate::database::Database;
use crate::services::ProxyService;
use std::sync::Arc;

/// Global application state
pub struct AppState {
    pub db: Arc<Database>,
    pub proxy_service: ProxyService,
}

impl AppState {
    /// Create a new application state
    pub fn new(db: Arc<Database>) -> Self {
        let proxy_service = ProxyService::new(db.clone());

        Self { db, proxy_service }
    }
}
