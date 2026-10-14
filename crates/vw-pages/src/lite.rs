//! Markdown (lite) pages as handbook pages (Handbook Format 1.2 §7): front matter mapped onto the HTML head fields
//! (`title`, `kind`, `status`, `created`, `updated`/`date`, `summary`, `related`, `supersedes`, `superseded-by`),
//! headings as sections (slug ids), task list items as plan steps numbered by position (`s1`, `s2`, ...), links.
//!
//! A page with no front matter in `decisions/`, `plans/`, `reports/` or `research/` (at any depth) takes that
//! folder's kind (§7); the page path is then needed (`scan_markdown_at`).
//!
//! Depends on: `markdown` (front matter, outline), `page` (types), `handbook::format` (keys, kinds, dates). Called by:
//! `hub`, `handbook::validate`, `handbook::overview`, `markdown::render` (the frame), tests.
//! Invariants: values are length-capped page text; front matter problems are kept as hints (never errors); a value
//! is never interpreted beyond splitting `related` lists on commas.

use crate::handbook::format::{self, FRONT_MATTER_KEYS};
use crate::html::{clip, squash};
use crate::page::{
    MAX_SECTIONS, MAX_STEPS, PageDetail, PageFacts, PageScan, Relation, Section, Step,
};

/// Folders whose Markdown pages get their kind inferred when they have no front matter (§7), as
/// `(folder, kind)`. A subset of the folders of §2: the kinds a Markdown page is commonly written for.
pub const INFERRED_FOLDERS: &[(&str, &str)] = &[
    ("decisions", "decision"),
    ("plans", "plan"),
    ("reports", "report"),
    ("research", "research"),
];

/// Reads a Markdown page (facts only), without a place in a handbook: no kind is inferred.
#[must_use]
pub fn parse_markdown(source: &str) -> PageFacts {
    scan_markdown(source).facts
}

/// Reads the Markdown page at `path` (relative to the handbook folder, `/` separators); facts only.
#[must_use]
pub fn parse_markdown_at(source: &str, path: &str) -> PageFacts {
    scan_markdown_at(source, path).facts
}

/// Reads a Markdown page for validation (front matter problems are in `detail.lite_problems`), without a place in
/// a handbook: no kind is inferred.
#[must_use]
pub fn scan_markdown(source: &str) -> PageScan {
    scan_page(source, None)
}

/// Reads the Markdown page at `path` (relative to the handbook folder, `/` separators) for validation. A page
/// without front matter in one of [`INFERRED_FOLDERS`] (or below it, at any depth) gets that folder's kind and an
/// unknown status, reported as a hint (§7). Front matter always wins, even when it disagrees with the folder.
#[must_use]
pub fn scan_markdown_at(source: &str, path: &str) -> PageScan {
    scan_page(source, Some(path))
}

/// The kind the first folder of `path` stands for (`plans/2026/x.md` → `plan`); `None` for a file at the root or
/// in any other folder. Exact, case-sensitive folder names, as §2 writes them.
fn inferred_kind(path: &str) -> Option<(&'static str, &'static str)> {
    let (folder, _) = path.trim_start_matches('/').split_once('/')?;
    INFERRED_FOLDERS
        .iter()
        .find(|(f, _)| *f == folder)
        .map(|(f, k)| (*f, *k))
}

fn scan_page(source: &str, path: Option<&str>) -> PageScan {
    let (front, body) = crate::markdown::split_front_matter(source);
    let mut facts = PageFacts {
        lite: true,
        ..PageFacts::default()
    };
    let mut detail = PageDetail {
        truncated: source.len() > crate::html::MAX_INPUT,
        lite_problems: front.problems.clone(),
        ..PageDetail::default()
    };
    for (key, value) in &front.pairs {
        let value = clip(&squash(value), 300);
        match key.as_str() {
            "title" => facts.title = Some(clip(&value, 200)).filter(|v| !v.is_empty()),
            "kind" => facts.kind = Some(value.to_ascii_lowercase()),
            "status" => facts.status = Some(value),
            "created" => facts.created = Some(value),
            "updated" => facts.modified = Some(value),
            // MADR files carry `date`: read as `updated` when `updated` is absent (§7).
            "date" if front.get("updated").is_none() => facts.modified = Some(value),
            "summary" => facts.description = Some(value),
            "related" | "supersedes" | "superseded-by" => {
                for href in value.split(',').map(str::trim).filter(|h| !h.is_empty()) {
                    facts.relations.push(Relation {
                        rel: key.clone(),
                        href: clip(href, 2000),
                    });
                }
            }
            _ => {}
        }
        if !FRONT_MATTER_KEYS.contains(&key.as_str()) {
            detail
                .lite_problems
                .push(format!("unknown front matter key \"{}\"", clip(key, 40)));
        }
    }
    check_values(&facts, &front, &mut detail);
    // No front matter at all (an empty block or a malformed one is not "none"): the folder names the kind.
    if front.pairs.is_empty()
        && front.problems.is_empty()
        && let Some((folder, kind)) = path.and_then(inferred_kind)
    {
        facts.kind = Some(kind.to_owned());
        detail.lite_problems.push(format!(
            "kind \"{kind}\" inferred from the folder {folder}/; status unknown (front matter sets both)"
        ));
    }
    let outline = crate::markdown::outline(body);
    if facts.title.is_none() {
        facts.title = outline
            .headings
            .first()
            .map(|(_, _, text)| clip(&squash(text), 200));
    }
    facts.h1 = outline
        .headings
        .iter()
        .find(|(level, _, _)| *level == 1)
        .map(|(_, _, t)| clip(&squash(t), 200));
    detail.over_limits = outline.headings.len() > MAX_SECTIONS || outline.tasks.len() > MAX_STEPS;
    detail.headings = outline.headings.iter().map(|(l, _, _)| *l).collect();
    facts.sections = outline
        .headings
        .into_iter()
        .take(MAX_SECTIONS)
        .map(|(_, id, label)| Section {
            id,
            label: clip(&squash(&label), 120),
        })
        .collect();
    detail.ids = facts.sections.iter().map(|s| s.id.clone()).collect();
    facts.steps = outline
        .tasks
        .into_iter()
        .take(MAX_STEPS)
        .enumerate()
        .map(|(n, (done, text))| Step {
            id: format!("s{}", n + 1),
            status: if done { "done" } else { "todo" }.to_owned(),
            text: clip(&squash(&text), 300),
        })
        .collect();
    facts.links = outline.links.into_iter().take(2000).collect();
    facts
        .links
        .extend(facts.relations.iter().map(|r| r.href.clone()));
    PageScan { facts, detail }
}

/// Front matter values that a reader cannot use: unknown kind or status, bad dates (hints).
fn check_values(facts: &PageFacts, front: &crate::markdown::FrontMatter, detail: &mut PageDetail) {
    if let Some(kind) = facts.kind.as_deref() {
        match format::kind(kind) {
            None => detail.lite_problems.push(format!(
                "kind \"{}\" is not a handbook kind",
                clip(kind, 40)
            )),
            Some(spec) => {
                if let Some(status) = facts.status.as_deref()
                    && !spec.statuses.contains(&status)
                {
                    detail.lite_problems.push(format!(
                        "status \"{}\" is not a {} status ({})",
                        clip(status, 40),
                        spec.name,
                        spec.statuses.join(", ")
                    ));
                }
            }
        }
    }
    for key in ["created", "updated", "date"] {
        if let Some(value) = front.get(key)
            && !format::is_date(value.trim())
        {
            detail
                .lite_problems
                .push(format!("{key} \"{}\" is not YYYY-MM-DD", clip(value, 40)));
        }
    }
}
