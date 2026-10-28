//! App session ownership across data folders that share a Vibe home.
//! Core/channel lifecycle owns OS locks; shared pointers identify the instance and pid holding each lock.
//! Persistent lock files arbitrate ownership and release automatically when an owner exits.

use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};

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
    _lock: File,
}

/// Optional disk leases, enabled when the app configures session bookkeeping.
pub(crate) struct Leases {
    root: PathBuf,
    data: Option<PathBuf>,
    instance: String,
    claims: Mutex<HashMap<String, Weak<Lease>>>,
    active: Mutex<HashMap<String, Arc<Lease>>>,
}

fn failure(error: impl std::fmt::Display) -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        format!("Could not check session ownership: {error}"),
    )
}

/// Longest a claim waits for this instance's own lease to finish its release, in milliseconds. A child process
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
            claims: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
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
    /// still names this instance. Dropping a lease ends its claim before its lock file is closed, so a claim or probe
    /// on another thread can find the lock busy for that moment.
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
        let claims = self.claims.lock().unwrap_or_else(PoisonError::into_inner);
        if claims.get(id).and_then(Weak::upgrade).is_some() {
            return Ok(None);
        }
        let lock = self.lock(id)?;
        match lock.try_lock() {
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
        let mut claims = self.claims.lock().unwrap_or_else(PoisonError::into_inner);
        claims.retain(|_, lease| lease.strong_count() > 0);
        if let Some(lease) = claims.get(id).and_then(Weak::upgrade) {
            return Ok(Some(lease));
        }
        let lock = self.lock(id)?;
        // Our own lease's release finishes within moments; wait for it (at most RELEASE_WAIT_MS) instead of reporting
        // our own process as another window. A lock another instance holds is refused at once.
        let mut waited = 0;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if waited < RELEASE_WAIT_MS && self.releasing(id) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    waited += 1;
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(crate::session_holder::held_refusal(self.holder(id)));
                }
                Err(TryLockError::Error(error)) => return Err(failure(error)),
            }
        }
        let lease = Arc::new(Lease { _lock: lock });
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
mod tests {
    use super::*;

    /// A claim racing this instance's own lease release (the lock file still open for a moment after the last
    /// handle dropped) must wait for it, never report the session as held by another window with our own pid.
    #[test]
    fn a_claim_during_our_own_release_waits_instead_of_refusing() {
        let dir = std::env::temp_dir().join(format!("vw-lease-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let leases = Leases::new(&dir.join("home"), Some(&dir.join("data"))).unwrap();
        drop(leases.claim("s1").unwrap());
        // The released lease's lock file is still open: hold the OS lock the way its closing handle does. A child
        // process another test starts at this moment can share the released lock until it runs its own program, so
        // the setup waits (at most 5 s) for the lock to come free first.
        let closing = leases.lock("s1").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while let Err(error) = closing.try_lock() {
            assert!(
                matches!(error, TryLockError::WouldBlock) && std::time::Instant::now() < deadline,
                "the released lock did not come free: {error:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            leases.foreign("s1").unwrap(),
            None,
            "our own release is not another window"
        );
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            drop(closing);
        });
        assert!(
            leases.claim("s1").unwrap().is_some(),
            "the claim waits for our release"
        );
        release.join().unwrap();

        // Another instance (another window sharing the Vibe home) is still refused at once.
        let other = Leases::new(&dir.join("home"), Some(&dir.join("other-data"))).unwrap();
        let held = leases.claim("s2").unwrap();
        let refused = other.claim("s2").err().expect("held by the first instance");
        assert_eq!(refused.server_code.as_deref(), Some("sessionHeld"));
        assert_eq!(other.foreign("s2").unwrap(), Some(std::process::id()));
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
