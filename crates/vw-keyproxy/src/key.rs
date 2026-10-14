//! The real key as the proxy sees it: a watch cell the key service updates (stored, replaced,
//! removed, vault locked or unlocked). Rotation needs no Vibe restart: the next request uses the
//! new value.
//!
//! Depends on: tokio (watch), zeroize. Called by: `server`, vw-keys.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use zeroize::Zeroizing;

/// Why no key is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyNeed {
    /// The vault is locked: the user must unlock it.
    Unlock,
    /// No key was set up yet.
    Setup,
}

/// The key cell's state.
#[derive(Clone)]
pub enum KeyState {
    /// The key (only the proxy and the test call read it).
    Ready(Arc<Zeroizing<String>>),
    /// Not available, and why.
    Missing(KeyNeed),
}

impl std::fmt::Debug for KeyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready(_) => f.write_str("Ready(<redacted>)"),
            Self::Missing(need) => write!(f, "Missing({need:?})"),
        }
    }
}

/// Shared, observable key state.
#[derive(Clone)]
pub struct KeyCell {
    tx: Arc<watch::Sender<KeyState>>,
}

impl Default for KeyCell {
    fn default() -> Self {
        Self::new(KeyState::Missing(KeyNeed::Setup))
    }
}

impl KeyCell {
    /// A cell starting in `state`.
    #[must_use]
    pub fn new(state: KeyState) -> Self {
        Self {
            tx: Arc::new(watch::channel(state).0),
        }
    }

    /// Sets the key (setup, replace, unlock).
    pub fn set_key(&self, key: Zeroizing<String>) {
        self.tx.send_replace(KeyState::Ready(Arc::new(key)));
    }

    /// Marks the key unavailable (lock, remove).
    pub fn clear(&self, need: KeyNeed) {
        self.tx.send_replace(KeyState::Missing(need));
    }

    /// The current state.
    #[must_use]
    pub fn current(&self) -> KeyState {
        self.tx.borrow().clone()
    }

    /// The key, waiting up to `limit` for one; calls `on_wait` once if it has to wait.
    ///
    /// # Errors
    /// The last [`KeyNeed`] when the wait ends without a key.
    pub async fn wait_ready(
        &self,
        limit: Duration,
        on_wait: impl FnOnce(KeyNeed),
    ) -> Result<Arc<Zeroizing<String>>, KeyNeed> {
        let mut rx = self.tx.subscribe();
        let need = match &*rx.borrow_and_update() {
            KeyState::Ready(key) => return Ok(Arc::clone(key)),
            KeyState::Missing(need) => *need,
        };
        on_wait(need);
        let waited = tokio::time::timeout(limit, async {
            loop {
                if rx.changed().await.is_err() {
                    return None;
                }
                if let KeyState::Ready(key) = &*rx.borrow_and_update() {
                    return Some(Arc::clone(key));
                }
            }
        })
        .await;
        match waited {
            Ok(Some(key)) => Ok(key),
            _ => Err(match self.current() {
                KeyState::Missing(need) => need,
                KeyState::Ready(_) => need,
            }),
        }
    }
}
