//! Lock access that survives a poisoned lock: a panic while a guard was held leaves the data as the panicking code
//! left it, and every caller in the workspace chose to keep using that data rather than panic again. One helper
//! instead of `.lock().unwrap_or_else(PoisonError::into_inner)` at every site; the behaviour is the same.
//!
//! Depends on: std only. Called by: every workspace crate that holds a `std::sync` lock.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// `lock` that takes the guard of a poisoned `Mutex` instead of returning the poison error.
pub trait LockRecover<T: ?Sized> {
    /// Acquires the lock, poisoned or not.
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> LockRecover<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Shared and exclusive access to an `RwLock`, poisoned or not.
pub trait RwLockRecover<T: ?Sized> {
    /// Shared access.
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T>;
    /// Exclusive access.
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockRecover<T> for RwLock<T> {
    fn read_or_recover(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }
    fn write_or_recover(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::{LockRecover, RwLockRecover};
    use std::sync::{Arc, Mutex, RwLock};

    #[test]
    fn a_poisoned_lock_still_gives_its_data() {
        let mutex = Arc::new(Mutex::new(1));
        let rw = Arc::new(RwLock::new(2));
        let (m, r) = (Arc::clone(&mutex), Arc::clone(&rw));
        let _ = std::thread::spawn(move || {
            let _held = m.lock().unwrap();
            let _write = r.write().unwrap();
            panic!("poison both");
        })
        .join();
        assert!(mutex.is_poisoned() && rw.is_poisoned());
        *mutex.lock_or_recover() += 1;
        assert_eq!(*mutex.lock_or_recover(), 2);
        *rw.write_or_recover() += 1;
        assert_eq!(*rw.read_or_recover(), 3);
    }
}
