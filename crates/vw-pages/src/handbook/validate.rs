//! The handbook validator (Handbook Format 1.2 §10): reads a handbook folder and reports errors, warnings and
//! information under the spec's rule ids. Errors break the book (a page cannot be placed, dated, linked or tracked),
//! warnings mean something is missing, out of date or inconsistent, information is a hint; Markdown (lite) pages only
//! ever get hints (HB-I04).
//!
//! This module holds the result types, the driver and the rules about the book as a whole (the manifest HB-E09,
//! the index's links HB-W03 and date HB-W04, other files HB-I03, lite page hints HB-I04,
//! stylesheet integrity HB-E11 and theme rules HB-I06);
//! the rules about one HTML page are in `handbook::rules`, link rules in `handbook::refs`.
//!
//! Depends on: `handbook::{files, format, rules, refs}`, `page`, serde. Called by: `hub`, vw-core-api (the hub
//! answer, the overview's health, the agent's `handbook_check`), tests.
//! Invariants: reads files only (through `files::scan`), never edits them; findings are sorted by page, then
//! severity, then rule; every message is plain text built from capped page values.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::handbook::files::{FolderScan, Found, scan};
use crate::handbook::format;
use crate::handbook::refs::{self, Resolved};
use crate::html::clip;
use crate::paths::INDEX;

/// How bad a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The book is broken here (the app still shows the page, flagged).
    Error,
    /// The book works, but something is missing, out of date or inconsistent.
    Warning,
    /// A hint.
    Info,
}

impl Severity {
    /// The word the text report uses.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

/// One finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// The rule id (`HB-E03`).
    pub rule: String,
    /// Its severity.
    pub severity: Severity,
    /// The page (path inside the handbook) or file it is about.
    pub page: String,
    /// What is wrong, plain words.
    pub message: String,
}

/// The validator's answer for one handbook folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    /// Every finding, sorted by page, severity, rule.
    pub findings: Vec<Finding>,
    /// Errors.
    pub errors: u32,
    /// Warnings.
    pub warnings: u32,
    /// Information.
    pub infos: u32,
    /// Pages read.
    pub pages: u32,
    /// More files exist than were read.
    pub truncated: bool,
}

impl Validation {
    /// The report as text, in the spec's example layout (§10): one line per page, then one indented line per
    /// finding (`  error    HB-E03  ...`).
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut page: Option<&str> = None;
        for f in &self.findings {
            if page != Some(f.page.as_str()) {
                let _ = writeln!(out, "{}", f.page);
                page = Some(f.page.as_str());
            }
            let _ = writeln!(out, "  {:<8} {}  {}", f.severity.word(), f.rule, f.message);
        }
        if self.findings.is_empty() {
            let _ = writeln!(out, "no findings ({} pages)", self.pages);
        }
        out
    }

    /// Findings about `page`.
    pub fn of<'a>(&'a self, page: &'a str) -> impl Iterator<Item = &'a Finding> {
        self.findings.iter().filter(move |f| f.page == page)
    }
}

/// Collects findings while the rules run.
#[derive(Default)]
pub(crate) struct Report {
    findings: Vec<Finding>,
}

impl Report {
    /// Adds a finding; the severity comes from the rule id's letter (`E`, `W`, `I`).
    pub(crate) fn add(&mut self, rule: &str, page: &str, message: impl Into<String>) {
        let severity = match rule.as_bytes().get(3) {
            Some(b'E') => Severity::Error,
            Some(b'W') => Severity::Warning,
            _ => Severity::Info,
        };
        // Bound the report: a page that breaks every rule a thousand times still gives a readable answer.
        if self.findings.iter().filter(|f| f.page == page).count() >= 100 {
            return;
        }
        self.findings.push(Finding {
            rule: rule.to_owned(),
            severity,
            page: page.to_owned(),
            message: message.into(),
        });
    }
}

/// Validates the handbook in `root` (a folder; a missing folder gives one HB-E09 finding).
#[must_use]
pub fn validate(root: &Path) -> Validation {
    validate_scan(root, &scan(root))
}

/// Validates an already scanned folder (`root` is used to check that linked non-page files exist).
#[must_use]
pub fn validate_scan(root: &Path, folder: &FolderScan) -> Validation {
    let mut report = Report::default();
    let index = folder.page(INDEX);
    let manifest = index;
    let linked = manifest.map(refs::linked_pages);
    match index {
        None => report.add(
            "HB-E09",
            INDEX,
            "no index.html: the handbook has no manifest (§3)",
        ),
        Some(found) => manifest_rules(found, &mut report),
    }
    for found in &folder.pages {
        if found.scan.facts.lite {
            lite_rules(root, folder, found, linked.as_ref(), &mut report);
        } else {
            crate::handbook::rules::page_rules(root, folder, found, &mut report);
        }
    }
    if let (Some(index), Some(linked)) = (manifest, &linked) {
        index_rules(folder, index, linked, &mut report);
    }
    for other in &folder.others {
        let allowed =
            other == format::STYLESHEET || other.starts_with(&format!("{}/", format::ASSETS));
        if !allowed {
            report.add(
                "HB-I03",
                other,
                "not a page, handbook.css or a file under assets/",
            );
        }
    }
    if let Some(problem) = super::stylesheet::problem(root) {
        report.add("HB-E11", format::STYLESHEET, problem);
    }
    theme_rule(root, &mut report);
    finish(report, folder)
}

/// HB-I06 (Format 1.2 §6.4): `handbook.css` has no `[data-theme]` rules, so a reader with its own theme (Vibe Work's
/// light or dark) cannot pass it to the pages; they follow the system setting instead. Reads at most 256 KB.
fn theme_rule(root: &Path, report: &mut Report) {
    let Ok(Some(css)) = super::stylesheet::read(root) else {
        return;
    };
    let compact: String = css
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect::<String>()
        .to_ascii_lowercase();
    let light = compact.contains(&format!("[{}=light]", format::THEME_ATTRIBUTE));
    let dark = compact.contains(&format!("[{}=dark]", format::THEME_ATTRIBUTE));
    if !(light && dark) {
        report.add(
            "HB-I06",
            format::STYLESHEET,
            "no [data-theme=light] and [data-theme=dark] rules: pages follow the system theme, not the app's (§6.4)",
        );
    }
}

fn finish(report: Report, folder: &FolderScan) -> Validation {
    let mut findings = report.findings;
    findings.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then(a.severity.cmp(&b.severity))
            .then(a.rule.cmp(&b.rule))
    });
    let count = |s: Severity| {
        u32::try_from(findings.iter().filter(|f| f.severity == s).count()).unwrap_or(u32::MAX)
    };
    Validation {
        errors: count(Severity::Error),
        warnings: count(Severity::Warning),
        infos: count(Severity::Info),
        pages: u32::try_from(folder.pages.len()).unwrap_or(u32::MAX),
        truncated: folder.truncated,
        findings,
    }
}

/// HB-E09: the manifest names the project and says what it is.
fn manifest_rules(index: &Found, report: &mut Report) {
    let facts = &index.scan.facts;
    if facts.kind.as_deref() != Some("index") {
        report.add(
            "HB-E09",
            INDEX,
            "index.html is not a manifest: its handbook.kind is not \"index\"",
        );
        return;
    }
    if facts.project.is_none() {
        report.add("HB-E09", INDEX, "the manifest lacks handbook.project");
    }
    if facts.description.is_none() {
        report.add("HB-E09", INDEX, "the manifest lacks description");
    }
}

/// HB-W03 (every HTML page linked from the index) and HB-W04 (index date not older than the newest page).
fn index_rules(folder: &FolderScan, index: &Found, linked: &HashSet<String>, report: &mut Report) {
    for found in &folder.pages {
        let path = found.path.as_str();
        let facts = &found.scan.facts;
        if path == INDEX || facts.lite || linked.contains(path) {
            continue;
        }
        report.add(
            "HB-W03",
            INDEX,
            format!("{path} is not linked from the index"),
        );
    }
    let Some(index_date) = index
        .scan
        .facts
        .modified
        .as_deref()
        .filter(|d| format::is_date(d))
    else {
        return;
    };
    let newest = folder
        .pages
        .iter()
        .filter(|f| f.path.as_str() != INDEX && !f.scan.facts.lite)
        .filter_map(|f| {
            let date = f.scan.facts.modified.as_deref()?;
            format::is_date(date).then_some((date, f.path.as_str()))
        })
        .max();
    if let Some((date, page)) = newest
        && date > index_date
    {
        report.add(
            "HB-W04",
            INDEX,
            format!("index dcterms.modified {index_date} is older than {page} ({date})"),
        );
    }
}

/// HB-I04: hints for a Markdown page (front matter, broken links, not listed in the index).
fn lite_rules(
    root: &Path,
    folder: &FolderScan,
    found: &Found,
    linked: Option<&HashSet<String>>,
    report: &mut Report,
) {
    let path = found.path.as_str();
    for problem in &found.scan.detail.lite_problems {
        report.add("HB-I04", path, clip(problem, 200));
    }
    for href in &found.scan.facts.links {
        match refs::resolve_in(root, folder, path, href) {
            Resolved::Missing(reason) => report.add(
                "HB-I04",
                path,
                format!("broken link {}: {reason}", clip(href, 120)),
            ),
            Resolved::Ok | Resolved::External | Resolved::Outside => {}
        }
    }
    if linked.is_some_and(|l| !l.contains(path)) {
        report.add("HB-I04", path, "not listed in the index");
    }
}
