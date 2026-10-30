//! How sessions land in projects: the filing hint of a start, the hooks that file
//! every session the core sees (loaded, listed) or forget it (deleted), the reconcile sweep over Vibe's full session
//! list, and the folder whose handbook a session's Project tab shows.
//!
//! Depends on: vw-projects (store, rules), `core::Shared` (gate, store, names, worktree records, events).
//! Called by: `dispatch` (start hint, listings), `sessions::observe` (loaded, deleted), `stuck` (continue-in-new),
//! `projects` (list, refresh), `pages` / `pages_overview` (handbook folder, main folders).
//! Invariants:
//! - The hint lives in a task-local scope around exactly one start, so two starts in flight never swap hints.
//! - Store writes run on the blocking pool; a failed write is logged and changes nothing (the store's rule).
//! - The sweep prunes only after a COMPLETE listing (every page, archived included), and the store drops a member
//!   only when two sweeps at least `vw_projects::DROP_AFTER_MS` apart missed it; a listing that stops early only files.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::task::JoinHandle;
use vw_projects::{Hint, NewSession, ProjectStore, ProjectsError, Seen, State};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;

/// Sessions per `session/list` page of a sweep (Vibe allows 1..=500, `protocol.py:498`).
const SWEEP_PAGE: u32 = 500;
/// Most pages of one sweep (20,000 sessions); a longer list only files, never prunes.
const SWEEP_PAGES: usize = 40;
/// Longest wait for one sweep page.
const SWEEP_PAGE_TIMEOUT: Duration = Duration::from_secs(30);

tokio::task_local! {
    /// Where the session started inside this scope goes (a start in a project, continue-in-new).
    static FILING: Hint;
}

/// Runs `future` with `hint` as the filing of any session it creates.
pub(crate) async fn with_hint<F: std::future::Future>(hint: Option<Hint>, future: F) -> F::Output {
    match hint {
        Some(hint) => FILING.scope(hint, future).await,
        None => future.await,
    }
}

/// The projects part of the core.
pub(crate) struct ProjectsState {
    pub(crate) store: Arc<ProjectStore>,
    /// One sweep at a time.
    sweeping: tokio::sync::Mutex<()>,
    /// A complete sweep ran in this run.
    pub(crate) swept: AtomicBool,
    /// The background sweep of this run was started.
    started: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for ProjectsState {
    fn drop(&mut self) {
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

impl ProjectsState {
    /// The state over `<dir>/projects.json`, or in memory without a folder.
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            store: Arc::new(ProjectStore::open(dir)),
            sweeping: tokio::sync::Mutex::new(()),
            swept: AtomicBool::new(false),
            started: AtomicBool::new(false),
            task: Mutex::new(None),
        }
    }
}

fn io_error(error: &ProjectsError) -> CoreError {
    let code = match error {
        ProjectsError::Invalid(_) => ErrorCode::InvalidParams,
        ProjectsError::NotFound(_) => ErrorCode::NotFound,
        ProjectsError::Io(_) => ErrorCode::Internal,
    };
    CoreError::new(code, error.to_string())
}

/// Applies `f` to the store on the blocking pool; emits `projectsChanged` when something changed.
pub(crate) async fn update<T: Send + 'static>(
    shared: &Shared,
    f: impl FnOnce(&mut State) -> Result<T, ProjectsError> + Send + 'static,
) -> Result<T, CoreError> {
    let store = shared.projects.store.clone();
    let (value, changed) = tokio::task::spawn_blocking(move || store.update(f))
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("task failed: {e}")))?
        .map_err(|e| io_error(&e))?;
    if changed {
        shared.emit(CoreEvent::ProjectsChanged {});
    }
    Ok(value)
}

/// [`update`] for the filing hooks: a failure is logged, never raised (the session call itself worked).
async fn file(shared: &Shared, f: impl FnOnce(&mut State) -> usize + Send + 'static) {
    if let Err(error) = update(shared, move |s| Ok(f(s))).await {
        tracing::warn!(%error, "projects: a session could not be filed");
    }
}

/// The folder the folder rule uses for a listed or resumed session: the repository its copy was made from
/// (the record's `repoRoot`), else its `cwd`.
fn rule_folder(shared: &Shared, id: &str, session: &Value) -> Option<String> {
    let copy = session.get("worktree").is_some_and(|w| !w.is_null());
    if copy
        && let Some(record) = shared
            .worktrees
            .records()
            .ok()
            .and_then(|records| records.get(id))
    {
        return Some(record.repo_root.to_string_lossy().into_owned());
    }
    session
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
}

fn seen_of(shared: &Shared, item: &Value) -> Option<Seen> {
    let id = item.get("id").and_then(Value::as_str)?;
    Some(Seen {
        id: id.to_owned(),
        folder: rule_folder(shared, id, item),
    })
}

/// A listing page: every session seen for the first time is filed by its folder (auto projects at the end).
pub(crate) async fn listed(shared: &Shared, items: &[Value]) {
    let seen: Vec<Seen> = items.iter().filter_map(|i| seen_of(shared, i)).collect();
    if seen.is_empty() {
        return;
    }
    let now = vw_projects::now_ms();
    file(shared, move |s| s.file_seen(&seen, now)).await;
}

/// A session state was loaded (`sessions::observe`). `new`: Vibe created the session in this call (start, fork, a
/// rewind or compaction that made a new id): filed by the hint, the source, then the folder the start asked for.
/// Otherwise (resume, read) only a never-seen session is filed, by its folder.
pub(crate) async fn loaded(shared: &Shared, params: &Value, state: &Value, new: bool) {
    let Some(session) = state.get("session") else {
        return;
    };
    let Some(id) = session.get("id").and_then(Value::as_str).map(str::to_owned) else {
        return;
    };
    let rule = rule_folder(shared, &id, session);
    let now = vw_projects::now_ms();
    if !new {
        let seen = vec![Seen { id, folder: rule }];
        file(shared, move |s| s.file_seen(&seen, now)).await;
        return;
    }
    let hint = FILING.try_with(Clone::clone).ok();
    let source = params
        .get("sourceSessionId")
        .or_else(|| params.get("sessionId"))
        .and_then(Value::as_str)
        .filter(|s| *s != id)
        .map(str::to_owned);
    // The start's own folder: for a copy this is the user's folder, not the copy Vibe moved into.
    let folder = params
        .pointer("/agentConfig/cwd")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(rule);
    file(shared, move |s| {
        usize::from(s.file_new(
            &NewSession {
                id: &id,
                hint,
                source: source.as_deref(),
                folder: folder.as_deref(),
            },
            now,
        ))
    })
    .await;
}

/// Session `old` was handed off to `new`: `new` is filed where `old` is, unless it is filed already (a
/// rewind or `/clear` answered through `loaded` was filed by its source there; a compaction or context clear that
/// Vibe announced only by notification was not).
pub(crate) async fn handed_off(shared: &Shared, old: &str, new: &str) {
    let (old, new) = (old.to_owned(), new.to_owned());
    let now = vw_projects::now_ms();
    file(shared, move |s| {
        usize::from(s.file_new(
            &NewSession {
                id: &new,
                hint: None,
                source: Some(&old),
                folder: None,
            },
            now,
        ))
    })
    .await;
}

/// Vibe deleted the session (through the core): it leaves the store at once.
pub(crate) async fn deleted(shared: &Shared, id: &str) {
    let id = id.to_owned();
    file(shared, move |s| usize::from(s.forget(&id))).await;
}

/// The hint that keeps a continued session in its source's project ("Continue in a new session").
pub(crate) fn inherit_hint(shared: &Shared, source: &str) -> Option<Hint> {
    shared.projects.store.read(|s| {
        s.filing_of(source).map(|p| match p {
            Some(id) => Hint::Project(id.to_owned()),
            None => Hint::Unfiled,
        })
    })
}

/// Reads every page of Vibe's session list (archived included) and reconciles; see the module invariants.
///
/// # Errors
/// `timeout` / gate errors of a page (nothing was changed by the failed page; earlier pages were only filed).
pub(crate) async fn sweep(shared: &Shared) -> Result<(), CoreError> {
    let _one = shared.projects.sweeping.lock().await;
    let mut listed: Vec<Seen> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut complete = false;
    for _ in 0..SWEEP_PAGES {
        let mut params = json!({"includeArchived": true, "limit": SWEEP_PAGE});
        if let Some(cursor) = &cursor {
            params["cursor"] = json!(cursor);
        }
        let reply = tokio::time::timeout(
            SWEEP_PAGE_TIMEOUT,
            shared.gate.call("session/list", params, None),
        )
        .await
        .map_err(|_| CoreError::new(ErrorCode::Timeout, "the session list did not answer"))?
        .map_err(|e| CoreError::from_gate("session/list", &e))?;
        let mut items = reply
            .result
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // The name cache keeps titles with vault-value parts withheld (`title_guard`).
        for item in &mut items {
            crate::title_guard::withhold_in(shared, item);
        }
        shared.names.note_listing(&items);
        listed.extend(items.iter().filter_map(|i| seen_of(shared, i)));
        cursor = reply
            .result
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if cursor.is_none() {
            complete = true;
            break;
        }
    }
    let loaded: BTreeSet<String> = shared
        .store
        .views()
        .into_iter()
        .map(|v| v.session_id)
        .collect();
    let now = vw_projects::now_ms();
    let outcome = update(shared, move |s| {
        Ok(if complete {
            s.sweep(&listed, &loaded, now)
        } else {
            vw_projects::SweepOutcome {
                filed: s.file_seen(&listed, now),
                ..Default::default()
            }
        })
    })
    .await?;
    if complete {
        shared.projects.swept.store(true, Ordering::Relaxed);
    } else {
        tracing::warn!(
            pages = SWEEP_PAGES,
            "projects: the session list is longer than a sweep reads; nothing pruned"
        );
    }
    tracing::info!(
        filed = outcome.filed,
        missing = outcome.missing,
        dropped = outcome.dropped,
        complete,
        "projects: reconciled with Vibe's session list"
    );
    Ok(())
}

/// Starts this run's background sweep once (an owned task, aborted with the core).
pub(crate) fn sweep_once(shared: &Arc<Shared>) {
    if shared.projects.started.swap(true, Ordering::Relaxed) {
        return;
    }
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let task = tokio::spawn(async move {
        let Some(shared) = weak.upgrade() else {
            return;
        };
        if let Err(error) = sweep(&shared).await {
            tracing::warn!(%error, "projects: the first sweep failed; the next refresh tries again");
        }
    });
    *shared
        .projects
        .task
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(task);
}

/// The folder whose handbook session `id` shows in its Project tab: its project's main folder,
/// unless the session works in that folder or in a copy of it (then its own folder, `cwd`).
pub(crate) fn handbook_folder(shared: &Shared, id: &str, cwd: &str) -> PathBuf {
    let main = shared.projects.store.read(|s| {
        s.filing_of(id)
            .flatten()
            .and_then(|p| s.project(p))
            .and_then(|p| p.main_folder.clone())
    });
    let Some(main) = main else {
        return PathBuf::from(cwd);
    };
    // One folder in any spelling (case and separators on Windows, case on macOS).
    if vw_projects::same_folder(cwd, &main) {
        return PathBuf::from(cwd);
    }
    let copy_of_main = shared
        .worktrees
        .records()
        .ok()
        .and_then(|records| records.get(id))
        .is_some_and(|r| vw_projects::same_folder(&r.repo_root.to_string_lossy(), &main));
    if copy_of_main {
        PathBuf::from(cwd)
    } else {
        PathBuf::from(main)
    }
}

/// The main folders of every project, in order, without repeats (the Projects overview's cards).
pub(crate) fn main_folders(shared: &Shared) -> Vec<String> {
    shared.projects.store.read(|s| {
        let mut seen = BTreeSet::new();
        s.projects()
            .iter()
            .filter_map(|p| p.main_folder.clone())
            .filter(|f| seen.insert(vw_platform::folder_key(f)))
            .collect()
    })
}
