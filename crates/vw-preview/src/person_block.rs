//! The built-in browser's input block while the agent controls a chat's browser: one lease per owner session (the
//! chat), read by the engine's native input watch of every page view of that chat. The core keeps the lease current
//! (the same rule as the agent control frame's Working state); the engine asks [`blocked`] for each of the person's
//! native events and swallows the input the platform rule names (`vw_platform::native_click::person::block`).
//!
//! Fails open in time, closed in scope: a lease ends at its deadline even when no one lifts it (a lost update never
//! keeps the person out for good), and every view of the chat's pages, including one mounted after the lease was
//! set, reads the chat's lease through its label.
//!
//! A press the block swallowed is reported per chat ([`refused`] -> the [`on_refused`] listeners): it reaches no
//! view, so the app window's own keyboard focus stays where it was, and the app must learn that the person meant the
//! page.
//!
//! Depends on: std only. Called by: `PreviewService::set_agent_input_block` (core), the mount (labels), and the
//! engine's input watch (reads and reports, on the UI thread or a hook thread).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Instant;

#[derive(Default)]
struct Registry {
    /// Owner session -> the lease's end.
    until: HashMap<String, Instant>,
    /// Mount label -> its page's owner session.
    owners: HashMap<String, String>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

/// Told of the owner chat of each swallowed press; answers `false` once it is gone (then it is dropped).
type Listener = Arc<dyn Fn(&str) -> bool + Send + Sync>;

static LISTENERS: LazyLock<Mutex<Vec<Listener>>> = LazyLock::new(Mutex::default);

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets (`Some`: until that instant) or lifts (`None`) the block of every page view of `owner`.
pub fn set(owner: &str, until: Option<Instant>) {
    let mut registry = registry();
    match until {
        Some(until) => {
            registry.until.insert(owner.to_owned(), until);
        }
        None => {
            registry.until.remove(owner);
        }
    }
}

/// The chats whose browser holds a block lease now (ended leases included until lifted).
#[must_use]
pub fn owners() -> Vec<String> {
    registry().until.keys().cloned().collect()
}

/// Records which chat a mounted view's page belongs to (at mount and when pages move between chats).
pub fn assign(label: &str, owner: &str) {
    registry().owners.insert(label.to_owned(), owner.to_owned());
}

/// Forgets a closed view.
pub fn forget(label: &str) {
    registry().owners.remove(label);
}

/// Whether the view `label` is blocked now.
#[must_use]
pub fn blocked(label: &str) -> bool {
    let registry = registry();
    registry
        .owners
        .get(label)
        .and_then(|owner| registry.until.get(owner))
        .is_some_and(|until| Instant::now() < *until)
}

/// Adds a listener told of the owner chat of every press a blocked view swallowed; it stays until it answers `false`.
pub fn on_refused(listener: impl Fn(&str) -> bool + Send + Sync + 'static) {
    LISTENERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Arc::new(listener));
}

/// The native watch of view `label` swallowed a press of the person's: tells the listeners its chat. Cheap and
/// non-blocking (it runs on the view's input thread); a view of no chat is not reported.
pub fn refused(label: &str) {
    let Some(owner) = registry().owners.get(label).cloned() else {
        return;
    };
    // Called without the lock held: a listener may read the registry.
    let listeners: Vec<Listener> = LISTENERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let gone: Vec<Listener> = listeners
        .into_iter()
        .filter(|listener| !listener(&owner))
        .collect();
    if !gone.is_empty() {
        LISTENERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|listener| !gone.iter().any(|g| Arc::ptr_eq(g, listener)));
    }
}

/// The engine's per-view watch's report of a swallowed press, for the native watch.
pub fn refusal(label: &str) -> impl Fn() + Send + Sync + 'static {
    let label = label.to_owned();
    move || refused(&label)
}

/// The engine's per-view question, for the native watch.
pub fn reader(label: &str) -> impl Fn() -> bool + Send + Sync + 'static {
    let label = label.to_owned();
    move || blocked(&label)
}

#[cfg(test)]
mod tests {
    //! PREDICT: without the registry no view is ever blocked (the person's input reaches the page while the agent
    //! works).
    use super::{assign, blocked, forget, on_refused, reader, refusal, set};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn a_swallowed_press_reaches_the_listeners_as_its_chat() {
        // PREDICT: without the report a press on the blocked page reaches no one: the view never sees it and the app
        // window's focus does not move, so keys typed next still answer the window's focused ask card.
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let alive = Arc::new(Mutex::new(true));
        {
            let (seen, alive) = (seen.clone(), alive.clone());
            on_refused(move |owner| {
                if !*alive.lock().unwrap() {
                    return false;
                }
                if owner.starts_with("pr-chat-") {
                    seen.lock().unwrap().push(owner.to_owned());
                }
                true
            });
        }
        assign("pr-a-1", "pr-chat-a");
        let press = refusal("pr-a-1");
        press();
        refusal("pr-unknown")();
        assert_eq!(
            *seen.lock().unwrap(),
            ["pr-chat-a"],
            "the view's chat, once; a view of no chat is not reported"
        );
        *alive.lock().unwrap() = false;
        press();
        *alive.lock().unwrap() = true;
        press();
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "a listener that answered false is dropped"
        );
        forget("pr-a-1");
    }

    #[test]
    fn every_view_of_the_owner_is_blocked_until_the_lease_ends() {
        assign("pb-a-1", "pb-chat-a");
        assign("pb-a-2", "pb-chat-a");
        assign("pb-b-1", "pb-chat-b");
        set("pb-chat-a", Some(Instant::now() + Duration::from_secs(30)));
        assert!(blocked("pb-a-1") && blocked("pb-a-2"));
        assert!(
            !blocked("pb-b-1"),
            "another chat's browser is never blocked"
        );
        assert!(!blocked("pb-unknown"));
        set("pb-chat-a", None);
        assert!(!blocked("pb-a-1"));
    }

    #[test]
    fn a_lease_ends_by_itself_at_its_deadline() {
        assign("pb-c-1", "pb-chat-c");
        set("pb-chat-c", Some(Instant::now()));
        assert!(!blocked("pb-c-1"));
    }

    #[test]
    fn a_view_mounted_after_the_lease_is_blocked_and_a_closed_one_is_not() {
        set("pb-chat-d", Some(Instant::now() + Duration::from_secs(30)));
        let read = reader("pb-d-2");
        assert!(!read());
        assign("pb-d-2", "pb-chat-d");
        assert!(read());
        forget("pb-d-2");
        assert!(!read());
    }
}
