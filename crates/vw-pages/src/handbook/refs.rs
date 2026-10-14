//! Link and resource checks of the validator (Handbook Format 1.2 §6.4, §8): where a relative link of a page lands
//! inside the handbook, whether its file and `#id` exist (HB-E06), network resources (HB-E07), stale targets
//! (HB-W07), script hooks (HB-W11), links leaving the folder (HB-W12) and the shared stylesheet (HB-W13).
//!
//! Depends on: `links` (resolution), `handbook::{files, format, validate}`, `page`. Called by: `handbook::rules`,
//! `handbook::validate`.
//! Invariant: a link is resolved only inside the handbook folder (`..` past its top, a leading `/` or a hidden
//! segment is "outside" and never followed); the existence check of a non-page file stays inside the folder
//! (canonical check).

use std::collections::HashSet;
use std::path::Path;

use crate::handbook::files::{FolderScan, Found, inside};
use crate::handbook::format;
use crate::handbook::validate::Report;
use crate::html::clip;
use crate::links::{Target, has_scheme, resolve};

/// Where a link lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolved {
    /// A file (and `#id`) of the handbook that exists.
    Ok,
    /// Another scheme or `//`: not checked here.
    External,
    /// Leaves the handbook folder.
    Outside,
    /// Points at nothing; the reason in plain words.
    Missing(String),
}

/// Resolves `href` found on `page` against the scanned `folder` rooted at `root`.
pub(crate) fn resolve_in(root: &Path, folder: &FolderScan, page: &str, href: &str) -> Resolved {
    match resolve(page, href) {
        Target::External => Resolved::External,
        Target::Outside => Resolved::Outside,
        Target::SamePage(fragment) => {
            let own = folder.page(page);
            if fragment.is_empty() || own.is_none_or(|f| has_id(f, &fragment)) {
                Resolved::Ok
            } else {
                Resolved::Missing(format!("this page has no id \"{}\"", clip(&fragment, 60)))
            }
        }
        Target::Local { path, fragment } => {
            if let Some(target) = folder.page(&path) {
                return match fragment {
                    Some(f) if !has_id(target, &f) => {
                        Resolved::Missing(format!("{path} has no id \"{}\"", clip(&f, 60)))
                    }
                    _ => Resolved::Ok,
                };
            }
            let file = path.split('/').fold(root.to_path_buf(), |p, s| p.join(s));
            if file.is_file() && inside(root, &file) {
                Resolved::Ok
            } else {
                Resolved::Missing(format!("{path} does not exist"))
            }
        }
    }
}

/// True when `found` has element id `id` (Markdown: a heading slug).
fn has_id(found: &Found, id: &str) -> bool {
    found.scan.detail.ids.iter().any(|i| i == id) || found.scan.facts.has_section(id)
}

/// Pages the index links to directly with `<a href>` (paths inside the handbook).
pub(crate) fn linked_pages(index: &Found) -> HashSet<String> {
    index
        .scan
        .detail
        .references
        .iter()
        .filter(|r| r.tag == "a")
        .filter_map(|r| match resolve(index.path.as_str(), &r.url) {
            Target::Local { path, .. } => Some(path),
            _ => None,
        })
        .collect()
}

/// `<link rel>` values that load something (the rest only name a related document).
const LOADING_RELS: &[&str] = &[
    "stylesheet",
    "icon",
    "preload",
    "prefetch",
    "modulepreload",
    "manifest",
    "apple-touch-icon",
];

/// True when a reference makes the page load something (not a link the reader follows).
fn loads(tag: &str, attr: &str, rel: &[String]) -> bool {
    match tag {
        "a" | "area" => false,
        "link" => rel.iter().any(|r| LOADING_RELS.contains(&r.as_str())),
        _ => attr == "src" || attr == "poster" || attr == "href",
    }
}

/// True for a URL that reaches the network: a scheme other than `data:` and `javascript:`, or `//`.
fn is_network(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("//")
        || (has_scheme(&lower) && !lower.starts_with("data:") && !lower.starts_with("javascript:"))
}

/// HB-E06, E07, W07, W11, W12, W13 for one HTML page.
pub(crate) fn page_links(root: &Path, folder: &FolderScan, found: &Found, report: &mut Report) {
    let path = found.path.as_str();
    let detail = &found.scan.detail;
    let is_index = path == crate::paths::INDEX;
    let mut seen = HashSet::new();
    let mut stylesheet = false;
    // Format 1.2: a prototype page may run its own script (§4, §6.4); every other page never runs one.
    let prototype = found.scan.facts.kind.as_deref() == Some(format::PROTOTYPE);
    for r in &detail.references {
        let url = r.url.trim();
        if !prototype && url.to_ascii_lowercase().starts_with("javascript:") {
            report.add("HB-W11", path, format!("javascript: URL in <{}>", r.tag));
            continue;
        }
        if loads(&r.tag, r.attr, &r.rel) && is_network(url) {
            report.add(
                "HB-E07",
                path,
                format!(
                    "network resource in <{} {}>: {}",
                    r.tag,
                    r.attr,
                    clip(url, 120)
                ),
            );
            continue;
        }
        if r.tag == "link" && r.rel.iter().any(|x| x == "stylesheet") {
            stylesheet |= matches!(resolve(path, url), Target::Local { ref path, .. } if path == format::STYLESHEET);
        }
        if !seen.insert((
            url.to_owned(),
            r.tag == "a" && r.rel.iter().any(|x| x == "supersedes"),
        )) {
            continue;
        }
        match resolve_in(root, folder, path, url) {
            Resolved::Missing(reason) => report.add(
                "HB-E06",
                path,
                format!("broken link {}: {reason}", clip(url, 120)),
            ),
            Resolved::Outside => report.add(
                "HB-W12",
                path,
                format!(
                    "{} leaves the handbook folder (name repository files as <code>path</code>)",
                    clip(url, 120)
                ),
            ),
            Resolved::External => {}
            Resolved::Ok => {
                let supersedes = r.rel.iter().any(|x| x == "supersedes");
                if r.tag == "a" && !is_index && !supersedes {
                    stale_target(folder, path, url, report);
                }
            }
        }
    }
    for url in &detail.style_urls {
        if is_network(url) {
            report.add(
                "HB-E07",
                path,
                format!("network resource in CSS: {}", clip(url, 120)),
            );
        } else if let Resolved::Missing(reason) = resolve_in(root, folder, path, url) {
            report.add(
                "HB-E06",
                path,
                format!("broken CSS url {}: {reason}", clip(url, 120)),
            );
        }
    }
    script_rules(found, prototype, report);
    if !root.join(format::STYLESHEET).is_file() {
        report.add(
            "HB-W13",
            path,
            "handbook.css is missing from the handbook folder",
        );
    } else if !stylesheet {
        report.add("HB-W13", path, "the page does not link handbook.css");
    }
}

/// HB-W11 (Format 1.2): any `<script>` or `on*` attribute outside a prototype page (it never runs in a reader).
fn script_rules(found: &Found, prototype: bool, report: &mut Report) {
    let path = found.path.as_str();
    let detail = &found.scan.detail;
    let external_scripts = detail
        .references
        .iter()
        .filter(|r| r.tag == "script" && r.attr == "src")
        .count();
    if !prototype && detail.inline_scripts + external_scripts > 0 {
        report.add(
            "HB-W11",
            path,
            format!(
                "{} <script> element(s): scripts run only in prototype pages (kind prototype)",
                detail.inline_scripts + external_scripts
            ),
        );
    }
    if !prototype {
        for attr in &detail.event_attributes {
            report.add("HB-W11", path, format!("event attribute {attr}"));
        }
    }
}

/// HB-W07: a link to a page whose status is superseded, deprecated or outdated.
fn stale_target(folder: &FolderScan, page: &str, url: &str, report: &mut Report) {
    let Target::Local { path, .. } = resolve(page, url) else {
        return;
    };
    if path == page {
        return;
    }
    let Some(target) = folder.page(&path) else {
        return;
    };
    if let Some(status) = target.scan.facts.status.as_deref()
        && format::is_stale(status)
    {
        report.add(
            "HB-W07",
            page,
            format!(
                "links to {path}, which is {}",
                status.trim().to_ascii_lowercase()
            ),
        );
    }
}
