//! The person is asked for the name and email Git writes into commits, once, when the agent's `git init` or
//! `git commit` would run without one: the Git guard waits until the person answers in the window (or the Debug API,
//! which acts as the person). Save writes them into Git's global settings (only fields that have no value there, never
//! overwriting one) and into the app's identity (`git_identity`, what the Commit dialog and Settings show); Cancel
//! refuses the call with [`NO_IDENTITY`].
//!
//! The question is asked only when the identity Git resolves for that folder is empty (`vw_git::identity::effective`:
//! the repository's own, else the global one), so a person with a global identity is never asked and nothing of
//! theirs is changed. A copy's shared repository config is never written (global only).
//!
//! One question at a time: calls that need it while it is open join it and all get its answer (as identical Git calls
//! share a guard sheet). The question closes when answered or when every waiting call is gone (Stop ends the hook's
//! call), and the window is told either way; a window that subscribes while it is open (a reload) gets it again.
//!
//! Depends on: vw-git (`identity`), `git_identity` (the app's identity file), core events. Called by: `gitcheck` (the
//! guard), `git_cmd` (`answerIdentity`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;
use vw_git::identity::{self, CommitIdentity, Global};

use crate::CoreEvent;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// The refusal of a `git init` or `git commit` when the person gave no name and email.
pub(crate) const NO_IDENTITY: &str =
    "Not run: No Git identity. The user did not give the name and email Git writes into commits.";

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The state of the question.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Open,
    Saved,
    Cancelled,
}

struct Pending {
    id: String,
    opened_at: u64,
    /// The chat whose call opened it.
    session: String,
    answer: watch::Sender<Answer>,
    waiters: usize,
}

/// Value-free metadata for /asks; the existing typed Git answer supplies the name and email.
pub(crate) fn question(shared: &Shared) -> Option<vw_ask::Ask> {
    shared
        .identity_asks
        .lock()
        .as_ref()
        .filter(|pending| *pending.answer.borrow() == Answer::Open)
        .map(|pending| vw_ask::Ask {
            id: pending.id.clone(),
            kind: "git_identity".into(),
            session: Some(pending.session.clone()),
            requester: vw_ask::Requester::Agent,
            surface: vw_ask::Surface::Card,
            title: "your Git name and email".into(),
            opened_at: pending.opened_at,
            offers: vec![],
            person_only: false,
            every_call: false,
            always_setting: None,
        })
}

/// The open question, and where the person's global Git settings are.
#[derive(Default)]
pub(crate) struct IdentityAsks {
    pending: Mutex<Option<Pending>>,
    /// An explicit global config file (tests); `None`: Git's own `--global`.
    global_file: Option<PathBuf>,
}

impl IdentityAsks {
    pub(crate) fn new(global_file: Option<PathBuf>) -> Self {
        Self {
            pending: Mutex::default(),
            global_file,
        }
    }

    fn global(&self) -> Global<'_> {
        self.global_file
            .as_deref()
            .map_or(Global::Git, Global::File)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Leaves the question when the waiting call ends; the last one closes an unanswered question.
struct Waiter<'a> {
    shared: &'a Arc<Shared>,
    id: String,
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        let mut pending = self.shared.identity_asks.lock();
        let closed = pending.as_mut().is_some_and(|p| {
            if p.id != self.id {
                return false;
            }
            p.waiters = p.waiters.saturating_sub(1);
            p.waiters == 0
        });
        if closed {
            *pending = None;
            drop(pending);
            self.shared.emit(CoreEvent::GitIdentityAskClosed {
                ask_id: self.id.clone(),
            });
        }
    }
}

/// The open question as the event a new subscriber (a reloaded window) gets first, as pending asks are replayed.
pub(crate) fn replay(shared: &Shared) -> Option<CoreEvent> {
    shared
        .identity_asks
        .lock()
        .as_ref()
        .filter(|p| *p.answer.borrow() == Answer::Open)
        .map(|p| CoreEvent::GitIdentityAsked {
            session_id: p.session.clone(),
            ask_id: p.id.clone(),
        })
}

/// `Ok(())` when a commit in `cwd` has a name and email (asking the person first when it has none), else the refusal.
pub(crate) async fn ensure(shared: &Arc<Shared>, session: &str, cwd: &Path) -> Result<(), String> {
    let worker = shared.clone();
    let folder = cwd.to_path_buf();
    let known = tokio::task::spawn_blocking(move || {
        identity::effective(&folder, worker.identity_asks.global())
    })
    .await
    .map_err(|e| format!("Not run: Vibe Work could not check the Git identity ({e})."))?;
    match known {
        Ok((Some(_), Some(_))) => return Ok(()),
        Ok(_) => {}
        // Git cannot say: the command's own Git reports what is missing.
        Err(error) => {
            tracing::warn!(%error, "git identity unreadable; not asking");
            return Ok(());
        }
    }
    let (id, mut answer, opened) = {
        let mut pending = shared.identity_asks.lock();
        if let Some(open) = pending.as_mut() {
            open.waiters += 1;
            (open.id.clone(), open.answer.subscribe(), false)
        } else {
            let id = format!(
                "git-identity-{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let (send, receive) = watch::channel(Answer::Open);
            *pending = Some(Pending {
                id: id.clone(),
                opened_at: shared.asks.now_ms(),
                session: session.to_owned(),
                answer: send,
                waiters: 1,
            });
            (id, receive, true)
        }
    };
    let _waiter = Waiter {
        shared,
        id: id.clone(),
    };
    if opened {
        shared.emit(CoreEvent::GitIdentityAsked {
            session_id: session.to_owned(),
            ask_id: id,
        });
    }
    let answered = *answer
        .wait_for(|state| *state != Answer::Open)
        .await
        .map_err(|_| NO_IDENTITY.to_owned())?;
    if answered == Answer::Saved {
        Ok(())
    } else {
        Err(NO_IDENTITY.to_owned())
    }
}

/// The person's answer to the open question `ask_id`: `Some` saves it (Git's global settings without overwriting a
/// value, and the app's identity) and lets the waiting calls run; `None` (Cancel) refuses them.
///
/// # Errors
/// `notFound` when `ask_id` is not the open question; `invalidParams` for an empty or malformed name or email;
/// `internal` when Git's settings cannot be written (the question stays open).
pub(crate) fn answer(
    shared: &Arc<Shared>,
    ask_id: &str,
    given: Option<CommitIdentity>,
) -> Result<(), CoreError> {
    let open = shared
        .identity_asks
        .lock()
        .as_ref()
        .is_some_and(|p| p.id == ask_id && *p.answer.borrow() == Answer::Open);
    if !open {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            "That question is closed.",
        ));
    }
    let given = given.map(|identity| CommitIdentity {
        name: identity.name.trim().to_owned(),
        email: identity.email.trim().to_owned(),
    });
    if let Some(identity) = &given {
        identity::validate(identity)
            .map_err(|e| CoreError::new(ErrorCode::InvalidParams, e.to_string()))?;
        let home = std::env::temp_dir();
        identity::save_global(&home, shared.identity_asks.global(), identity).map_err(|e| {
            CoreError::new(
                ErrorCode::Internal,
                format!("Git's settings could not be saved: {e}"),
            )
        })?;
        crate::git_identity::save(&shared.git, identity)?;
    }
    let mut pending = shared.identity_asks.lock();
    let Some(open) = pending.as_mut().filter(|p| p.id == ask_id) else {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            "That question is closed.",
        ));
    };
    open.answer.send_replace(if given.is_some() {
        Answer::Saved
    } else {
        Answer::Cancelled
    });
    *pending = None;
    drop(pending);
    shared.emit(CoreEvent::GitIdentityAskClosed {
        ask_id: ask_id.to_owned(),
    });
    Ok(())
}
