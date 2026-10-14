//! The fixed vocabulary of Project Handbook Format 1.2 (`docs/public/HANDBOOK_FORMAT.md`): the closed list of page
//! kinds (§4) with their statuses, closed statuses, required section ids and file-name rules (§2), the project
//! statuses of the manifest (§3), plan step states (§6.3), the input types (§6.6, added in 1.2), the theme attribute
//! (§6.4) and the Markdown front matter keys (§7).
//!
//! Depends on: nothing. Called by: `page` (step states), `handbook::validate`, `hub` (groups, archive), the overview,
//! tests.
//! Invariant: every table here is the spec's text; a change of the spec changes this file and its tests together.

/// The format version this reader implements.
pub const FORMAT: &str = "1.2";
/// Major version of [`FORMAT`].
pub const FORMAT_MAJOR: u32 = 1;
/// Minor version of [`FORMAT`].
pub const FORMAT_MINOR: u32 = 2;

/// The default handbook folder, relative to the project folder (§2).
pub const FOLDER: &str = "docs/handbook";
/// The folder earlier builds used (§11); read when [`FOLDER`] does not exist.
pub const LEGACY_FOLDER: &str = "docs/pages";
/// The shared stylesheet's file name at the handbook root (§6.4).
pub const STYLESHEET: &str = "handbook.css";
/// The assets folder (§2).
pub const ASSETS: &str = "assets";

/// Project statuses of the manifest (§3), in display order.
pub const PROJECT_STATUSES: &[&str] = &["active", "paused", "maintenance", "archived"];

/// Plan step states (§6.3).
pub const STEP_STATES: &[&str] = &["todo", "doing", "done", "blocked", "skipped"];
/// Step states that count as finished (progress counts them; a plan whose steps are all finished SHOULD be done).
pub const STEP_FINISHED: &[&str] = &["done", "skipped"];

/// Input types (§6.6, Format 1.2): pick one option, tick any options, put options in order, write a short answer.
pub const INPUT_TYPES: &[&str] = &["choice", "checklist", "rank", "text"];
/// Most inputs read per page (HB-W17 beyond it).
pub const MAX_INPUTS: usize = 50;
/// Most options read per input (HB-W17 beyond it).
pub const MAX_OPTIONS: usize = 100;

/// The kind whose pages may run their own script in a reader (§4, Format 1.2).
pub const PROTOTYPE: &str = "prototype";

/// The attribute a reader with its own theme sets on `<html>` (`light` or `dark`; §6.4, Format 1.2).
pub const THEME_ATTRIBUTE: &str = "data-theme";

/// Statuses that make a link to the page stale (HB-W07).
pub const STALE_STATUSES: &[&str] = &["superseded", "deprecated", "outdated"];

/// Front matter keys of a lite page (§7).
pub const FRONT_MATTER_KEYS: &[&str] = &[
    "title",
    "kind",
    "status",
    "created",
    "updated",
    "date",
    "summary",
    "related",
    "supersedes",
    "superseded-by",
];

/// How a kind's files are named (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Naming {
    /// Exactly this file at the handbook root (`index.html`, `roadmap.html`, `changelog.html`).
    Root(&'static str),
    /// `<folder>/<slug>.html`.
    Slug(&'static str),
    /// `<folder>/NNNN-<slug>.html` (4-digit number).
    Numbered(&'static str),
    /// `<folder>/YYYY-MM-DD-<slug>.html`.
    Dated(&'static str),
}

/// One page kind of §4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindSpec {
    /// The kind token (`handbook.kind`).
    pub name: &'static str,
    /// The label a reader shows ("Decision").
    pub label: &'static str,
    /// The heading of the kind's group in an index or hub ("Decisions").
    pub plural: &'static str,
    /// The index section id listing open pages of this kind (`None`: linked near the top, not a section).
    pub index_section: Option<&'static str>,
    /// Status tokens, the usual starting status first.
    pub statuses: &'static [&'static str],
    /// Statuses whose work is over (listed under Archive).
    pub closed: &'static [&'static str],
    /// Section ids a page of this kind must have (HB-W08).
    pub required: &'static [&'static str],
    /// File naming.
    pub naming: Naming,
}

const DOC_STATUSES: &[&str] = &["draft", "current", "outdated", "superseded"];

/// The closed list of kinds (§4), in the order a reader lists them. The index comes first.
pub const KINDS: &[KindSpec] = &[
    KindSpec {
        name: "index",
        label: "Handbook",
        plural: "Handbook",
        index_section: None,
        statuses: PROJECT_STATUSES,
        closed: &[],
        required: &[],
        naming: Naming::Root("index.html"),
    },
    KindSpec {
        name: "roadmap",
        label: "Roadmap",
        plural: "Roadmap",
        index_section: None,
        statuses: &["current"],
        closed: &[],
        required: &["now", "next", "later"],
        naming: Naming::Root("roadmap.html"),
    },
    KindSpec {
        name: "changelog",
        label: "Changelog",
        plural: "Changelog",
        index_section: None,
        statuses: &["current"],
        closed: &[],
        required: &["unreleased"],
        naming: Naming::Root("changelog.html"),
    },
    KindSpec {
        name: "plan",
        label: "Plan",
        plural: "Plans",
        index_section: Some("plans"),
        statuses: &[
            "draft",
            "active",
            "paused",
            "done",
            "abandoned",
            "superseded",
        ],
        closed: &["done", "abandoned", "superseded"],
        required: &["goal", "steps"],
        naming: Naming::Slug("plans"),
    },
    KindSpec {
        name: "decision",
        label: "Decision",
        plural: "Decisions",
        index_section: Some("decisions"),
        statuses: &[
            "proposed",
            "accepted",
            "rejected",
            "deprecated",
            "superseded",
        ],
        closed: &["rejected", "deprecated", "superseded"],
        required: &["context", "options", "outcome"],
        naming: Naming::Numbered("decisions"),
    },
    KindSpec {
        name: "request",
        label: "Request",
        plural: "Requests",
        index_section: Some("requests"),
        statuses: &[
            "open",
            "accepted",
            "deferred",
            "declined",
            "done",
            "superseded",
        ],
        closed: &["declined", "done", "superseded"],
        required: &["problem", "proposal"],
        naming: Naming::Numbered("requests"),
    },
    KindSpec {
        name: "research",
        label: "Research",
        plural: "Research",
        index_section: Some("research"),
        statuses: &["draft", "final", "superseded"],
        closed: &["superseded"],
        required: &["question", "findings", "sources"],
        naming: Naming::Slug("research"),
    },
    KindSpec {
        name: "report",
        label: "Report",
        plural: "Reports",
        index_section: Some("reports"),
        statuses: &["draft", "final", "superseded"],
        closed: &["superseded"],
        required: &["scope", "method", "results"],
        naming: Naming::Dated("reports"),
    },
    KindSpec {
        name: PROTOTYPE,
        label: "Prototype",
        plural: "Prototypes",
        index_section: Some("prototypes"),
        statuses: DOC_STATUSES,
        closed: &["superseded"],
        required: &[],
        naming: Naming::Slug("prototypes"),
    },
    KindSpec {
        name: "guide",
        label: "Guide",
        plural: "Guides",
        index_section: Some("guides"),
        statuses: DOC_STATUSES,
        closed: &["superseded"],
        required: &[],
        naming: Naming::Slug("guides"),
    },
    KindSpec {
        name: "reference",
        label: "Reference",
        plural: "Reference",
        index_section: Some("reference"),
        statuses: DOC_STATUSES,
        closed: &["superseded"],
        required: &[],
        naming: Naming::Slug("reference"),
    },
    KindSpec {
        name: "explanation",
        label: "Explanation",
        plural: "Explanation",
        index_section: Some("explanation"),
        statuses: DOC_STATUSES,
        closed: &["superseded"],
        required: &[],
        naming: Naming::Slug("explanation"),
    },
];

/// The spec of kind `name` (lowercase token), if it is one of §4.
#[must_use]
pub fn kind(name: &str) -> Option<&'static KindSpec> {
    KINDS.iter().find(|k| k.name == name)
}

/// The kind whose files live in folder `folder` (`plans` → plan), if any.
#[must_use]
pub fn kind_of_folder(folder: &str) -> Option<&'static KindSpec> {
    KINDS.iter().find(|k| match k.naming {
        Naming::Slug(f) | Naming::Numbered(f) | Naming::Dated(f) => f == folder,
        Naming::Root(_) => false,
    })
}

/// True when `status` is a closed status of `kind` (the page goes under Archive).
#[must_use]
pub fn is_closed(kind: &str, status: &str) -> bool {
    self::kind(kind).is_some_and(|k| k.closed.contains(&status))
}

/// True when a link to a page with `status` is stale (HB-W07).
#[must_use]
pub fn is_stale(status: &str) -> bool {
    STALE_STATUSES.contains(&status.trim().to_ascii_lowercase().as_str())
}

/// `(major, minor)` of a `handbook.format` value (`1.1`); `None` when it is not `MAJOR.MINOR` digits.
#[must_use]
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.trim().split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.len() <= 4 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// True for a valid calendar date written `YYYY-MM-DD`.
#[must_use]
pub fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let s = text.get(r)?;
        if s.bytes().all(|c| c.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    };
    let (Some(year), Some(month), Some(day)) = (num(0..4), num(5..7), num(8..10)) else {
        return false;
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// True for an id in the section-id pattern `[a-z0-9][a-z0-9-]*` (§6.2).
#[must_use]
pub fn is_id(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// True for a file-name slug: lowercase `a-z`, `0-9` and `-` (§2), not empty.
#[must_use]
pub fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// True when `path` (relative to the handbook root) is named as §2 asks for `kind`.
#[must_use]
pub fn name_matches(kind: &KindSpec, path: &str) -> bool {
    let (folder, file) = path.rsplit_once('/').unwrap_or(("", path));
    let Some(stem) = file.strip_suffix(".html") else {
        return false;
    };
    match kind.naming {
        Naming::Root(name) => path == name,
        Naming::Slug(f) => folder == f && is_slug(stem),
        // A file name is untrusted text, so no `&str` slice below may start or end inside a multi-byte character
        // (`123456789ā-x`: byte 10 is the second half of `ā`). Each slice is either `get` (`None` on a bad cut,
        // which refuses the name) or follows a check that the byte before the cut is an ASCII digit or `-`.
        Naming::Numbered(f) => {
            folder == f
                && stem.len() > 5
                && stem.as_bytes()[..4].iter().all(u8::is_ascii_digit)
                && stem.as_bytes()[4] == b'-'
                // Byte 4 is an ASCII `-`, so byte 5 starts a character.
                && is_slug(&stem[5..])
        }
        Naming::Dated(f) => {
            folder == f
                && stem.len() > 11
                && stem.get(..10).is_some_and(is_date)
                && stem.as_bytes()[10] == b'-'
                // Byte 10 is an ASCII `-`, so byte 11 starts a character.
                && is_slug(&stem[11..])
        }
    }
}

/// A heading-style label for a status token (`superseded-by` → `Superseded-by`); display only.
#[must_use]
pub fn capitalize(token: &str) -> String {
    let mut chars = token.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
