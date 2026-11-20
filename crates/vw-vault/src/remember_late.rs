//! The OS credential store opened late: when the system keyring was not there at start (Linux without a Secret
//! Service, a keyring daemon not started yet), the app runs on the keep-nothing fallback, and every use of the store
//! tries the OS store again (at most once per [`RETRY_EVERY`]). Once it opens, it is used from then on, so a
//! keyring the person sets up while Vibe Work runs serves "remember this device" and the browser connection's
//! identity without a restart.
//!
//! Depends on: [`super::remember`] (the store trait and the fallback). Called by: the app's startup (product launch).
//! Invariant: the fallback never keeps anything, so nothing needs to move when the OS store appears.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vw_platform::sync::LockRecover;

use crate::remember::{DeviceSecretStore, MemoryDeviceStore};
use crate::{SecretString, VaultError};

/// The shortest time between two attempts to open the OS store.
pub const RETRY_EVERY: Duration = Duration::from_secs(2);

/// Opens the OS store (`Err`: still not available).
pub type Opener = Box<dyn Fn() -> Result<Arc<dyn DeviceSecretStore>, VaultError> + Send + Sync>;

/// The OS store once it opened, and when it was last tried.
type State = (Option<Arc<dyn DeviceSecretStore>>, Option<Instant>);

/// The OS store once it opened, else the keep-nothing fallback (module docs).
pub struct LateDeviceStore {
    open: Opener,
    state: Mutex<State>,
    fallback: MemoryDeviceStore,
    retry_every: Duration,
}

impl LateDeviceStore {
    /// A store that could not open the OS store at start (that attempt counts as the first try).
    #[must_use]
    pub fn new(open: Opener) -> Self {
        Self::with_retry(open, RETRY_EVERY)
    }

    /// As [`Self::new`], with the time between two attempts (tests).
    #[must_use]
    pub fn with_retry(open: Opener, retry_every: Duration) -> Self {
        Self {
            open,
            state: Mutex::new((None, Some(Instant::now()))),
            fallback: MemoryDeviceStore::temporary(),
            retry_every,
        }
    }

    /// The OS store when it is open or opens now; `None` while it is still unavailable.
    fn os(&self) -> Option<Arc<dyn DeviceSecretStore>> {
        {
            let mut state = self.state.lock_or_recover();
            if let Some(store) = &state.0 {
                return Some(Arc::clone(store));
            }
            if state.1.is_some_and(|at| at.elapsed() < self.retry_every) {
                return None;
            }
            state.1 = Some(Instant::now());
        }
        // Opened outside the lock: on Linux an open can wait for its deadline.
        let opened = (self.open)().ok()?;
        let mut state = self.state.lock_or_recover();
        Some(Arc::clone(state.0.get_or_insert(opened)))
    }
}

impl DeviceSecretStore for LateDeviceStore {
    fn get(&self, account: &str) -> Result<Option<SecretString>, VaultError> {
        match self.os() {
            Some(store) => store.get(account),
            None => self.fallback.get(account),
        }
    }

    fn set(&self, account: &str, secret: &SecretString) -> Result<(), VaultError> {
        match self.os() {
            Some(store) => store.set(account, secret),
            None => self.fallback.set(account, secret),
        }
    }

    fn delete(&self, account: &str) -> Result<(), VaultError> {
        match self.os() {
            Some(store) => store.delete(account),
            None => self.fallback.delete(account),
        }
    }
}

#[cfg(test)]
#[path = "remember_late_tests.rs"]
mod tests;
