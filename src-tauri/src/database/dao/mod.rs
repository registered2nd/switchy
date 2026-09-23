//! Data Access Object layer
//!
//! Database access operations for each domain

pub mod account_switches;
pub mod failover;
pub mod providers;
pub mod proxy;
pub mod settings;
pub mod stream_check;
pub mod universal_providers;
pub mod usage_rollup;

// All DAO methods live on the Database impl, so nothing else needs exporting
// Export FailoverQueueItem for external use
pub use account_switches::{AccountSwitch, SwitchReason};
pub use failover::FailoverQueueItem;
