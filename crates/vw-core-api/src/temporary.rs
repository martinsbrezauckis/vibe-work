//! Temporary sessions are discarded after use.
//!
//! Vibe 3.0.2 has no per-session "don't save": `SessionKind.EPHEMERAL` is promoted to a stored session on its
//! first turn, and `session_logging.enabled = false` is process-wide. So a temporary session is a
//! normal Vibe session that Vibe Work deletes for the user: when Vibe Work quits ([`Core::delete_temporary_all`], from
//! `Core::shutdown`), or at the next start after a crash ([`Core::delete_temporary_all`]). The user may delete it
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
//!   found" succeeds only after checking for leftover files, or for a chat this core knows was never stored.
//!   Each sweep attempts an id once ([`Core::delete_temporary_all`]); unfinished deletion stays recorded.
//! - When a folder is configured, an id is on the list only if the record on disk names it too ([`TemporarySessions::add`]
//!   rolls the id back when the write fails).
//! - A temporary chat started without a folder works in a scratch folder of its own that Vibe Work makes for it
//!   ([`TemporarySessions::new_scratch`]: `temporary-folders/chat-<random>` beside `sessions_dir`, user-only). It
//!   lies outside the session data (`sessions_dir`), which the agent's folder guard keeps the agent out of. The folder
//!   and its automatic project go with the chat ([`chat_ended`]); a folder whose chat is gone without that (a crash)
//!   is removed by the sweep once no temporary chat is left ([`Core::delete_temporary_all`]). The sweep also removes
//!   folders of the same shape that earlier test builds made inside `sessions_dir` (`<sessions_dir>/temporary-folders`)
//!   under the same rules. Nothing outside those folders is ever removed here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tracing::{error, info, warn};

use crate::command::Origin;
use crate::core::{Core, Shared};
use crate::error::CoreError;
use crate::event::{AlertKind, CoreEvent};

/// Longest one deletion may take at quit or start.
const DELETE_WAIT: Duration = Duration::from_secs(10);
/// Short-lived readers can hold a scratch folder; give them at most this long before the next sweep takes over.
const SCRATCH_REMOVE_WAIT: Duration = Duration::from_secs(2);
/// The record's file name inside `sessions_dir`.
const FILE: &str = "temporary.json";
/// The folder beside `sessions_dir` that holds the scratch folders of temporary chats started without a folder.
const SCRATCH: &str = "temporary-folders";

/// How one delete of a temporary session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deleted {
    /// Vibe confirmed it, or answered "not found" (the session is gone either way).
    Yes,
    /// Vibe answered with another error.
    Refused,
    /// The gate refused to issue the token, or Vibe did not answer within [`DELETE_WAIT`].
    NoAnswer,
}

/// Why a temporary session could not be recorded; the text goes to the log and into the error the start answers.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct NotRecorded(String);

/// The temporary sessions of this app (in memory, mirrored to disk when a folder is configured).
#[derive(Default)]
pub(crate) struct TemporarySessions {
    ids: Mutex<BTreeSet<String>>,
    notices: Mutex<BTreeMap<String, CoreEvent>>,
    file: Option<PathBuf>,
    /// Where scratch folders are made (`temporary-folders` beside `sessions_dir`; memory-only runs: the system temp
    /// folder).
    scratch_root: PathBuf,
    /// Each memory-only unit-test core owns its root until the last fixture reference drops.
    #[cfg(test)]
    _scratch_fixture: Option<tempfile::TempDir>,
    /// Where earlier test builds made them (`<sessions_dir>/temporary-folders`): only swept, never used.
    old_scratch_root: Option<PathBuf>,
    /// The scratch folder of each temporary chat that has one (this run).
    scratch: Mutex<BTreeMap<String, PathBuf>>,
    /// Set at load when the record on disk is damaged and could not be moved aside: it has to stay as it is, so no
    /// write is attempted and every [`TemporarySessions::add`] fails with this text.
    frozen: Option<String>,
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
        let scratch_root = sessions_dir.as_ref().map_or_else(
            || std::env::temp_dir().join(format!("vibe-work-{SCRATCH}-{}", std::process::id())),
            |dir| dir.parent().unwrap_or(dir).join(SCRATCH),
        );
        // Unit-test cores share a process, but their startup/shutdown sweeps must not share folders.
        #[cfg(test)]
        let scratch_fixture = sessions_dir.is_none().then(tests::scratch_fixture);
        #[cfg(test)]
        let scratch_root = scratch_fixture
            .as_ref()
            .map_or(scratch_root, |dir| dir.path().to_path_buf());
        let old_scratch_root = sessions_dir
            .as_ref()
            .map(|dir| dir.join(SCRATCH))
            .filter(|old| *old != scratch_root);
        let file = sessions_dir.map(|dir| dir.join(FILE));
        let (ids, damage) = file
            .as_deref()
            .map_or_else(|| (BTreeSet::new(), None), read_record);
        let mut list = Self {
            ids: Mutex::new(ids),
            notices: Mutex::default(),
            file,
            scratch_root,
            #[cfg(test)]
            _scratch_fixture: scratch_fixture,
            old_scratch_root,
            scratch: Mutex::default(),
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

    /// Notices retained for a UI that subscribes after the startup sweep finished.
    pub(crate) fn notices(&self) -> Vec<CoreEvent> {
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Keeps the latest cleanup result for this session and publishes it to current subscribers.
    fn notice(&self, shared: &Shared, id: &str, removed: bool) {
        let (kind, message) = if removed {
            (
                AlertKind::TemporaryCleanupRecovered,
                "A temporary chat was not deleted before the previous run ended. It has now been deleted.".to_owned(),
            )
        } else {
            (AlertKind::TemporaryCleanupFailed,
                "A temporary chat could not be deleted. It remains stored by Vibe. Vibe Work will retry at the next start; you can also delete it from the sessions list.".to_owned())
        };
        let event = CoreEvent::Alert {
            session_id: Some(id.to_owned()),
            kind,
            message,
        };
        self.notices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), event.clone());
        shared.emit(event);
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

    /// [`Self::add`] on the blocking pool: the record's write waits for the disk (a synced replace, seen taking
    /// seconds on a busy disk) and must not hold an async worker. Returns once the record is on disk, as `add` does.
    pub(crate) async fn add_saved(self: &Arc<Self>, id: &str) -> Result<(), NotRecorded> {
        let (record, id) = (self.clone(), id.to_owned());
        tokio::task::spawn_blocking(move || record.add(&id))
            .await
            .unwrap_or_else(|join| Err(NotRecorded(format!("the record task stopped ({join})"))))
    }

    /// [`Self::remove`] on the blocking pool (see [`Self::add_saved`]); returns once the record was rewritten.
    pub(crate) async fn remove_saved(self: &Arc<Self>, id: &str) {
        let (record, id) = (self.clone(), id.to_owned());
        if let Err(join) = tokio::task::spawn_blocking(move || record.remove(&id)).await {
            warn!(%join, "temporary sessions: the record update stopped");
        }
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

    /// Makes a new, empty scratch folder for a temporary chat started without a folder (user-only, a random name no
    /// other chat has). Blocking (disk).
    ///
    /// # Errors
    /// [`NotRecorded`] naming the folder when it cannot be made.
    pub(crate) fn new_scratch(&self) -> Result<PathBuf, NotRecorded> {
        let cannot = |e: &dyn std::fmt::Display| {
            NotRecorded(format!(
                "the folder {} cannot be made ({e})",
                self.scratch_root.display()
            ))
        };
        vw_platform::ensure_private_dir(&self.scratch_root).map_err(|e| cannot(&e))?;
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).map_err(|e| cannot(&e))?;
        let name = random
            .iter()
            .fold(String::with_capacity(16), |mut name, b| {
                use std::fmt::Write as _;
                let _ = write!(name, "{b:02x}");
                name
            });
        let dir = self.scratch_root.join(format!("chat-{name}"));
        // `create_dir`, never `create_dir_all`: a name that exists already is not taken over.
        std::fs::create_dir(&dir).map_err(|e| cannot(&e))?;
        vw_platform::ensure_private_dir(&dir).map_err(|e| cannot(&e))?;
        Ok(dir)
    }

    /// Whether `folder` is a scratch folder made by [`Self::new_scratch`] (a direct `chat-` child of the scratch root).
    pub(crate) fn is_scratch(&self, folder: &Path) -> bool {
        is_scratch_in(&self.scratch_root, folder)
    }

    /// [`Self::new_scratch`] on the blocking pool.
    pub(crate) async fn new_scratch_saved(self: &Arc<Self>) -> Result<PathBuf, NotRecorded> {
        let record = self.clone();
        tokio::task::spawn_blocking(move || record.new_scratch())
            .await
            .unwrap_or_else(|join| Err(NotRecorded(format!("the folder task stopped ({join})"))))
    }

    /// Notes that chat `id` works in scratch folder `dir` (it goes with the chat).
    pub(crate) fn bind_scratch(&self, id: &str, dir: PathBuf) {
        self.scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), dir);
    }

    /// The scratch folder chat `id` works in, if it has one.
    fn scratch_of(&self, id: &str) -> Option<PathBuf> {
        self.scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Removes chat `id`'s scratch folder, if it has one; returns it. Blocking (disk).
    fn drop_scratch(&self, id: &str) -> Option<PathBuf> {
        let dir = self
            .scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)?;
        self.remove_scratch(&dir);
        Some(dir)
    }

    /// Removes one scratch folder: only a direct child of the scratch root, and never through a link.
    pub(crate) fn remove_scratch(&self, dir: &Path) {
        remove_scratch_in(&self.scratch_root, dir);
    }

    /// Removes every scratch folder whose chat is gone: all of them once no temporary chat is left (a chat the sweep
    /// could not delete keeps the folders until a later sweep). Returns the folders it removed. Blocking (disk).
    fn sweep_scratch(&self) -> Vec<PathBuf> {
        if !self
            .ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
        {
            return Vec::new();
        }
        let mut found = sweep_root(&self.scratch_root);
        if let Some(old) = &self.old_scratch_root {
            found.extend(sweep_root(old));
            // The old root goes too once it is empty (never recursively, never through a link).
            if is_real_folder(old) {
                let _ = std::fs::remove_dir(old);
            }
        }
        self.scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        found
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

/// Chat `id` ended (deleted): its scratch folder, if it has one, goes, and with it the automatic project the folder
/// got in the sidebar. Called last, when nothing of the chat holds the folder any more (its processes, terminals and
/// file views ended first).
pub(crate) async fn chat_ended(shared: &Shared, id: &str) {
    if shared.temporary.scratch_of(id).is_none() {
        return;
    }
    let (record, id) = (shared.temporary.clone(), id.to_owned());
    if let Ok(Some(dir)) = tokio::task::spawn_blocking(move || record.drop_scratch(&id)).await {
        crate::projects_filing::retire_folder_project(shared, &dir).await;
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
        let removed = self.delete_temporary(id).await == Deleted::Yes;
        if removed && self.shared.store.view(id).is_some() {
            crate::session_end::deleted(&self.shared, id).await;
        }
        removed
    }

    /// Deletes one temporary session without a dialog (see the module header).
    async fn delete_temporary(&self, id: &str) -> Deleted {
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
                return Deleted::NoAnswer;
            }
        };
        let call = self.forward(
            Origin::Ui,
            "session/delete",
            params,
            Some(token.as_str().to_owned()),
        );
        match tokio::time::timeout(DELETE_WAIT, call).await {
            Ok(Ok(_)) => Deleted::Yes,
            Ok(Err(error))
                if error.code == crate::ErrorCode::NotFound
                    || error.server_code.as_deref() == Some("not_found") =>
            {
                // Gone already: deleted in the CLI, or never stored (Vibe stores a session from its first turn, so
                // a temporary chat nobody wrote in answers "not found" to its own delete). Nothing left to delete,
                // and the id must leave the record or every start would try it again.
                self.shared.temporary.remove_saved(id).await;
                chat_ended(&self.shared, id).await;
                Deleted::Yes
            }
            Ok(Err(error)) => {
                warn!(session_id = id, %error, "temporary session: delete failed");
                Deleted::Refused
            }
            Err(_) => {
                warn!(session_id = id, "temporary session: delete timed out");
                Deleted::NoAnswer
            }
        }
    }

    /// Deletes every temporary session (quit, or the start after a crash). Returns how many were deleted.
    ///
    /// Each id is attempted once. Startup awaits this sweep before admitting any session readers.
    pub(crate) async fn delete_temporary_all(&self, at_start: bool) -> usize {
        let ids = self.shared.temporary.ids();
        let mut deleted = 0;
        for id in &ids {
            let outcome = self.delete_temporary(id).await;
            let removed = outcome == Deleted::Yes;
            if removed {
                deleted += 1;
            }
            if at_start || !removed {
                self.shared.temporary.notice(&self.shared, id, removed);
            }
        }
        if !ids.is_empty() {
            info!(deleted, of = ids.len(), "temporary sessions deleted");
        }
        let record = self.shared.temporary.clone();
        match tokio::task::spawn_blocking(move || record.sweep_scratch()).await {
            Ok(removed) => {
                for dir in removed {
                    crate::projects_filing::retire_folder_project(&self.shared, &dir).await;
                }
            }
            Err(join) => warn!(%join, "temporary sessions: the scratch folder sweep stopped"),
        }
        deleted
    }
}

/// Removes `dir` when it is a scratch folder directly in `root` and `root` is a real folder; a link in place of the
/// folder is removed itself, never followed. Busy readers get a short, bounded retry on the calling blocking task.
fn remove_scratch_in(root: &Path, dir: &Path) {
    remove_scratch_in_with(root, dir, |path| std::fs::remove_dir_all(path));
}

/// The directory-removal seam lets tests model EBUSY on hosts that can delete an open folder.
fn remove_scratch_in_with(
    root: &Path,
    dir: &Path,
    mut remove_dir: impl FnMut(&Path) -> std::io::Result<()>,
) {
    let deadline = Instant::now() + SCRATCH_REMOVE_WAIT;
    let mut backoff = Duration::from_millis(20);
    let removed = loop {
        // A reader can release the folder while another actor replaces its root or the folder itself.
        if !is_scratch_in(root, dir) || !is_real_folder(root) {
            warn!(folder = ?dir, "temporary sessions: not a scratch folder; left alone");
            return;
        }
        let attempt = match std::fs::symlink_metadata(dir) {
            // A link or junction in place of the folder: remove the link itself, never what it points to.
            Ok(meta) if is_link(&meta) => remove_link(dir, &meta),
            Ok(meta) if meta.is_dir() => remove_dir(dir),
            Ok(_) => std::fs::remove_file(dir),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        match attempt {
            Err(error) if folder_is_busy(&error) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break Err(error);
                }
                std::thread::sleep(backoff.min(remaining));
                backoff = (backoff * 2).min(Duration::from_millis(100));
            }
            other => break other,
        }
    };
    if let Err(error) = removed {
        // A process the chat left running can hold it (Windows); the next sweep tries again.
        warn!(folder = ?dir, %error, "temporary sessions: a scratch folder could not be removed");
    }
}

/// Retry only sharing violations on Windows and EBUSY elsewhere, never general permission or I/O failures.
fn folder_is_busy(error: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(32)
    }
    #[cfg(not(windows))]
    {
        error.kind() == std::io::ErrorKind::ResourceBusy
    }
}

/// Whether `folder` is a direct `chat-<16 hex>` child of `root`.
fn is_scratch_in(root: &Path, folder: &Path) -> bool {
    folder.parent() == Some(root)
        && folder.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name.strip_prefix("chat-")
                .is_some_and(|hex| hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        })
}

/// Whether `root` is a real folder, not a link or (Windows) junction to somewhere else: cleanup only ever deletes
/// inside a root it can trust.
fn is_real_folder(root: &Path) -> bool {
    std::fs::symlink_metadata(root).is_ok_and(|meta| meta.is_dir() && !is_link(&meta))
}

/// Removes every scratch folder in `root` (a real folder only); returns them. Only names this code makes
/// (`chat-` + 16 hex digits) count; anything else in the root is left alone.
fn sweep_root(root: &Path) -> Vec<PathBuf> {
    if !is_real_folder(root) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_scratch_in(root, path))
        .collect();
    for dir in &found {
        remove_scratch_in(root, dir);
    }
    found
}

/// Whether `meta` (from `symlink_metadata`) is a symbolic link, or on Windows any reparse point (junctions too).
fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// Removes the link at `path` itself (a directory link or junction on Windows is removed as a directory entry).
fn remove_link(path: &Path, meta: &std::fs::Metadata) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        // A directory link or junction carries the directory attribute and is removed as a directory entry.
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        if meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return std::fs::remove_dir(path);
        }
    }
    let _ = meta;
    std::fs::remove_file(path)
}

#[cfg(test)]
#[path = "temporary_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "temporary_removal_tests.rs"]
mod removal_tests;
