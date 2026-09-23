//! A provider picked by hand holds for a while.
//!
//! While the hold lasts the proxy serves only that provider: no failover, no
//! rotation and no automatic switch moves the app off it, so a broken account
//! shows its error in the session instead of being passed over silently.
//! When the hold ends, automatic switching resumes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a provider picked by hand is kept.
pub const HOLD: Duration = Duration::from_secs(10 * 60);

type Holds = HashMap<String, (String, Instant)>;

/// app type → (held provider id, hold end)
#[cfg(not(test))]
static HOLDS: once_cell::sync::Lazy<std::sync::Mutex<Holds>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

#[cfg(not(test))]
fn with_holds<R>(f: impl FnOnce(&mut Holds) -> R) -> R {
    f(&mut HOLDS.lock().unwrap_or_else(|e| e.into_inner()))
}

// Tests keep holds per thread, so a pick made in one test cannot steer
// another running in the same process.
#[cfg(test)]
thread_local! {
    static HOLDS: std::cell::RefCell<Holds> = std::cell::RefCell::new(HashMap::new());
}

#[cfg(test)]
fn with_holds<R>(f: impl FnOnce(&mut Holds) -> R) -> R {
    HOLDS.with(|holds| f(&mut holds.borrow_mut()))
}

/// Holds `provider_id` for `app_type` for [`HOLD`], replacing any earlier hold.
pub fn hold(app_type: &str, provider_id: &str) {
    hold_for(app_type, provider_id, HOLD);
}

fn hold_for(app_type: &str, provider_id: &str, duration: Duration) {
    with_holds(|holds| {
        holds.insert(
            app_type.to_string(),
            (provider_id.to_string(), Instant::now() + duration),
        )
    });
}

/// The provider held for `app_type`, while the hold lasts.
pub fn held(app_type: &str) -> Option<String> {
    with_holds(|holds| match holds.get(app_type) {
        Some((provider_id, until)) if Instant::now() < *until => Some(provider_id.clone()),
        Some(_) => {
            holds.remove(app_type);
            None
        }
        None => None,
    })
}

/// Ends the hold for `app_type`.
pub fn release(app_type: &str) {
    with_holds(|holds| holds.remove(app_type));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_lasts_until_it_ends_and_a_new_pick_replaces_it() {
        hold_for("hold-test", "a", Duration::from_secs(60));
        assert_eq!(held("hold-test").as_deref(), Some("a"));
        hold_for("hold-test", "b", Duration::from_secs(60));
        assert_eq!(held("hold-test").as_deref(), Some("b"));
        hold_for("hold-test", "b", Duration::ZERO);
        assert_eq!(held("hold-test"), None);
    }
}
