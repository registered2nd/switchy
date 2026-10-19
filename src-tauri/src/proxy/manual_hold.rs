//! A provider picked by hand holds for a while, or until it is unlocked.
//!
//! While the hold lasts the proxy serves only that provider: no failover, no
//! rotation and no automatic switch moves the app off it, so a broken account
//! shows its error in the session instead of being passed over silently.
//! When the hold ends, automatic switching resumes. A locked provider holds
//! with no end, until it is unlocked or another provider is picked by hand.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a provider picked by hand is kept.
pub const HOLD: Duration = Duration::from_secs(10 * 60);

/// app type → (held provider id, hold end; none while locked)
type Holds = HashMap<String, (String, Option<Instant>)>;

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
/// A lock on the same provider stays.
pub fn hold(app_type: &str, provider_id: &str) {
    hold_for(app_type, provider_id, HOLD);
}

fn hold_for(app_type: &str, provider_id: &str, duration: Duration) {
    with_holds(|holds| {
        if matches!(holds.get(app_type), Some((id, None)) if id == provider_id) {
            return;
        }
        holds.insert(
            app_type.to_string(),
            (provider_id.to_string(), Some(Instant::now() + duration)),
        );
    });
}

/// Holds `provider_id` for `app_type` until [`unlock`].
pub fn lock(app_type: &str, provider_id: &str) {
    with_holds(|holds| holds.insert(app_type.to_string(), (provider_id.to_string(), None)));
}

/// Ends the lock for `app_type`, if there is one.
pub fn unlock(app_type: &str) {
    with_holds(|holds| {
        if matches!(holds.get(app_type), Some((_, None))) {
            holds.remove(app_type);
        }
    });
}

/// The provider locked for `app_type`.
pub fn locked(app_type: &str) -> Option<String> {
    with_holds(|holds| match holds.get(app_type) {
        Some((provider_id, None)) => Some(provider_id.clone()),
        _ => None,
    })
}

/// The provider held for `app_type`, while the hold lasts.
pub fn held(app_type: &str) -> Option<String> {
    with_holds(|holds| match holds.get(app_type) {
        Some((provider_id, None)) => Some(provider_id.clone()),
        Some((provider_id, Some(until))) if Instant::now() < *until => Some(provider_id.clone()),
        Some(_) => {
            holds.remove(app_type);
            None
        }
        None => None,
    })
}

/// Ends the hold for `app_type`.
#[cfg(test)]
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

    #[test]
    fn a_lock_holds_until_it_is_unlocked_or_another_provider_is_picked() {
        lock("lock-test", "a");
        hold_for("lock-test", "a", Duration::ZERO);
        assert_eq!(
            held("lock-test").as_deref(),
            Some("a"),
            "re-picking it keeps the lock"
        );
        assert_eq!(locked("lock-test").as_deref(), Some("a"));
        unlock("lock-test");
        assert_eq!(held("lock-test"), None);

        lock("lock-test", "a");
        hold_for("lock-test", "b", Duration::from_secs(60));
        assert_eq!(held("lock-test").as_deref(), Some("b"));
        assert_eq!(
            locked("lock-test"),
            None,
            "picking another provider ends the lock"
        );
    }
}
