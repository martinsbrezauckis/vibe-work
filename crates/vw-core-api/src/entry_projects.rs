//! Session-open snapshots of vault project identity, independent of later Vibe state or Git-file edits.
//!
//! Depends on: `entry_permissions`, `core::Shared`, `serde_json`. Called by: `dispatch`, `recovery`,
//! `handoff`, and vault consumers. A failed capture stays failed; reopening never broadens authority.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

use crate::core::Shared;
use crate::entry_permissions::{Identity, capture, registered};

type Captured = Result<Identity, String>;

/// Core-owned identities; only session opening and a core handoff write them.
#[derive(Default)]
pub(crate) struct EntryProjects {
    identities: Mutex<HashMap<String, Captured>>,
    copies: Mutex<HashMap<String, Opening>>,
}

impl EntryProjects {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Captured>> {
        self.identities
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn get(&self, id: &str) -> Captured {
        self.lock()
            .get(id)
            .cloned()
            .unwrap_or_else(|| Err("this session has no captured project identity".to_owned()))
    }

    pub(crate) fn known(&self, id: &str) -> Option<Captured> {
        self.lock().get(id).cloned()
    }

    pub(crate) fn forget(&self, id: &str) {
        self.lock().remove(id);
        self.copies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }

    pub(crate) fn folders(&self) -> Vec<PathBuf> {
        self.lock()
            .values()
            .filter_map(|v| v.as_ref().ok().map(|i| PathBuf::from(&i.plain)))
            .collect()
    }

    pub(crate) fn hand_off(&self, old: &str, new: &str) {
        let mut identities = self.lock();
        if let Some(identity) = identities.get(old).cloned() {
            identities.insert(new.to_owned(), identity);
        }
    }
}

/// Prepared before the start/resume call. New Vibe copies are checked against this registration snapshot.
pub(crate) struct Opening {
    identity: Captured,
    registered: BTreeSet<PathBuf>,
    copy_name: Option<String>,
}

/// Captures the user-selected folder before Vibe starts; existing sessions always keep their first snapshot.
pub(crate) async fn before_open(shared: &Shared, method: &str, params: &Value) -> Option<Opening> {
    if !matches!(
        method,
        "session/start" | "session/resume" | "session/continue" | "session/fork"
    ) {
        return None;
    }
    let source = params
        .get("sourceSessionId")
        .or_else(|| params.get("sessionId"))
        .and_then(Value::as_str);
    let existing = source.and_then(|id| shared.entry_projects.lock().get(id).cloned());
    if let Some(identity) = existing {
        return Some(Opening {
            identity,
            registered: BTreeSet::new(),
            copy_name: None,
        });
    }
    let registered = registered(shared);
    let folder = params
        .pointer("/agentConfig/cwd")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| source.and_then(|id| shared.names.cwd_of(&shared.store, id)));
    let identity = if let Some(folder) = folder {
        let known = registered.clone();
        tokio::task::spawn_blocking(move || capture(&folder, &known))
            .await
            .unwrap_or_else(|_| Err("the project identity could not be captured".to_owned()))
    } else {
        Err("this session has no user-selected project folder".to_owned())
    };
    Some(Opening {
        identity,
        registered,
        copy_name: (method == "session/start"
            && params
                .pointer("/agentConfig/worktree/kind")
                .and_then(Value::as_str)
                == Some("create"))
        .then(|| {
            params
                .pointer("/agentConfig/worktree/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        }),
    })
}

impl Opening {
    /// Binds the first identity before the result is loaded into the session store and agent tools can use it.
    pub(crate) fn finish(self, shared: &Shared, result: &Value) {
        let Some(id) = result.pointer("/state/session/id").and_then(Value::as_str) else {
            return;
        };
        // Vibe creates its worktree after answering start. No secret authority exists during that transition.
        if self.copy_name.is_some() {
            shared
                .entry_projects
                .lock()
                .entry(id.to_owned())
                .or_insert_with(|| Err("the copy's project identity is not ready".to_owned()));
            shared
                .entry_projects
                .copies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.to_owned(), self);
            return;
        }
        shared
            .entry_projects
            .lock()
            .entry(id.to_owned())
            .or_insert(self.identity);
    }
}

/// Finalizes a Vibe-created copy once, while the authorized start is still waiting for its folder.
pub(crate) async fn finish_copy(shared: &Shared, id: &str, path: Option<&Path>) {
    let opening = shared
        .entry_projects
        .copies
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(id);
    let Some(opening) = opening else { return };
    let path = path.map(Path::to_path_buf);
    let cwd = shared.names.cwd_of(&shared.store, id);
    let vibe_root = shared.worktrees.vibe_root().to_path_buf();
    let identity = tokio::task::spawn_blocking(move || {
        opening.identity.and_then(|chosen| {
            let path = path.ok_or_else(|| "the copy has no project folder".to_owned())?;
            let path = vw_platform::canonical_plain(&path)
                .map_err(|_| "the copy folder could not be resolved".to_owned())?;
            let root = vw_platform::canonical_plain(&vibe_root)
                .map_err(|_| "the copy root could not be resolved".to_owned())?;
            if !path.starts_with(root)
                || path.file_name().and_then(|n| n.to_str()) != opening.copy_name.as_deref()
            {
                return Err("the copy does not match the selected start target".to_owned());
            }
            // Vibe preserves a selected repository subfolder inside the new copy. Bind the
            // actual tool cwd, while refusing a symlink or context that escapes the checked root.
            let cwd = cwd
                .and_then(|cwd| vw_platform::canonical_plain(Path::new(&cwd)).ok())
                .filter(|cwd| cwd.starts_with(&path))
                .ok_or_else(|| "the session folder is outside its copy".to_owned())?;
            let mut registered = opening.registered;
            registered.retain(|p| p == Path::new(chosen.allowed_project()));
            // An admitted start makes its exact chosen folder known, including a native copy-dialog choice.
            registered.insert(PathBuf::from(&chosen.plain));
            capture(&cwd.to_string_lossy(), &registered)
        })
    })
    .await
    .unwrap_or_else(|_| Err("the copy identity could not be captured".to_owned()));
    let mut identities = shared.entry_projects.lock();
    // A concurrent delete must not resurrect the session's authority.
    if let Some(current) = identities.get_mut(id) {
        *current = identity;
    }
}
