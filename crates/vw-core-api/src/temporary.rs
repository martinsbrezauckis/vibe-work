//! Temporary sessions are discarded after use.
//!
//! Vibe 2.26.0 has no per-session "don't save": `SessionKind.EPHEMERAL` is promoted to a stored session on its
//! first turn, and `session_logging.enabled = false` is process-wide. So a temporary session is a
//! normal Vibe session that Vibe Work deletes for the user: when Vibe Work quits ([`delete_all`], from
//! `Core::shutdown`), or at the next start when the app crashed before ([`sweep_leftovers`]). The user may delete it
//! earlier like any session. Until then it is stored by Vibe like any other (the Vibe CLI lists it when sessions are
//! shared with the CLI, the default), which the UI says in words.
//!
//! The ids are kept in memory and in `<sessions_dir>/temporary.json` (user-only, written whole), so a crash cannot
//! turn a temporary session into a kept one. That promise only holds while the record is trustworthy, so:
//! - a session is registered ([`TemporarySessions::add`]) before its start is answered, and a start whose record
//!   cannot be written fails: the new session is deleted again ([`Core::refuse_unrecorded`]) instead of living on as
//!   an ordinary chat that the window believes is temporary;
//! - a record that cannot be read or does not parse is never overwritten: it is moved aside unchanged
//!   (`temporary.json.bad-<ms>`), logged at error level, and the ids that are still whole in it stay on the list. When
//!   it cannot be moved aside either, nothing is written over it and no new temporary session can start until it is
//!   fixed or removed.
//!
//! Depends on: vw-gate (`Requester::Core`), `dispatch` (`forward`, so a delete ends the session in the one place,
//! `session_end.rs`), vw-platform (user-only file, `set_aside`), vw-projects (`now_ms`). Called by: `dispatch`
//! (`startSession {temporary: true}`, `sessionTools temporary`), `core` (start, shutdown), `session_end` (a deleted
//! session leaves the list), `session_tools` (`startTemporary`), `subagents_setting` (a short-lived chat for the
//! Subagents setting is recorded here and removed with [`Core::discard_unused`]).
//! Invariants:
//! - Only sessions started with `temporary: true` are on the list; nothing else is ever deleted here. Ids salvaged from
//!   a damaged record come only from the leading whole entries of its top-level list, never from other text.
//! - The deletion asks no dialog ([`vw_gate::Requester::Core`]): the consent is the user's "Temporary chat" choice.
//! - Each deletion is bounded ([`DELETE_WAIT`]); a failure keeps the id on the list for the next start. Vibe's "not
//!   found" is not a failure: the session is gone (a chat nobody wrote in was never stored).
//! - When a folder is configured, an id is on the list only if the record on disk names it too ([`TemporarySessions::add`]
//!   rolls the id back when the write fails).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tracing::{error, info, warn};

use crate::command::Origin;
use crate::core::{Core, Shared};
use crate::error::CoreError;

/// Longest one deletion may take at quit or start.
const DELETE_WAIT: Duration = Duration::from_secs(10);
/// The record's file name inside `sessions_dir`.
const FILE: &str = "temporary.json";

/// Why a temporary session could not be recorded; the text goes to the log and into the error the start answers.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct NotRecorded(String);

/// The temporary sessions of this app (in memory, mirrored to disk when a folder is configured).
#[derive(Default)]
pub(crate) struct TemporarySessions {
    ids: Mutex<BTreeSet<String>>,
    file: Option<PathBuf>,
    /// Set at load when the record on disk is damaged and could not be moved aside: it has to stay as it is, so no
    /// write is attempted and every [`TemporarySessions::add`] fails with this text.
    frozen: Option<String>,
}

/// Deletes the temporary sessions an earlier run left (it crashed before quitting). Runs once, in the background;
/// holds only a weak reference, so a core shut down meanwhile is not kept alive. Called by `Core::start`.
pub(crate) fn spawn_sweep(shared: &Arc<Shared>) {
    if shared.temporary.ids().is_empty() {
        return;
    }
    let weak = Arc::downgrade(shared);
    tokio::spawn(async move {
        let Some(shared) = weak.upgrade() else { return };
        Core { shared }.delete_temporary_all().await;
    });
}

/// Reads the record. Returns the ids that can be used and, when the record is not a clean list of ids, what is
/// wrong with it. A missing file is a first run (or a clean quit), not damage.
fn read_record(path: &Path) -> (BTreeSet<String>, Option<String>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (BTreeSet::new(), None),
        Err(e) => return (BTreeSet::new(), Some(format!("it could not be read ({e})"))),
    };
    match serde_json::from_slice::<Vec<Value>>(&bytes) {
        Ok(items) => {
            let mut ids = BTreeSet::new();
            let mut odd = 0_usize;
            for item in items {
                match item {
                    Value::String(id) if !id.is_empty() => {
                        ids.insert(id);
                    }
                    _ => odd += 1,
                }
            }
            let damage =
                (odd > 0).then(|| format!("it holds {odd} entries that are not session ids"));
            (ids, damage)
        }
        Err(e) => (salvage(&bytes), Some(format!("it does not parse ({e})"))),
    }
}

/// The whole ids at the start of a list that no longer parses (cut off, or damaged further on). Only the leading run
/// of complete strings of a top-level array counts; nothing is taken from text that is not such a list, because an
/// id that was never on it must not be deleted.
fn salvage(bytes: &[u8]) -> BTreeSet<String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or_default(),
    };
    let mut ids = BTreeSet::new();
    let Some(mut rest) = text.trim_start().strip_prefix('[') else {
        return ids;
    };
    loop {
        rest = rest.trim_start();
        let mut parsed = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        // A string cut off before its closing quote is not an id.
        let Some(Ok(id)) = parsed.next() else { break };
        let used = parsed.byte_offset();
        if !id.is_empty() {
            ids.insert(id);
        }
        rest = rest[used..].trim_start();
        match rest.strip_prefix(',') {
            Some(next) => rest = next,
            None => break,
        }
    }
    ids
}

impl TemporarySessions {
    /// The list for `sessions_dir` (`None`: memory only), with the ids a previous run left in its file. A damaged
    /// record is moved aside and reported (see the module header).
    pub(crate) fn load(sessions_dir: Option<PathBuf>) -> Self {
        let file = sessions_dir.map(|dir| dir.join(FILE));
        let (ids, damage) = file
            .as_deref()
            .map_or_else(|| (BTreeSet::new(), None), read_record);
        let mut list = Self {
            ids: Mutex::new(ids),
            file,
            frozen: None,
        };
        if let Some(damage) = damage {
            list.set_damaged_record_aside(&damage);
        }
        list
    }

    /// Moves the damaged record out of the way, unchanged, and writes the salvaged ids as the new record. When the
    /// move fails the record is left alone and the list is frozen (see the `frozen` field).
    fn set_damaged_record_aside(&mut self, damage: &str) {
        let Some(path) = self.file.clone() else {
            return;
        };
        let kept = self.ids().len();
        if let Some(aside) = vw_platform::set_aside(&path, vw_projects::now_ms()) {
            error!(
                file = ?aside, damage, kept,
                "temporary sessions: the record was damaged and has been moved aside unchanged; the session ids still whole in it were kept"
            );
            let ids = self.ids.lock().unwrap_or_else(PoisonError::into_inner);
            if !ids.is_empty()
                && let Err(error) = self.write(&ids)
            {
                error!(%error, "temporary sessions: the kept ids could not be written to a new record");
            }
        } else {
            error!(
                file = ?path, damage, kept,
                "temporary sessions: the record is damaged and could not be moved aside; it is left untouched and no temporary session can be started until it is fixed or removed"
            );
            self.frozen = Some(format!(
                "the record {} is damaged ({damage}) and could not be moved aside",
                path.display()
            ));
        }
    }

    /// The ids now, sorted.
    pub(crate) fn ids(&self) -> Vec<String> {
        self.ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Fails at once when the record is known to be unusable (damaged and not movable, see the module header), so a
    /// temporary start does not create a session it could only delete again. A record that merely cannot be written
    /// right now is caught by [`TemporarySessions::add`] instead.
    ///
    /// # Errors
    /// [`CoreError`] (`internal`) naming the record, for the start to answer with.
    pub(crate) fn ensure_recordable(&self) -> Result<(), CoreError> {
        match &self.frozen {
            Some(why) => Err(CoreError::new(
                crate::ErrorCode::Internal,
                format!(
                    "The temporary chat was not started: its record cannot be used ({why}). Nothing was created."
                ),
            )),
            None => Ok(()),
        }
    }

    /// Adds a session and writes the record before returning (a crash right after still deletes it).
    ///
    /// # Errors
    /// [`NotRecorded`] when the record cannot be written (or must not be, see the module header). The id is then not
    /// on the list either, so the caller must not treat the session as temporary: it has to end it.
    pub(crate) fn add(&self, id: &str) -> Result<(), NotRecorded> {
        let mut ids = self.ids.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = ids.insert(id.to_owned());
        if let Err(error) = self.write(&ids) {
            if fresh {
                ids.remove(id);
            }
            return Err(error);
        }
        Ok(())
    }

    /// Removes a session (it was deleted). A record that cannot be updated only means a stale id: the next start
    /// finds no such session, drops it and goes on.
    pub(crate) fn remove(&self, id: &str) {
        let mut ids = self.ids.lock().unwrap_or_else(PoisonError::into_inner);
        if ids.remove(id)
            && let Err(error) = self.write(&ids)
        {
            warn!(session_id = id, %error, "temporary sessions: the record still names a deleted session");
        }
    }

    /// Writes the whole record, replacing the file atomically. Memory-only (no folder configured) always succeeds.
    fn write(&self, ids: &BTreeSet<String>) -> Result<(), NotRecorded> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(why) = &self.frozen {
            return Err(NotRecorded(why.clone()));
        }
        let list: Vec<&String> = ids.iter().collect();
        let bytes = serde_json::to_vec(&list)
            .map_err(|e| NotRecorded(format!("the list could not be encoded ({e})")))?;
        if let Some(dir) = file.parent() {
            vw_platform::ensure_private_dir(dir).map_err(|e| {
                NotRecorded(format!("the folder {} cannot be used ({e})", dir.display()))
            })?;
        }
        vw_platform::replace_private(file, &bytes)
            .map_err(|e| NotRecorded(format!("{} cannot be written ({e})", file.display())))
    }
}

impl Core {
    /// Undoes the start of a session whose record could not be written, so it cannot live on as an ordinary chat
    /// that the window shows as temporary. Returns the error the start answers with.
    ///
    /// The new session has run no turn, so deleting it loses nothing. If Vibe cannot delete it, the session stays in
    /// the list as an ordinary (empty) chat and the error says so.
    pub(crate) async fn refuse_unrecorded(&self, id: &str, why: &NotRecorded) -> CoreError {
        error!(session_id = id, %why, "temporary session: the record could not be written; deleting the new session again");
        let removed = self.discard_unused(id).await;
        let message = if removed {
            format!(
                "The temporary chat was not started: its record could not be saved ({why}). Nothing was kept."
            )
        } else {
            error!(
                session_id = id,
                "temporary session: the new session could not be deleted again"
            );
            format!(
                "The temporary chat was not started: its record could not be saved ({why}). The new empty chat could \
                 not be removed either and stays in the list as an ordinary chat."
            )
        };
        CoreError::new(crate::ErrorCode::Internal, message)
    }

    /// Deletes a session the core started for its own use and that has run no turn, without a dialog, and finishes its
    /// bookkeeping (store, projects, tools). True when it is gone. Vibe stores a session on its first turn, so for
    /// a session nobody wrote in it may answer "not found": no confirmed delete then ended the session in the core,
    /// and the bookkeeping is finished here instead.
    pub(crate) async fn discard_unused(&self, id: &str) -> bool {
        let removed = self.delete_temporary(id).await;
        if removed && self.shared.store.view(id).is_some() {
            crate::session_end::deleted(&self.shared, id).await;
        }
        removed
    }

    /// Deletes one temporary session without a dialog (see the module header). True when Vibe confirmed it.
    async fn delete_temporary(&self, id: &str) -> bool {
        let params = json!({"sessionId": id});
        let token = match self
            .shared
            .gate
            .request_confirmation_as("session/delete", &params, vw_gate::Requester::Core)
            .await
        {
            Ok(token) => token,
            Err(error) => {
                warn!(session_id = id, %error, "temporary session: delete not confirmed");
                return false;
            }
        };
        let call = self.forward(
            Origin::Ui,
            "session/delete",
            params,
            Some(token.as_str().to_owned()),
        );
        match tokio::time::timeout(DELETE_WAIT, call).await {
            Ok(Ok(_)) => true,
            Ok(Err(error))
                if error.code == crate::ErrorCode::NotFound
                    || error.server_code.as_deref() == Some("not_found") =>
            {
                // Gone already: deleted in the CLI, or never stored (Vibe stores a session from its first turn, so
                // a temporary chat nobody wrote in answers "not found" to its own delete). Nothing left to delete,
                // and the id must leave the record or every start would try it again.
                self.shared.temporary.remove(id);
                true
            }
            Ok(Err(error)) => {
                warn!(session_id = id, %error, "temporary session: delete failed");
                false
            }
            Err(_) => {
                warn!(session_id = id, "temporary session: delete timed out");
                false
            }
        }
    }

    /// Deletes every temporary session (quit, or the start after a crash). Returns how many were deleted.
    pub(crate) async fn delete_temporary_all(&self) -> usize {
        let ids = self.shared.temporary.ids();
        let mut deleted = 0;
        for id in &ids {
            if self.delete_temporary(id).await {
                deleted += 1;
            }
        }
        if !ids.is_empty() {
            info!(deleted, of = ids.len(), "temporary sessions deleted");
        }
        deleted
    }
}
