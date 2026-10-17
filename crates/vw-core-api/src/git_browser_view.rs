//! Project browser wire contract. Depends on serde and vw-git status; called by command and the generated UI API.

use crate::git_view::GitStatusView;
use serde::{Deserialize, Serialize};
use vw_git::browser_status::{BrowserFile, BrowserStatus};

/// Read, subscribe, or release a project's browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum GitBrowserRequest {
    /// Read only, no watch allocation.
    Read {
        /// Saved project id, never an arbitrary filesystem path.
        project_id: String,
    },
    /// Read and watch safe directories inside the saved project.
    Watch {
        /// Saved project id.
        project_id: String,
    },
    /// Stop watching this project.
    Unwatch {
        /// Saved project id.
        project_id: String,
    },
}
/// A repository or copy in the project tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GitBrowserNode {
    /// Stable folder identity.
    pub id: String,
    /// Absolute folder, only from bounded discovery.
    pub path: String,
    /// Folder name.
    pub name: String,
    /// repository, copy, submodule.
    pub kind: String,
    /// Repository parent for a linked copy.
    pub parent_id: Option<String>,
    /// Explicit outside-project marker.
    pub outside_project: bool,
    /// Branch, when readable.
    pub branch: Option<String>,
    /// Plain status counts and words.
    pub status: Option<BrowserStatus>,
    /// Sanitized changed paths.
    pub files: Vec<BrowserFile>,
    /// Refusal or missing-status reason; never presented as clean.
    pub restricted: Option<String>,
    /// Only an existing admitted session may be used for Git actions.
    pub session_id: Option<String>,
    /// Existing session repository selector, never an arbitrary path.
    pub repo: Option<String>,
    /// Existing Git action state; no forge calls are made.
    pub git_status: Option<GitStatusView>,
}
/// A complete bounded snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GitBrowserSnapshot {
    /// Saved project identity.
    pub project_id: String,
    /// Saved root, absent for name-only projects.
    pub folder: Option<String>,
    /// Nodes ordered by path.
    pub nodes: Vec<GitBrowserNode>,
    /// Discovery hit a budget or an unreadable directory.
    pub truncated: bool,
    /// Plain explanation of limits or watch failures.
    pub note: Option<String>,
}
/// Browser command output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GitBrowserResult {
    /// Fresh browser snapshot.
    Snapshot {
        /// View.
        snapshot: GitBrowserSnapshot,
    },
    /// Watch released.
    Stopped {},
}
