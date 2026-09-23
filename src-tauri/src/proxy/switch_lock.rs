//! Per-app switch lock
//!
//! Ensures only one provider switch runs at a time per app,
//! so concurrent switches cannot leave is_current out of sync with the live backup.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

/// One mutex per app type, so switches for the same app run serially.
///
/// Different apps (e.g. Claude and Codex) can switch in parallel.
#[derive(Clone, Default)]
pub struct SwitchLockManager {
    locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
}

impl SwitchLockManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the switch lock for the given app.
    ///
    /// Returns an `OwnedMutexGuard`; while it is held, other switches for the same `app_type` queue up.
    pub async fn lock_for_app(&self, app_type: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let locks = self.locks.read().await;
            if let Some(lock) = locks.get(app_type) {
                lock.clone()
            } else {
                drop(locks);
                let mut locks = self.locks.write().await;
                locks
                    .entry(app_type.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone()
            }
        };
        lock.lock_owned().await
    }
}
