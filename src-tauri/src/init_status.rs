use serde::Serialize;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Serialize)]
pub struct InitErrorPayload {
    pub path: String,
    pub error: String,
}

static INIT_ERROR: OnceLock<RwLock<Option<InitErrorPayload>>> = OnceLock::new();

fn cell() -> &'static RwLock<Option<InitErrorPayload>> {
    INIT_ERROR.get_or_init(|| RwLock::new(None))
}

#[allow(dead_code)]
pub fn set_init_error(payload: InitErrorPayload) {
    #[allow(clippy::unwrap_used)]
    if let Ok(mut guard) = cell().write() {
        *guard = Some(payload);
    }
}

pub fn get_init_error() -> Option<InitErrorPayload> {
    cell().read().ok()?.clone()
}
