//! App session ownership across data folders that share a Vibe home.
//! Core/channel lifecycle owns OS locks; shared pointers identify the instance and pid holding each lock.
//! Persistent lock files arbitrate ownership and release automatically when an owner exits.

use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorCode};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pointer {
    // Shipped readers require this field to decode the holder pid.
    record: PathBuf,
    instance_id: String,
    pid: u32,
}

/// A claim held across one operation, and retained while its channel is open.
pub(crate) struct Lease {
    lock: Option<File>,
    id: String,
    claims: Arc<Claims>,
}

#[derive(Default)]
struct Claims {
    entries: Mutex<HashMap<String, Weak<Lease>>>,
    released: Condvar,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut claims = self
            .claims
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Weak::upgrade stops succeeding before Drop starts. Publish absence only after closing the OS handle.
        if let Some(lock) = self.lock.take() {
            let _ = lock.unlock();
            drop(lock);
        }
        claims.remove(&self.id);
        drop(claims);
        self.claims.released.notify_all();
    }
}

fn probe(lock: File) -> Result<(), TryLockError> {
    lock.try_lock()?;
    let result = lock.unlock().map_err(TryLockError::Error);
    drop(lock);
    result
}

/// Optional disk leases, enabled when the app configures session bookkeeping.
pub(crate) struct Leases {
    root: PathBuf,
    data: Option<PathBuf>,
    instance: String,
    claims: Arc<Claims>,
    active: Mutex<HashMap<String, Arc<Lease>>>,
    #[cfg(test)]
    before_wait: Mutex<Option<tests::Pause>>,
}

fn failure(error: impl std::fmt::Display) -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        format!("Could not check session ownership: {error}"),
    )
}

/// Longest a claim waits for a forked child to close this instance's inherited lock handle, in milliseconds. A child process
/// started at that moment shares the lock file until it runs its own program, which can take a while on a busy
/// machine; the wait applies only when the pointer names this instance, so another window is refused at once.
const RELEASE_WAIT_MS: u32 = 250;

impl Leases {
    pub(crate) fn new(home: &Path, data: Option<&Path>) -> Result<Self, CoreError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(failure)?;
        Ok(Self {
            root: home.join("vibe-work-leases"),
            data: data.map(|path| path.join("leases")),
            instance: blake3::hash(&random).to_hex().to_string(),
            claims: Arc::default(),
            active: Mutex::new(HashMap::new()),
            #[cfg(test)]
            before_wait: Mutex::new(None),
        })
    }

    fn key(id: &str) -> String {
        blake3::hash(id.as_bytes()).to_hex().to_string()
    }

    fn lock(&self, id: &str) -> Result<File, CoreError> {
        vw_platform::ensure_private_dir(&self.root).map_err(failure)?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(format!("{}.lock", Self::key(id))))
            .map_err(failure)
    }

    fn pointer(&self, id: &str) -> PathBuf {
        self.root.join(format!("{}.json", Self::key(id)))
    }

    fn read_pointer(&self, id: &str) -> Option<Pointer> {
        std::fs::read(self.pointer(id))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Pointer>(&bytes).ok())
    }

    fn holder(&self, id: &str) -> u32 {
        self.read_pointer(id).map_or(0, |pointer| pointer.pid)
    }

    /// True when the busy lock is this instance's own lease being released: no live claim here, but the pointer
    /// still names this instance. In-process release is ordered; a child may inherit the handle until exec.
    fn releasing(&self, id: &str) -> bool {
        self.read_pointer(id)
            .is_some_and(|pointer| pointer.instance_id == self.instance)
    }

    /// Checks without acquiring ownership or rewriting a foreign pointer.
    pub(crate) fn foreign(&self, id: &str) -> Result<Option<u32>, CoreError> {
        if self.data.is_none() {
            return Ok(None);
        }
        // A probe also briefly owns the OS lock. Serialize it with our claims and other probes so
        // this instance cannot mistake its own ownership check for a foreign owner.
        let claims = self
            .claims
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if claims.contains_key(id) {
            return Ok(None);
        }
        let lock = self.lock(id)?;
        match probe(lock) {
            Ok(()) => Ok(None),
            Err(TryLockError::WouldBlock) if self.releasing(id) => Ok(None),
            Err(TryLockError::WouldBlock) => Ok(Some(self.holder(id))),
            Err(TryLockError::Error(error)) => Err(failure(error)),
        }
    }

    /// Acquires before any mutation or resume. An unlocked file permits a new owner.
    pub(crate) fn claim(&self, id: &str) -> Result<Option<Arc<Lease>>, CoreError> {
        let Some(data) = &self.data else {
            return Ok(None);
        };
        let mut claims = self
            .claims
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut lock = None;
        let mut waited = 0;
        loop {
            if let Some(previous) = claims.get(id) {
                if let Some(lease) = previous.upgrade() {
                    return Ok(Some(lease));
                }
                // The last Arc is in Drop. Its ordered close wakes this claim without polling the pointer file.
                claims = self
                    .claims
                    .released
                    .wait(claims)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let Some(file) = &lock else {
                lock = Some(self.lock(id)?);
                continue;
            };
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if waited < RELEASE_WAIT_MS && self.releasing(id) => {
                    // Only a fork-until-exec handle can outlive the ordered in-process close. Other chats proceed.
                    drop(claims);
                    #[cfg(test)]
                    tests::pause(&self.before_wait);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    waited += 1;
                    claims = self
                        .claims
                        .entries
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(crate::session_holder::held_refusal(self.holder(id)));
                }
                Err(TryLockError::Error(error)) => return Err(failure(error)),
            }
        }
        let name = format!("{}-{}", Self::key(id), self.instance);
        let record = data.join(format!(
            "{}.json",
            &blake3::hash(name.as_bytes()).to_hex()[..32]
        ));
        vw_platform::replace_private(
            &self.pointer(id),
            &serde_json::to_vec(&Pointer {
                record,
                instance_id: self.instance.clone(),
                pid: std::process::id(),
            })
            .map_err(failure)?,
        )
        .map_err(failure)?;
        let lease = Arc::new(Lease {
            lock,
            id: id.to_owned(),
            claims: Arc::clone(&self.claims),
        });
        claims.insert(id.to_owned(), Arc::downgrade(&lease));
        Ok(Some(lease))
    }

    /// A channel keeps its claim until its acknowledged close or core shutdown.
    pub(crate) fn retain(&self, id: &str, lease: Option<Arc<Lease>>) {
        if let Some(lease) = lease {
            self.active
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.to_owned(), lease);
        }
    }

    pub(crate) fn release(&self, id: &str) {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }

    pub(crate) fn release_all(&self) {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

#[cfg(test)]
#[path = "session_lease_tests.rs"]
mod tests;
