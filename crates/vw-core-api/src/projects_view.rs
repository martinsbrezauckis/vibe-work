//! The `projects` command's requests and results: the user's project groups. Logic lives in
//! `projects.rs`; the store and the rules in the vw-projects crate.
//!
//! Depends on: serde. Called by: `command` (the `projects` variant), `projects`, the UI through the generated
//! TypeScript (`projects.ts`).
//! Invariants: wire form like every command (`{"op": "<camelCase>", ...}`, unknown fields refused); both clients get
//! the same answers (no redaction for the Debug API).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One project request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProjectsRequest {
    /// The projects in the user's order and the session -> project map. The first one of a run also starts the
    /// reconcile with Vibe's full session list in the background.
    List {},
    /// Reconciles with Vibe's full session list now (files unseen sessions, drops sessions Vibe no longer has after
    /// two sweeps at least ten minutes apart), then answers like `list`.
    Refresh {},
    /// A new project at the top of the order.
    Create {
        /// Its name (1-120 characters after cleaning).
        name: String,
        /// Its main folder: a folder a known session works in, or one picked in the native dialog.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        main_folder: Option<String>,
    },
    /// Renames a project.
    Rename {
        /// Project id.
        project_id: String,
        /// The new name.
        name: String,
    },
    /// Deletes a project. Its sessions move to "not in a project"; no session is deleted.
    Delete {
        /// Project id.
        project_id: String,
    },
    /// Files sessions into a project, or out of every project (`projectId` null or absent).
    Move {
        /// The sessions (1-500, each one Vibe Work has seen).
        session_ids: Vec<String>,
        /// The target project; `null`: not in a project.
        #[serde(default)]
        project_id: Option<String>,
    },
    /// Moves a project before another one (`before` null or absent: to the end).
    Place {
        /// Project id.
        project_id: String,
        /// The project it goes before.
        #[serde(default)]
        before: Option<String>,
    },
    /// Starts a session (`session/start` through the gate, like `startSession`) filed into the project: no auto
    /// project is made for its folder. Answers like `startSession` (a `session` result).
    Start {
        /// Project id.
        project_id: String,
        /// `AgentConfig` object (filtered by the gate; `worktree` is refused: copies start through `worktrees`).
        agent_config: Value,
    },
    /// Sets (a folder a known session works in, or one picked in the native dialog) or clears (`null`) a project's
    /// main folder.
    SetFolder {
        /// Project id.
        project_id: String,
        /// The folder, or `null` to clear.
        #[serde(default)]
        folder: Option<String>,
    },
}

/// One project as the clients see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    /// Project id.
    pub id: String,
    /// The user's name for it.
    pub name: String,
    /// The main folder; `null` for a name-only project.
    pub main_folder: Option<String>,
    /// The main folder's last name (`null` without a main folder).
    pub folder_name: Option<String>,
    /// Created by the filing rule for a folder.
    pub auto: bool,
    /// Sessions filed in it (Vibe Work's count; archived ones included).
    pub sessions: u32,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ProjectsResult {
    /// `list`, `refresh`.
    List {
        /// Projects in the user's order.
        projects: Vec<ProjectView>,
        /// Session id -> project id; `null` = seen, not in a project. A session missing here was never seen.
        members: BTreeMap<String, Option<String>>,
        /// True once this run reconciled with Vibe's full session list.
        swept: bool,
    },
    /// `create`.
    Created {
        /// The new project.
        project: ProjectView,
    },
    /// `delete`, `move`: how many sessions changed project (none was deleted).
    Moved {
        /// Sessions re-filed.
        moved: u32,
    },
    /// `rename`, `place`, `setFolder`.
    Done {},
}
