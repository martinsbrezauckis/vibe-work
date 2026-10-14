//! The core's credential table: one random credential per session (handed to that session's MCP server in its
//! environment), pending credentials for sessions whose id Vibe has not returned yet, and the hook's
//! credential.
//!
//! Depends on: getrandom, subtle. Called by: vw-core-api (`agent_tools`: injection in the transport, the
//! listener's authentication), tests.
//! Invariants:
//! - A credential is 32 random bytes as 64 lowercase hex characters, new on every core start (in memory only).
//! - Lookup compares every stored credential in constant time (no early exit on the first match), so the
//!   answer time does not depend on how much of a guess was right.
//! - A pending credential authorizes nothing until it is bound to a session; a revoked one nothing ever.
//! - The lock is never held across an await (plain methods).

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use subtle::ConstantTimeEq;

/// What a credential stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Holder {
    /// The MCP server of this session.
    Session(String),
    /// Minted for a session start whose id is not known yet: authorizes nothing.
    Pending,
    /// The `pre_tool` hook (asks `kill_check` and `git_check` only).
    Hook,
}

/// Why a credential could not be minted (the OS random source failed).
#[derive(Debug)]
pub struct RandomError(pub String);

impl std::fmt::Display for RandomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "random source failed: {}", self.0)
    }
}

impl std::error::Error for RandomError {}

/// A fresh credential: 32 random bytes, hex.
///
/// # Errors
/// [`RandomError`] when the OS random source fails.
pub fn new_credential() -> Result<String, RandomError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| RandomError(e.to_string()))?;
    let mut text = String::with_capacity(64);
    for byte in bytes {
        text.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        text.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    Ok(text)
}

/// The table. Share it behind an `Arc`.
#[derive(Default)]
pub struct CredentialTable {
    entries: Mutex<BTreeMap<u64, (String, Holder)>>,
    next: Mutex<u64>,
}

/// Handle of one table entry (to bind or revoke a pending credential).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId(u64);

impl CredentialTable {
    fn entries(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, (String, Holder)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn insert(&self, holder: Holder) -> Result<(EntryId, String), RandomError> {
        let credential = new_credential()?;
        let id = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        self.entries().insert(id, (credential.clone(), holder));
        Ok((EntryId(id), credential))
    }

    /// The credential of `session_id`, minted on first use (the same one on every later call, so Vibe's
    /// MCP pool key stays the same for the session).
    ///
    /// # Errors
    /// [`RandomError`] when a new credential cannot be minted.
    pub fn for_session(&self, session_id: &str) -> Result<String, RandomError> {
        if let Some((credential, _)) = self
            .entries()
            .values()
            .find(|(_, h)| *h == Holder::Session(session_id.to_owned()))
        {
            return Ok(credential.clone());
        }
        self.insert(Holder::Session(session_id.to_owned()))
            .map(|(_, c)| c)
    }

    /// A new pending credential (bind it with [`CredentialTable::bind`] once the session id is known).
    ///
    /// # Errors
    /// [`RandomError`] when a new credential cannot be minted.
    pub fn pending(&self) -> Result<(EntryId, String), RandomError> {
        self.insert(Holder::Pending)
    }

    /// A new hook credential (replaces none; the core mints one per start).
    ///
    /// # Errors
    /// [`RandomError`] when a new credential cannot be minted.
    pub fn hook(&self) -> Result<String, RandomError> {
        self.insert(Holder::Hook).map(|(_, c)| c)
    }

    /// Binds a pending credential to `session_id`. When the session already has a credential the pending one
    /// becomes a second credential of the same session (both MCP server configs may run).
    pub fn bind(&self, entry: EntryId, session_id: &str) {
        if let Some((_, holder)) = self.entries().get_mut(&entry.0)
            && *holder == Holder::Pending
        {
            *holder = Holder::Session(session_id.to_owned());
        }
    }

    /// Removes one entry (a start that failed).
    pub fn revoke(&self, entry: EntryId) {
        self.entries().remove(&entry.0);
    }

    /// Removes every credential of `session_id` (session deleted).
    pub fn revoke_session(&self, session_id: &str) {
        self.entries()
            .retain(|_, (_, h)| *h != Holder::Session(session_id.to_owned()));
    }

    /// The conversation of session `old` now runs as `new` (Vibe handed it off: `/clear`, a rewind into a new
    /// session, a compaction with a new id). Every credential of `old` now stands for `new`: Vibe starts
    /// the new session's MCP server from the old session's derived config (`_unified_harness_backend_adapter.py`
    /// `clear_history`/`rewind_fork`: `source._context.derive(...)`), so that server keeps presenting the old
    /// credential, and it must act for the session the window shows. Returns how many credentials moved. A later
    /// resume of `old` mints it a fresh credential (`for_session`).
    #[must_use = "the moved items tell what followed the hand-off"]
    pub fn hand_off(&self, old: &str, new: &str) -> usize {
        let from = Holder::Session(old.to_owned());
        let mut moved = 0;
        for (_, holder) in self.entries().values_mut() {
            if *holder == from {
                *holder = Holder::Session(new.to_owned());
                moved += 1;
            }
        }
        moved
    }

    /// Who `presented` belongs to. `None` for an unknown or revoked credential. Every stored entry is compared
    /// in constant time.
    #[must_use]
    pub fn holder(&self, presented: &str) -> Option<Holder> {
        let entries = self.entries();
        let mut found: Option<Holder> = None;
        for (credential, holder) in entries.values() {
            let same = credential.len() == presented.len()
                && bool::from(credential.as_bytes().ct_eq(presented.as_bytes()));
            if same && found.is_none() {
                found = Some(holder.clone());
            }
        }
        found
    }
}
