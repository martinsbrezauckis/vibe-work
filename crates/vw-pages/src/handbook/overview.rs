//! The Projects overview's data for one project, read without opening a session (Handbook Format 1.2 §3: "The
//! Projects overview reads only this head, the heads of active plans and their steps"): the manifest head of
//! `index.html`, the heads of the files in `plans/`, and the steps of the plans whose head says `active`.
//!
//! Depends on: `page`, `lite`, `handbook::{files, format}`, serde. Called by: vw-core-api (`overview`), tests (the
//! cost measurement).
//! Invariants:
//! - Reads at most [`HEAD_BYTES`] of each head (up to `</head>` for HTML, the front matter for Markdown), and a
//!   whole page only for an active plan; at most [`MAX_PLAN_FILES`] plan files per project; never writes.
//! - Every value is capped page text (the UI shows it as plain text).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::handbook::files::{modified_ms, read_page, read_prefix};
use crate::handbook::format;
use crate::page::{PageFacts, Step, scan_html};

/// Bytes read for a head.
pub const HEAD_BYTES: usize = 64 * 1024;
/// Plan files looked at per project.
pub const MAX_PLAN_FILES: usize = 200;
/// Active plans returned per project (newest first).
pub const MAX_ACTIVE: usize = 10;

/// What kind of handbook a folder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum HandbookState {
    /// No handbook folder.
    None,
    /// A 1.0 handbook (`index.html` is a manifest).
    Handbook,
    /// Markdown files without a manifest (lite pages only).
    Lite,
    /// A folder with pages but no usable manifest (index missing or not a manifest).
    NoManifest,
    /// Pages of an earlier build (`docs/pages/`, `vw-*` names): needs migration (§11).
    PreFormat,
}

/// The manifest head (§3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// `handbook.project`.
    pub project: Option<String>,
    /// `description`.
    pub description: Option<String>,
    /// `handbook.status` (`active`, `paused`, `maintenance`, `archived`).
    pub status: Option<String>,
    /// `handbook.format`.
    pub format: Option<String>,
    /// `dcterms.created`.
    pub created: Option<String>,
    /// `dcterms.modified`.
    pub modified: Option<String>,
    /// `author` values.
    pub authors: Vec<String>,
}

impl Manifest {
    /// The manifest fields of an index page's facts.
    #[must_use]
    pub fn of(facts: &PageFacts) -> Self {
        Self {
            project: facts.project.clone(),
            description: facts.description.clone(),
            status: facts.status.clone(),
            format: facts.format.clone(),
            created: facts.created.clone(),
            modified: facts.modified.clone(),
            authors: facts.authors.clone(),
        }
    }
}

/// An active plan on a card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    /// Path inside the handbook (`plans/x.html`).
    pub path: String,
    /// Title (the file name when the page has none).
    pub title: String,
    /// `dcterms.modified`.
    pub modified: Option<String>,
    /// File modification time, ms since the epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub modified_ms: u64,
    /// Steps in page order.
    pub steps: Vec<Step>,
}

/// One project's card data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectCard {
    /// The project folder.
    pub project: String,
    /// Its folder name.
    pub name: String,
    /// The handbook folder relative to the project.
    pub folder: String,
    /// What the folder holds.
    pub state: HandbookState,
    /// The manifest head, when `index.html` is a manifest.
    pub manifest: Option<Manifest>,
    /// Active plans, newest first (at most [`MAX_ACTIVE`]).
    pub plans: Vec<PlanSummary>,
    /// Newest modification time seen (index, plans, the folder itself), ms since the epoch; 0 without a handbook.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub changed_ms: u64,
}

/// The facts of a page head: HTML up to `</head>`, Markdown front matter (plus the start of the body).
#[must_use]
pub fn read_head(file: &Path, markdown: bool) -> Option<PageFacts> {
    let (text, _) = read_prefix(file, HEAD_BYTES).ok()?;
    if markdown {
        return Some(crate::lite::parse_markdown(&text));
    }
    let end = text
        .to_ascii_lowercase()
        .find("</head")
        .unwrap_or(text.len());
    Some(scan_html(&text[..end]).facts)
}

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| modified_ms(&m))
}

/// The card of `project` whose handbook folder is `folder` (relative); `legacy` when the folder is the pre-1.0
/// `docs/pages/`.
#[must_use]
pub fn card(project: &Path, folder: &str, legacy: bool) -> ProjectCard {
    let root = folder
        .split('/')
        .fold(project.to_path_buf(), |p, s| p.join(s));
    let name = project.file_name().map_or_else(
        || project.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut card = ProjectCard {
        project: project.display().to_string(),
        name,
        folder: folder.to_owned(),
        state: HandbookState::None,
        manifest: None,
        plans: Vec::new(),
        changed_ms: 0,
    };
    if !root.is_dir() {
        return card;
    }
    card.changed_ms = mtime(&root);
    let index_file = root.join(crate::paths::INDEX);
    let index = index_file
        .is_file()
        .then(|| read_head(&index_file, false))
        .flatten();
    card.changed_ms = card.changed_ms.max(mtime(&index_file));
    card.state = match &index {
        Some(facts) if facts.is_pre_format() => HandbookState::PreFormat,
        _ if legacy => HandbookState::PreFormat,
        Some(facts) if facts.kind.as_deref() == Some("index") => {
            card.manifest = Some(Manifest::of(facts));
            HandbookState::Handbook
        }
        None if has_markdown(&root) => HandbookState::Lite,
        _ => HandbookState::NoManifest,
    };
    card.plans = active_plans(&root, card.state == HandbookState::PreFormat);
    if let Some(newest) = card.plans.iter().map(|p| p.modified_ms).max() {
        card.changed_ms = card.changed_ms.max(newest);
    }
    card
}

fn has_markdown(root: &Path) -> bool {
    std::fs::read_dir(root).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|e| {
            e.file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".md")
        })
    })
}

/// Active plans of `root/plans/`: by head status `active` (a pre-1.0 plan: any plan with an open step).
fn active_plans(root: &Path, pre_format: bool) -> Vec<PlanSummary> {
    let Ok(entries) = std::fs::read_dir(root.join("plans")) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| crate::paths::segment_allowed(n) && crate::paths::is_page_name(n))
        .collect();
    names.sort();
    names.truncate(MAX_PLAN_FILES);
    let mut plans = Vec::new();
    for name in names {
        let file = root.join("plans").join(&name);
        let markdown = name.to_ascii_lowercase().ends_with(".md");
        if !std::fs::symlink_metadata(&file).is_ok_and(|m| m.is_file()) {
            continue;
        }
        let active = pre_format
            || read_head(&file, markdown).is_some_and(|h| h.status.as_deref() == Some("active"));
        if !active {
            continue;
        }
        let Some(scan) = read_page(&file, &format!("plans/{name}")) else {
            continue;
        };
        let facts = scan.facts;
        let open = facts
            .steps
            .iter()
            .any(|s| !format::STEP_FINISHED.contains(&s.status.as_str()));
        if pre_format && (facts.steps.is_empty() || !open) {
            continue;
        }
        plans.push(PlanSummary {
            path: format!("plans/{name}"),
            title: facts.title.clone().unwrap_or_else(|| name.clone()),
            modified: facts
                .modified
                .clone()
                .or_else(|| facts.legacy.as_ref().and_then(|l| l.date.clone())),
            modified_ms: mtime(&file),
            steps: facts.steps,
        });
    }
    plans.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.path.cmp(&b.path))
    });
    plans.truncate(MAX_ACTIVE);
    plans
}
