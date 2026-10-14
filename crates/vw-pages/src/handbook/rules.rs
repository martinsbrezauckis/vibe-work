//! The validator's rules for one HTML page of Handbook Format 1.2 (§4, §5, §6, §9, §10): head fields (HB-E01..E04,
//! W01, W15, I01), steps (HB-E05, W10, I02), ids and sections (HB-E08, W08), the visible header (HB-W02), file
//! naming (HB-W05), relations (HB-W06), headings and images (HB-W09), size limits (HB-W14); links and resources are
//! checked in `handbook::refs`.
//!
//! Depends on: `handbook::{files, format, refs, validate}`, `page`. Called by: `handbook::validate`.
//! Invariant: a page from a newer minor or major version keeps its unknown kind or status as information (HB-I01)
//! instead of an error (§9 "shown as-is and reported as information").

use std::collections::HashSet;
use std::path::Path;

use crate::handbook::files::{FolderScan, Found};
use crate::handbook::format::{self, FORMAT_MAJOR, FORMAT_MINOR, KindSpec};
use crate::handbook::validate::Report;
use crate::html::clip;
use crate::paths::INDEX;

/// Every page rule for `found` (an HTML page with a 1.0 head).
pub(crate) fn page_rules(root: &Path, folder: &FolderScan, found: &Found, report: &mut Report) {
    let path = found.path.as_str();
    let newer = version_rules(found, report);
    let kind = kind_rules(found, newer, report);
    head_rules(found, kind, newer, report);
    step_rules(found, kind, report);
    input_rules(found, newer, report);
    structure_rules(folder, found, kind, report);
    crate::handbook::refs::page_links(root, folder, found, report);
    let detail = &found.scan.detail;
    if detail.truncated || detail.over_limits {
        report.add(
            "HB-W14",
            path,
            "page over 256 KB, over 500 sections or over 200 steps: the rest is not read",
        );
    }
}

/// HB-W01 (format), W15, I01 (unknown fields). Returns true when the page is from a newer version.
fn version_rules(found: &Found, report: &mut Report) -> bool {
    let path = found.path.as_str();
    let facts = &found.scan.facts;
    let Some(text) = facts.format.as_deref() else {
        report.add("HB-W01", path, "handbook.format is missing");
        return false;
    };
    let Some((major, minor)) = format::parse_version(text) else {
        report.add(
            "HB-W01",
            path,
            format!("handbook.format \"{}\" is not MAJOR.MINOR", clip(text, 20)),
        );
        return false;
    };
    if major > FORMAT_MAJOR {
        report.add(
            "HB-W15",
            path,
            format!(
                "handbook.format {major}.{minor} is newer than {}: read best-effort",
                format::FORMAT
            ),
        );
    }
    let newer = major > FORMAT_MAJOR || (major == FORMAT_MAJOR && minor > FORMAT_MINOR);
    if newer {
        for field in &found.scan.detail.unknown_fields {
            report.add(
                "HB-I01",
                path,
                format!("field {field} from a newer version"),
            );
        }
    }
    newer
}

/// HB-E10 (input markup that breaks an input), HB-W17 (an incomplete input), HB-I01 (an input type from a newer
/// version), HB-I05 (a Format 1.2 feature on a page that says an older version).
fn input_rules(found: &Found, newer: bool, report: &mut Report) {
    let path = found.path.as_str();
    let detail = &found.scan.detail;
    for problem in &detail.input_problems {
        report.add("HB-E10", path, clip(problem, 200));
    }
    for kind in &detail.unknown_input_types {
        if newer {
            report.add(
                "HB-I01",
                path,
                format!("input type \"{}\" from a newer version", clip(kind, 40)),
            );
        } else {
            report.add(
                "HB-E10",
                path,
                format!(
                    "input type \"{}\" is not {}",
                    clip(kind, 40),
                    format::INPUT_TYPES.join(", ")
                ),
            );
        }
    }
    for warning in &detail.input_warnings {
        report.add("HB-W17", path, clip(warning, 200));
    }
    let facts = &found.scan.facts;
    let uses = if !facts.inputs.is_empty() {
        Some("inputs (§6.6)")
    } else if facts.kind.as_deref() == Some(format::PROTOTYPE) {
        Some("the prototype kind")
    } else {
        None
    };
    let older = facts
        .format
        .as_deref()
        .and_then(format::parse_version)
        .is_some_and(|(major, minor)| major == 1 && minor < 2);
    if let (Some(feature), true) = (uses, older) {
        report.add(
            "HB-I05",
            path,
            format!("the page uses {feature}, added in 1.2: set handbook.format to 1.2"),
        );
    }
}

/// HB-E01: the kind. Returns the kind's spec when it is one of §4.
fn kind_rules(found: &Found, newer: bool, report: &mut Report) -> Option<&'static KindSpec> {
    let path = found.path.as_str();
    match found.scan.facts.kind.as_deref() {
        None => {
            report.add("HB-E01", path, "handbook.kind is missing");
            None
        }
        Some(k) => match format::kind(k) {
            Some(spec) => Some(spec),
            None if newer => {
                report.add(
                    "HB-I01",
                    path,
                    format!("kind \"{}\" from a newer version", clip(k, 40)),
                );
                None
            }
            None => {
                let names: Vec<&str> = format::KINDS.iter().map(|k| k.name).collect();
                report.add(
                    "HB-E01",
                    path,
                    format!(
                        "kind \"{}\" is not a handbook kind ({})",
                        clip(k, 40),
                        names.join(", ")
                    ),
                );
                None
            }
        },
    }
}

/// HB-E02, E03, E04, W01 (description), W02.
fn head_rules(found: &Found, kind: Option<&KindSpec>, newer: bool, report: &mut Report) {
    let path = found.path.as_str();
    let facts = &found.scan.facts;
    let detail = &found.scan.detail;
    if facts.title.is_none() {
        report.add("HB-E02", path, "<title> is missing");
    }
    match (&facts.status, kind) {
        (None, _) => report.add("HB-E03", path, "handbook.status is missing"),
        (Some(status), Some(spec)) if !spec.statuses.contains(&status.as_str()) => {
            if newer {
                report.add(
                    "HB-I01",
                    path,
                    format!("status \"{}\" from a newer version", clip(status, 40)),
                );
            } else {
                report.add(
                    "HB-E03",
                    path,
                    format!(
                        "status \"{}\" is not a{} {} status ({})",
                        clip(status, 40),
                        if spec.name.starts_with(['a', 'e', 'i', 'o', 'u']) {
                            "n"
                        } else {
                            ""
                        },
                        spec.name,
                        spec.statuses.join(", ")
                    ),
                );
            }
        }
        _ => {}
    }
    date_rules(
        path,
        facts.created.as_deref(),
        facts.modified.as_deref(),
        report,
    );
    if facts.description.is_none() {
        report.add("HB-W01", path, "description is missing");
    }
    if let Some(status) = facts.status.as_deref() {
        match detail.visible_status.as_deref() {
            None => report.add(
                "HB-W02",
                path,
                "the header does not show the status (data-field=\"status\")",
            ),
            Some(shown) if !shown.trim().eq_ignore_ascii_case(status.trim()) => report.add(
                "HB-W02",
                path,
                format!(
                    "visible status \"{}\" differs from handbook.status \"{}\"",
                    clip(shown, 40),
                    clip(status, 40)
                ),
            ),
            Some(_) => {}
        }
    }
    if let Some(modified) = facts.modified.as_deref() {
        match detail.visible_modified.as_deref() {
            None => report.add(
                "HB-W02",
                path,
                "the header does not show the update date (data-field=\"modified\")",
            ),
            Some(shown) if shown.trim() != modified.trim() => report.add(
                "HB-W02",
                path,
                format!(
                    "visible update date \"{}\" differs from dcterms.modified {}",
                    clip(shown, 40),
                    clip(modified, 20)
                ),
            ),
            Some(_) => {}
        }
    }
}

/// HB-E04: both dates present, `YYYY-MM-DD`, `modified` not before `created`.
fn date_rules(path: &str, created: Option<&str>, modified: Option<&str>, report: &mut Report) {
    let mut ok = true;
    for (name, value) in [("dcterms.created", created), ("dcterms.modified", modified)] {
        match value {
            None => {
                report.add("HB-E04", path, format!("{name} is missing"));
                ok = false;
            }
            Some(v) if !format::is_date(v) => {
                report.add(
                    "HB-E04",
                    path,
                    format!("{name} \"{}\" is not YYYY-MM-DD", clip(v, 30)),
                );
                ok = false;
            }
            Some(_) => {}
        }
    }
    if ok
        && let (Some(c), Some(m)) = (created, modified)
        && m < c
    {
        report.add(
            "HB-E04",
            path,
            format!("dcterms.modified {m} is before dcterms.created {c}"),
        );
    }
}

/// HB-E05, W10, I02.
fn step_rules(found: &Found, kind: Option<&KindSpec>, report: &mut Report) {
    let path = found.path.as_str();
    let facts = &found.scan.facts;
    for problem in &found.scan.detail.step_problems {
        report.add("HB-E05", path, clip(problem, 200));
    }
    let doing = found
        .scan
        .detail
        .step_raw
        .iter()
        .filter(|s| s.as_deref() == Some("doing"))
        .count();
    if doing > 1 {
        report.add(
            "HB-I02",
            path,
            format!("{doing} steps are doing (keep at most one)"),
        );
    }
    if kind.is_none_or(|k| k.name != "plan") || facts.steps.is_empty() {
        return;
    }
    let finished = |s: &&crate::page::Step| format::STEP_FINISHED.contains(&s.status.as_str());
    let all_finished = facts.steps.iter().all(|s| finished(&s));
    match facts.status.as_deref() {
        Some("active") if all_finished => report.add(
            "HB-W10",
            path,
            "the plan is active but every step is done or skipped (set it to done)",
        ),
        Some("done") if !all_finished => report.add(
            "HB-W10",
            path,
            format!(
                "the plan is done but {} step(s) are still open",
                facts.steps.iter().filter(|s| !finished(s)).count()
            ),
        ),
        _ => {}
    }
}

/// HB-E08, W05, W06, W08, W09.
fn structure_rules(
    folder: &FolderScan,
    found: &Found,
    kind: Option<&KindSpec>,
    report: &mut Report,
) {
    let path = found.path.as_str();
    let facts = &found.scan.facts;
    let detail = &found.scan.detail;
    let mut seen = HashSet::new();
    let mut reported = HashSet::new();
    for id in &detail.ids {
        if !seen.insert(id.as_str()) && reported.insert(id.as_str()) {
            report.add(
                "HB-E08",
                path,
                format!("id \"{}\" is used more than once", clip(id, 60)),
            );
        }
    }
    if let Some(spec) = kind
        && !format::name_matches(spec, path)
    {
        report.add(
            "HB-W05",
            path,
            format!(
                "the file name does not match kind {} ({})",
                spec.name,
                naming_hint(spec)
            ),
        );
    }
    let required: Vec<String> = match kind {
        Some(spec) if spec.name == "index" => index_sections(folder),
        Some(spec) => spec.required.iter().map(|s| (*s).to_owned()).collect(),
        None => Vec::new(),
    };
    for id in required
        .iter()
        .filter(|id| !detail.section_ids.contains(id))
    {
        report.add(
            "HB-W08",
            path,
            format!("required section id \"{id}\" is missing"),
        );
    }
    relation_rules(folder, found, report);
    let h1s = detail
        .headings
        .iter()
        .fold(0usize, |n, &level| n + usize::from(level == 1));
    if h1s != 1 {
        report.add(
            "HB-W09",
            path,
            format!("{h1s} <h1> elements (exactly one expected)"),
        );
    }
    if let Some(&[from, to]) = detail
        .headings
        .windows(2)
        .find(|w| matches!(w, [a, b] if *b > *a + 1))
    {
        report.add(
            "HB-W09",
            path,
            format!("heading level skipped: h{from} then h{to}"),
        );
    }
    if let (Some(title), Some(h1)) = (&facts.title, &facts.h1)
        && title != h1
    {
        report.add(
            "HB-W09",
            path,
            format!(
                "<title> \"{}\" differs from <h1> \"{}\"",
                clip(title, 60),
                clip(h1, 60)
            ),
        );
    }
    if detail.images_without_alt > 0 {
        report.add(
            "HB-W09",
            path,
            format!("{} <img> without alt", detail.images_without_alt),
        );
    }
}

fn naming_hint(spec: &KindSpec) -> String {
    match spec.naming {
        format::Naming::Root(name) => name.to_owned(),
        format::Naming::Slug(f) => format!("{f}/<slug>.html"),
        format::Naming::Numbered(f) => format!("{f}/NNNN-<slug>.html"),
        format::Naming::Dated(f) => format!("{f}/YYYY-MM-DD-<slug>.html"),
    }
}

/// The index's required section ids: one per kind with an open page, `other` for kind-less lite pages, `archive`.
fn index_sections(folder: &FolderScan) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut other = false;
    for found in folder.pages.iter().filter(|f| f.path.as_str() != INDEX) {
        let facts = &found.scan.facts;
        let spec = facts.kind.as_deref().and_then(format::kind);
        match spec {
            None if facts.lite => other = true,
            Some(spec) => {
                let open = facts
                    .status
                    .as_deref()
                    .is_none_or(|s| !format::is_closed(spec.name, s));
                if let (true, Some(section)) = (open, spec.index_section)
                    && !ids.iter().any(|i| i == section)
                {
                    ids.push(section.to_owned());
                }
            }
            None => {}
        }
    }
    if other {
        ids.push("other".to_owned());
    }
    ids.push("archive".to_owned());
    ids
}

/// HB-W06: `superseded` needs a `superseded-by` link; a `supersedes` target must be superseded or deprecated.
fn relation_rules(folder: &FolderScan, found: &Found, report: &mut Report) {
    let path = found.path.as_str();
    let facts = &found.scan.facts;
    if facts.status.as_deref() == Some("superseded")
        && !facts.relations.iter().any(|r| r.rel == "superseded-by")
    {
        report.add(
            "HB-W06",
            path,
            "status superseded without a rel=\"superseded-by\" link",
        );
    }
    for relation in facts.relations.iter().filter(|r| r.rel == "supersedes") {
        let crate::links::Target::Local { path: target, .. } =
            crate::links::resolve(path, &relation.href)
        else {
            continue;
        };
        let status = folder
            .page(&target)
            .and_then(|t| t.scan.facts.status.clone());
        if let Some(status) = status
            && !matches!(status.as_str(), "superseded" | "deprecated")
        {
            report.add(
                "HB-W06",
                path,
                format!(
                    "supersedes {target}, whose status is {} (expected superseded or deprecated)",
                    clip(&status, 40)
                ),
            );
        }
    }
}
