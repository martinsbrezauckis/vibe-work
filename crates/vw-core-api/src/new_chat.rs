//! The New chat screen's owner: terminals, the browser (App preview and web tabs) and a port lease open there
//! before any session exists, under an owner id the core makes. The first message hands them to the session it
//! started; "Close all" stops them.
//!
//! Depends on: `handoff` (the move), `terminals`, `processes`, `previews`, `projects` folder rule. Called by:
//! `dispatch` (`Command::NewChat`), `terminals` (folder and limit text), `previews`/`browser_tabs` (owner check),
//! `session_readonly` (no session claim for the owner).
//! Invariants:
//! - The owner is never a Vibe session: nothing about it is sent to Vibe, it holds no agent credential, and the
//!   agent's tools refuse it because it is not a loaded session.
//! - A terminal's folder is the folder the core was told through `setFolder` and checked, else the home folder;
//!   never a path in a terminal request.
//! - Each draft has its own id, so the next draft never shares the adopted one's site data folder.
//! - `adopt` gives the session the draft's App preview site data (re-keyed to the session, vw-preview
//!   `adopt_storage`), so sign-ins made before the first message stay; `closeAll` wipes the draft's and renews the
//!   id, so a wiped store is never used again.
//! - `adopt` runs only for a loaded session and stops nothing: the same shells, processes, pages and ports.
//! - After a restart the window shows the New chat folder it had; when that folder was picked for the screen, it works
//!   again without a new pick (Files tab, terminals): a saved link spelling first needs native Keep naming its target.
//!   The core keeps its own record of the picked New chat folder in
//!   `<sessions_dir>/new-chat-folder.json` (the window's saved state never counts). The record names exactly one
//!   folder, in the spelling the window uses and as resolved at the pick; it applies only while that folder is the
//!   New chat folder, is replaced when the screen moves to another picked folder and dropped when it moves to any
//!   other folder or none. It never makes the folder trusted or picked for anything else.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use vw_gate::{Confirmation, ConfirmationRequest, Requester};

use crate::Origin;
use crate::command::CommandResult;
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::new_chat_view::{NewChatRequest, NewChatView};

/// What the core knows about the New chat screen.
pub(crate) struct Draft {
    state: Mutex<DraftState>,
    /// Folder changes wait for the current native decision, so repeated supplies share one answer.
    change: tokio::sync::Mutex<()>,
    /// `<sessions_dir>/new-chat-folder.json`; `None`: the record lives in memory for this run only.
    file: Option<PathBuf>,
}

struct DraftState {
    id: String,
    folder: Option<PathBuf>,
    /// The window named a folder the core may not open a terminal in.
    refused: bool,
    /// The picked New chat folder, from this run or the run before (see the module invariants).
    picked: Option<PickedFolder>,
    /// `folder` is allowed only because it is `picked` from the run before.
    restored: bool,
    /// The person kept the recorded link target in this run.
    confirmed: bool,
}

/// Native confirmation before a saved linked New chat folder can restore authority.
pub const NEW_CHAT_RESTORE_DIALOG_METHOD: &str = "newChat/restoreFolder";

/// The record's file name inside `sessions_dir`.
const PICKED_FILE: &str = "new-chat-folder.json";

/// A New chat folder the user picked: the spelling the window names it in and the folder as resolved at the pick.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PickedFolder {
    folder: PathBuf,
    canonical: PathBuf,
}

fn fresh_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut bytes = [0u8; 8];
    let value = if getrandom::fill(&mut bytes).is_ok() {
        u64::from_le_bytes(bytes)
    } else {
        (u64::from(std::process::id()) << 32)
            ^ NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    };
    format!("new-chat-{value:016x}")
}

impl Draft {
    /// The screen's state, with the picked New chat folder an earlier run recorded in `sessions_dir` (an unreadable or
    /// malformed record counts as none).
    pub(crate) fn new(sessions_dir: Option<&Path>) -> Self {
        let file = sessions_dir.map(|dir| dir.join(PICKED_FILE));
        let picked = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice::<PickedFolder>(&bytes).ok());
        Self {
            state: Mutex::new(DraftState {
                id: fresh_id(),
                folder: None,
                refused: false,
                picked,
                restored: false,
                confirmed: false,
            }),
            change: tokio::sync::Mutex::new(()),
            file,
        }
    }

    /// The New chat folder when it is allowed only as the picked folder of the run before (the Files tab and the
    /// file search accept exactly that folder then, as a pick of this run).
    pub(crate) fn restored_folder(&self) -> Option<PathBuf> {
        let state = self.lock();
        state.folder.clone().filter(|_| state.restored)
    }

    /// True when `folder` is the recorded picked New chat folder: the same spelling, and it still leads to the folder
    /// resolved at the pick (a folder replaced by a link since then does not count). Compared as written first, so
    /// the disk is read only for that one folder.
    fn is_recorded(&self, folder: &Path) -> bool {
        let Some(picked) = self.lock().picked.clone() else {
            return false;
        };
        picked.folder == folder
            && folder
                .canonicalize()
                .is_ok_and(|real| real == picked.canonical && real.is_dir())
    }

    /// A saved link is display state until the person keeps its captured target. The caller orders folder changes.
    async fn confirm_restore(&self, shared: &Shared) -> bool {
        let picked = {
            let mut state = self.lock();
            let Some(picked) = state.picked.clone() else {
                return false;
            };
            // Windows' verbatim prefix is a path representation, not a link in the person's spelling.
            if state.confirmed
                || vw_platform::plain_path(&picked.folder)
                    == vw_platform::plain_path(&picked.canonical)
            {
                return true;
            }
            state.folder = None;
            state.restored = false;
            state.refused = true;
            picked
        };
        let answer = shared
            .confirmer
            .begin(ConfirmationRequest {
                method: NEW_CHAT_RESTORE_DIALOG_METHOD.to_owned(),
                summary: format!(
                    "Keep using {}? It opens {}.",
                    picked.folder.display(),
                    picked.canonical.display()
                ),
                params: serde_json::json!(picked),
                // A saved link never authorizes its own restoration. The person or owner Debug API answers.
                requester: Requester::Window,
                always: None,
            })
            .await;
        // Keep covers only the target the person saw, even if the spelling changed while the dialog waited.
        let kept = answer == Confirmation::Approved && self.is_recorded(&picked.folder);
        self.lock().confirmed = kept;
        kept
    }

    /// Writes the record (`None`: removes it) when it changed. A failed write is logged; this run keeps working.
    fn record(&self, picked: Option<PickedFolder>) {
        let bytes = picked.as_ref().map(serde_json::to_vec);
        {
            let mut state = self.lock();
            if state.picked == picked {
                return;
            }
            state.picked = picked;
            state.confirmed = false;
        }
        let Some(file) = &self.file else { return };
        let written = match bytes {
            None => match std::fs::remove_file(file) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(error.to_string())
                }
                _ => Ok(()),
            },
            Some(Err(error)) => Err(error.to_string()),
            Some(Ok(bytes)) => file
                .parent()
                .map_or(Ok(()), vw_platform::ensure_private_dir)
                .and_then(|()| vw_platform::replace_private(file, &bytes))
                .map_err(|error| error.to_string()),
        };
        if let Err(error) = written {
            tracing::warn!(%error, "new chat: the picked folder record was not written");
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DraftState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `id` is the current owner id.
    pub(crate) fn is(&self, id: &str) -> bool {
        !id.is_empty() && self.lock().id == id
    }

    /// Gives the screen a fresh id (after a hand-off or Close all); returns the old one.
    fn renew(&self) -> String {
        std::mem::replace(&mut self.lock().id, fresh_id())
    }
}

/// Whether `id` is the New chat screen's owner.
pub(crate) fn is_draft(shared: &Shared, id: &str) -> bool {
    shared.new_chat.is(id)
}

/// Whether the user's browser and preview requests may act for `owner`: a loaded session or the New chat screen.
pub(crate) fn known_owner(shared: &Shared, owner: &str) -> bool {
    shared.store.view(owner).is_some() || is_draft(shared, owner)
}

/// The home folder, as the terminals' own environment names it.
fn home(shared: &Shared) -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    shared
        .terminals
        .config()
        .parent_env
        .vars()
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| PathBuf::from(value))
        .filter(|path| path.is_absolute())
}

/// The folder a new terminal of the New chat screen starts in.
pub(crate) fn terminal_folder(shared: &Shared) -> Result<PathBuf, CoreError> {
    let (folder, refused) = {
        let state = shared.new_chat.lock();
        (state.folder.clone(), state.refused)
    };
    if refused {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            "Choose the folder again with Change folder… to open a terminal in it.",
        ));
    }
    folder.or_else(|| home(shared)).ok_or_else(|| {
        CoreError::new(
            ErrorCode::NotFound,
            "Choose a folder first: there is no home folder to open a terminal in.",
        )
    })
}

/// The refusal text when the New chat screen has `max` terminals open.
pub(crate) fn too_many(max: usize) -> CoreError {
    CoreError::new(
        ErrorCode::Denied,
        format!("This new chat already has {max} terminals open. Close one first."),
    )
}

/// The same rule as a project's main folder: chosen with Choose folder, a session's folder or a project's.
fn may_use(shared: &Shared, origin: Origin, folder: &Path) -> bool {
    origin == Origin::DebugApi
        || shared.gate.is_picked_folder(folder)
        || folder
            .to_str()
            .is_some_and(|text| shared.names.is_session_folder(&shared.store, text))
        || folder.to_str().is_some_and(|text| {
            shared.projects.store.read(|s| {
                s.projects()
                    .iter()
                    .any(|p| p.main_folder.as_deref() == Some(text))
            })
        })
}

fn view(shared: &Shared) -> NewChatView {
    let (owner_id, folder) = {
        let state = shared.new_chat.lock();
        (
            state.id.clone(),
            state.folder.as_ref().map(|f| f.display().to_string()),
        )
    };
    let home = home(shared).map(|h| h.display().to_string());
    NewChatView {
        owner_id,
        folder,
        home,
    }
}

/// Moves the New chat screen's terminals, browser, port lease and (in the same folder) open files' handles to
/// `session_id` and gives the screen a fresh id.
pub(crate) async fn adopt(
    shared: &std::sync::Arc<Shared>,
    session_id: &str,
) -> Result<(), CoreError> {
    if shared.store.view(session_id).is_none() {
        return Err(CoreError::not_loaded(session_id));
    }
    let old = shared.new_chat.renew();
    // One lease per session: a fresh session that already leased ports gives back the ones nothing uses.
    if !shared.procmon.leases().of(&old).is_empty()
        && !shared.procmon.leases().of(session_id).is_empty()
    {
        let released = shared.procmon.leases().release(session_id);
        tracing::debug!(
            ?released,
            "new chat: the started session's own lease was released"
        );
    }
    tracing::info!("new chat: handing what is open to the started session");
    // The screen's App preview website data (sign-ins made there) becomes the session's, before any page moves so
    // none of the session's mounts its own empty store first. A session that already has website data of its own
    // keeps it (not expected for a session the first message just started); the screen's data is wiped below then.
    let adopted = match &shared.preview {
        Some(hub) => hub.service.adopt_storage(&old, session_id).await,
        None => false,
    };
    crate::handoff::move_running(shared, &old, session_id).await;
    // The old owner is gone: its end wipes its App preview data unless the session adopted it (the preview service
    // never wipes an adopted store for the old owner; the session's delete wipes it).
    tracing::debug!(adopted, "new chat: App preview site data");
    crate::previews::session_ended(shared, &old, true).await;
    // Files open on the screen follow when the session works in the screen's folder (not in a copy), as the window
    // moves its editors: their handles become the session's, so the session can read and save them.
    let folder = shared.new_chat.lock().folder.clone();
    if let Some(folder) = folder
        && shared
            .names
            .cwd_of(&shared.store, session_id)
            .is_some_and(|cwd| Path::new(&cwd) == folder)
    {
        let key = format!("{}{}", crate::explorer::WORKSPACE, folder.to_string_lossy());
        shared.files.hand_over(&key, session_id);
    }
    Ok(())
}

/// Stops the New chat screen's terminals, releases its lease and closes its pages (local site data cleared), and
/// gives the screen a fresh id: a wiped store is never used again (on Linux a closed preview's web context keeps
/// what it holds until quit), so the next first message can only adopt data made after Close all.
pub(crate) async fn close_all(shared: &std::sync::Arc<Shared>) {
    let id = shared.new_chat.renew();
    crate::terminals::close_session(shared, &id).await;
    crate::processes::end_session(shared, &id, false).await;
    crate::previews::session_ended(shared, &id, true).await;
}

impl Core {
    /// Executes one New chat request.
    pub(crate) async fn new_chat(
        &self,
        origin: Origin,
        request: NewChatRequest,
    ) -> Result<CommandResult, CoreError> {
        let shared = &self.shared;
        match request {
            NewChatRequest::State {} => {}
            NewChatRequest::SetFolder { folder } => {
                let _change = shared.new_chat.change.lock().await;
                let folder = folder.filter(|f| !f.is_empty()).map(PathBuf::from);
                let usable = |f: &Path| f.is_absolute() && f.is_dir();
                // Picked in this run (natively or by the Debug API): recorded for the next start.
                let picked = folder
                    .as_deref()
                    .filter(|f| usable(f) && shared.gate.is_picked_folder(f))
                    .and_then(|f| {
                        Some(PickedFolder {
                            folder: f.to_path_buf(),
                            canonical: f.canonicalize().ok()?,
                        })
                    });
                let restored = picked.is_none()
                    && folder
                        .as_deref()
                        .is_some_and(|f| usable(f) && shared.new_chat.is_recorded(f));
                if restored && !shared.new_chat.confirm_restore(shared).await {
                    shared.new_chat.record(None);
                    shared.new_chat.lock().refused = false;
                    return Ok(CommandResult::NewChat { view: view(shared) });
                }
                let allowed = folder
                    .as_deref()
                    .is_none_or(|f| usable(f) && (restored || may_use(shared, origin, f)));
                {
                    let mut state = shared.new_chat.lock();
                    state.refused = !allowed;
                    state.folder = if allowed { folder } else { None };
                    state.restored = allowed && restored;
                }
                // The record follows the screen's folder; a refused folder leaves it as it was.
                if allowed && !restored {
                    shared.new_chat.record(picked);
                }
                if !allowed {
                    return Err(CoreError::new(
                        ErrorCode::Denied,
                        "This folder was not chosen in this run. Choose it again with Change folder….",
                    ));
                }
            }
            NewChatRequest::Adopt { session_id } => adopt(shared, &session_id).await?,
            NewChatRequest::CloseAll {} => close_all(shared).await,
        }
        Ok(CommandResult::NewChat { view: view(shared) })
    }
}
