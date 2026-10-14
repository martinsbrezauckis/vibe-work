//! The live hub of a project handbook (Handbook Format 1.2 §8, §10): every page of the folder grouped the way an
//! index lists them (roadmap and changelog first, one group per kind, Other documents, Archive), built from the page
//! heads, with the validator's findings and the manifest. The app shows it; it never writes it: `index.html` is the
//! agent's page (§8). When the folder has no `index.html`, the page server answers its root with this hub as HTML
//! ([`live_index_html`], marked as built by the app, never saved).
//!
//! Depends on: `handbook::{files, format, overview, validate}`, `page`, `template`, `html`. Called by: `service`,
//! `server` (the live index), vw-core-api (hub requests), tests.
//! Invariants: one scan per build (files read once, then validated); never writes; every page text in the live
//! index HTML is escaped.

use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::handbook::Folder;
use crate::handbook::files::{Found, scan};
use crate::handbook::format;
use crate::handbook::overview::Manifest;
use crate::handbook::validate::{Validation, validate_scan};
use crate::html::escape;
use crate::paths::INDEX;

/// Groups in hub order: `(id, label)`. `top` holds the roadmap and the changelog.
pub const GROUPS: &[(&str, &str)] = &[
    ("top", "Roadmap and changelog"),
    ("plans", "Plans"),
    ("decisions", "Decisions"),
    ("requests", "Requests"),
    ("research", "Research"),
    ("reports", "Reports"),
    ("guides", "Guides"),
    ("reference", "Reference"),
    ("explanation", "Explanation"),
    ("prototypes", "Prototypes"),
    ("other", "Other documents"),
    ("archive", "Archive"),
];

/// One page in the hub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HubPage {
    /// Path inside the handbook.
    pub path: String,
    /// Title (the file name when the page has none).
    pub title: String,
    /// The kind (`handbook.kind`, front matter `kind`, or a pre-1.0 page's kind mapped to 1.0), if known.
    pub kind: Option<String>,
    /// Head status as written.
    pub status: Option<String>,
    /// `dcterms.modified` (a pre-1.0 page: `vw-date`).
    pub modified: Option<String>,
    /// `description`.
    pub description: Option<String>,
    /// File modification time, ms since the Unix epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub modified_ms: u64,
    /// Plan steps done or skipped.
    pub steps_done: u32,
    /// Plan steps.
    pub steps_total: u32,
    /// Inputs the page asks the user to answer (Format 1.2 §6.6).
    #[serde(default)]
    pub inputs: u32,
    /// A Markdown (lite) page.
    pub lite: bool,
    /// A pre-1.0 page (needs migration).
    pub legacy: bool,
}

/// A group of the hub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HubGroup {
    /// One of [`GROUPS`]' ids.
    pub id: String,
    /// Shown heading.
    pub label: String,
    /// Pages, newest first.
    pub pages: Vec<HubPage>,
}

/// The state of the handbook's `index.html` (what the hub says about it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum IndexHealth {
    /// A manifest that lists every page and is not older than them.
    Current,
    /// No `index.html` (HB-E09): the hub is the app's live list.
    Missing,
    /// `index.html` is not a usable manifest (HB-E09).
    NotManifest,
    /// The index misses pages or is older than the newest page (HB-W03, HB-W04).
    Stale,
    /// Written by an earlier build of the app or a pre-1.0 page (HB-W16).
    PreFormat,
}

/// The hub of one handbook folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Hub {
    /// The folder, relative to the project.
    pub folder: String,
    /// The folder or some pages are from an earlier build: a migration is suggested (§11).
    pub legacy: bool,
    /// The manifest head, when `index.html` is a manifest.
    pub manifest: Option<Manifest>,
    /// The index's state.
    pub index: IndexHealth,
    /// Non-empty groups in [`GROUPS`] order.
    pub groups: Vec<HubGroup>,
    /// Pages read.
    pub count: u32,
    /// More files exist than were read.
    pub truncated: bool,
    /// The validator's findings for the whole folder.
    pub validation: Validation,
}

impl Hub {
    /// Every page, in group order.
    pub fn pages(&self) -> impl Iterator<Item = &HubPage> {
        self.groups.iter().flat_map(|g| g.pages.iter())
    }
}

/// The 1.0 kind a pre-1.0 page's `vw-kind` maps to (§11 step 2).
fn legacy_kind(kind: &str, path: &str) -> Option<&'static str> {
    match kind {
        "plan" | "plans" => Some("plan"),
        "ask" | "asks" | "request" => Some("request"),
        "decision" | "decisions" | "adr" => Some("decision"),
        "research" => Some("research"),
        "report" | "reports" | "bench" => Some("report"),
        _ => match path.split_once('/').map(|(top, _)| top) {
            Some("plans") => Some("plan"),
            Some("asks") => Some("request"),
            Some("decisions") => Some("decision"),
            Some("reports") => Some("report"),
            Some("research") => Some("research"),
            _ => None,
        },
    }
}

/// The group id and the kind of a page. In a pre-1.0 folder a page without any head is placed by its folder.
fn place(found: &Found, legacy_folder: bool) -> (&'static str, Option<String>) {
    let facts = &found.scan.facts;
    let path = found.path.as_str();
    let headless = facts.kind.is_none() && facts.format.is_none() && !facts.lite;
    if facts.is_pre_format() || (legacy_folder && headless) {
        let legacy = facts.legacy.clone().unwrap_or_default();
        let kind = legacy_kind(legacy.kind.as_deref().unwrap_or(""), path);
        let status = legacy.status.unwrap_or_default().to_ascii_lowercase();
        let closed = ["superseded", "done", "rejected", "abandoned", "declined"]
            .iter()
            .any(|s| status.starts_with(s));
        let group = match kind.and_then(format::kind) {
            _ if closed => "archive",
            Some(spec) => spec.index_section.unwrap_or("other"),
            None => "other",
        };
        return (group, kind.map(str::to_owned));
    }
    let Some(spec) = facts.kind.as_deref().and_then(format::kind) else {
        return ("other", facts.kind.clone());
    };
    let closed = facts
        .status
        .as_deref()
        .is_some_and(|s| format::is_closed(spec.name, s));
    let group = if closed {
        "archive"
    } else if matches!(spec.name, "roadmap" | "changelog") {
        "top"
    } else {
        spec.index_section.unwrap_or("other")
    };
    (group, Some(spec.name.to_owned()))
}

/// Builds the hub of the handbook folder `root` (`folder` names it relative to the project).
#[must_use]
pub fn build(root: &Path, folder: &Folder) -> Hub {
    let scanned = scan(root);
    let validation = validate_scan(root, &scanned);
    let index = scanned.page(INDEX);
    let mut groups: Vec<HubGroup> = GROUPS
        .iter()
        .map(|(id, label)| HubGroup {
            id: (*id).to_owned(),
            label: (*label).to_owned(),
            pages: Vec::new(),
        })
        .collect();
    let mut legacy = folder.legacy;
    for found in scanned.pages.iter().filter(|f| f.path.as_str() != INDEX) {
        let facts = &found.scan.facts;
        legacy |= facts.is_pre_format();
        let (group, kind) = place(found, folder.legacy);
        let finished = facts
            .steps
            .iter()
            .filter(|s| format::STEP_FINISHED.contains(&s.status.as_str()))
            .count();
        let page = HubPage {
            path: found.path.to_string(),
            title: facts
                .title
                .clone()
                .unwrap_or_else(|| found.path.to_string()),
            kind,
            status: facts
                .status
                .clone()
                .or_else(|| facts.legacy.as_ref().and_then(|l| l.status.clone())),
            modified: facts
                .modified
                .clone()
                .or_else(|| facts.legacy.as_ref().and_then(|l| l.date.clone())),
            description: facts.description.clone(),
            modified_ms: found.modified_ms,
            steps_done: u32::try_from(finished).unwrap_or(u32::MAX),
            steps_total: u32::try_from(facts.steps.len()).unwrap_or(u32::MAX),
            inputs: u32::try_from(facts.inputs.len()).unwrap_or(u32::MAX),
            lite: facts.lite,
            legacy: facts.is_pre_format(),
        };
        if let Some(g) = groups.iter_mut().find(|g| g.id == group) {
            g.pages.push(page);
        }
    }
    for group in &mut groups {
        group.pages.sort_by(|a, b| {
            b.modified_ms
                .cmp(&a.modified_ms)
                .then_with(|| a.path.cmp(&b.path))
        });
    }
    groups.retain(|g| !g.pages.is_empty());
    let index_findings = |rules: &[&str]| {
        validation
            .of(INDEX)
            .any(|f| rules.contains(&f.rule.as_str()))
    };
    let health = match index {
        None => IndexHealth::Missing,
        Some(i) if i.scan.facts.is_pre_format() => {
            legacy = true;
            IndexHealth::PreFormat
        }
        Some(i) if i.scan.facts.kind.as_deref() != Some("index") => IndexHealth::NotManifest,
        Some(_) if index_findings(&["HB-W03", "HB-W04"]) => IndexHealth::Stale,
        Some(_) => IndexHealth::Current,
    };
    Hub {
        folder: folder.relative.clone(),
        legacy,
        manifest: index
            .filter(|i| i.scan.facts.kind.as_deref() == Some("index"))
            .map(|i| Manifest::of(&i.scan.facts)),
        index: health,
        groups,
        count: u32::try_from(scanned.pages.len()).unwrap_or(u32::MAX),
        truncated: scanned.truncated,
        validation,
    }
}

/// The live index as an HTML document (the page server's answer for a folder without `index.html`): built by the
/// app from the page heads, never saved. `project` names the folder.
#[must_use]
pub fn live_index_html(hub: &Hub, project: &str) -> String {
    let mut body = crate::template::header(&crate::template::HeaderParts {
        kind: "Handbook",
        title: &format!("{project} handbook"),
        title_id: None,
        status: None,
        modified: None,
        related: &[],
    });
    body.push_str(
        "<main>\n<p class=\"hb-meta\">This folder has no index.html yet. Vibe Work lists the pages from their \
         heads; this list is not saved.</p>\n",
    );
    for group in &hub.groups {
        let _ = write!(
            body,
            "<section id=\"{}\">\n<h2>{}</h2>\n<ul>\n",
            escape(&group.id),
            escape(&group.label)
        );
        for page in &group.pages {
            let meta = [page.status.clone(), page.description.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let _ = writeln!(
                body,
                "<li><a href=\"{}\">{}</a>{}{}</li>",
                escape(&href_of(&page.path)),
                escape(&page.title),
                if meta.is_empty() { "" } else { " — " },
                escape(&meta)
            );
        }
        body.push_str("</ul>\n</section>\n");
    }
    body.push_str("</main>\n");
    crate::template::document(&format!("{project} handbook"), None, &body)
}

/// A relative link to a page path, each segment percent-encoded where needed.
fn href_of(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        (b as char).to_string()
                    }
                    other => format!("%{other:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}
