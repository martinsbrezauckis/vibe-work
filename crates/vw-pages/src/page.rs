//! What the app reads from one handbook page FILE (Project Handbook Format 1.2, `docs/public/HANDBOOK_FORMAT.md`):
//! the head (§5: `handbook.*`, `dcterms.*`, `description`, `author`), the visible header markers (`data-field`), plan
//! steps (§6.3), inputs (§6.6), sections and every `id` (§6.2), headings, images, links with their `rel`, and what the validator
//! needs to know about scripts, styles and resources (§6.4). The generated-page marker keeps prototype scripts disabled.
//!
//! This file holds the types; the HTML reader is `page_scan`.
//!
//! Depends on: `handbook::format` (step states), `page_scan` (the reader). Called by: `hub`, `handbook::validate`,
//! `handbook::overview`, `service` (section checks for comments and picks), `lite` (shares the types), tests.
//! Invariants:
//! - Every string taken from a page is untrusted text: whitespace squashed and length-capped here (titles 200,
//!   labels 120, step text 300, ids 100, meta values 300); at most 500 sections, 200 steps, 2,000 links; at most
//!   `html::MAX_INPUT` bytes of a page are read (the rest is reported as truncated, never read).
//! - Section labels and ids come from the file, never from what a page script does at run time.

use serde::{Deserialize, Serialize};

pub use crate::page_scan::{normalize_step, parse_html, scan_html};

/// Plan step states (§6.3).
pub use crate::handbook::format::STEP_STATES;

/// A plan step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Step {
    /// The step id (`data-step`; `s1`, `s2`, ... by position in a Markdown page).
    pub id: String,
    /// `todo`, `doing`, `done`, `blocked` or `skipped` (an unknown or missing value reads as `todo`; the validator
    /// reports it).
    pub status: String,
    /// The step's text.
    pub text: String,
}

/// A section a comment can cite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Section {
    /// The element id.
    pub id: String,
    /// Its heading text (or the start of its own text).
    pub label: String,
}

/// A link with a relation (§5 "Relations"): `related`, `supersedes`, `superseded-by`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    /// The relation token.
    pub rel: String,
    /// The link as written.
    pub href: String,
}

/// One option of an input (§6.6, Format 1.2): `<li data-option="id">label</li>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InputOption {
    /// The option id (`data-option`), unique in its input.
    pub id: String,
    /// Its text (untrusted page text, at most 200 characters).
    pub label: String,
}

/// An input of a page (§6.6, Format 1.2): a question the user answers in the app (`div.hb-input[data-input]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Input {
    /// The element id (answers, comments and links point at it).
    pub id: String,
    /// `choice` (pick one), `checklist` (tick any), `rank` (order the options) or `text` (a short answer); a type
    /// this reader does not know is kept as written (the validator reports it).
    pub kind: String,
    /// The question (`.hb-question` inside the input; empty when missing, HB-W17).
    pub question: String,
    /// Options in page order (none for `text`).
    pub options: Vec<InputOption>,
}

/// The facts of one page (what the UI and the hub show).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PageFacts {
    /// `<title>` (a Markdown page: front matter `title`, else its first heading).
    pub title: Option<String>,
    /// The first `<h1>`'s text.
    pub h1: Option<String>,
    /// `handbook.format`.
    pub format: Option<String>,
    /// `handbook.kind`, lowercased (Markdown: front matter `kind`).
    pub kind: Option<String>,
    /// `handbook.status` as written (status tokens are lowercase; the validator checks it).
    pub status: Option<String>,
    /// `dcterms.created`.
    pub created: Option<String>,
    /// `dcterms.modified` (Markdown: `updated`, or `date`).
    pub modified: Option<String>,
    /// `description` (Markdown: `summary`).
    pub description: Option<String>,
    /// `handbook.project` (the manifest).
    pub project: Option<String>,
    /// `author`, repeatable.
    pub authors: Vec<String>,
    /// `handbook.outcome` (reports).
    pub outcome: Option<String>,
    /// `<meta name="vw-generated" content="hub">`: a page that must not run prototype scripts.
    pub generated: bool,
    /// Links with a relation, in page order.
    pub relations: Vec<Relation>,
    /// Plan steps in page order.
    pub steps: Vec<Step>,
    /// Inputs in page order (§6.6, Format 1.2; at most 500, each with at most 100 options).
    #[serde(default)]
    pub inputs: Vec<Input>,
    /// Sections (elements with an id; steps by their step id) in page order.
    pub sections: Vec<Section>,
    /// Every `href`/`src` of the page (links, images, scripts, styles, frames).
    pub links: Vec<String>,
    /// A Markdown (lite) page.
    pub lite: bool,
}

impl PageFacts {
    /// True when the page has a section (or step) with `id`.
    #[must_use]
    pub fn has_section(&self, id: &str) -> bool {
        self.sections.iter().any(|s| s.id == id)
    }

    /// The label of section `id`.
    #[must_use]
    pub fn section_label(&self, id: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.label.as_str())
    }
}

/// One `href`/`src`/`poster` of the page with where it was found (for the validator).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The element (`a`, `img`, `link`, ...).
    pub tag: String,
    /// The attribute (`href`, `src`, `poster`).
    pub attr: &'static str,
    /// The value as written.
    pub url: String,
    /// The `rel` tokens, lowercased (`a` and `link` only).
    pub rel: Vec<String>,
}

/// What the validator needs beyond [`PageFacts`] (never sent to the UI).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageDetail {
    /// More than `html::MAX_INPUT` bytes: the rest was not read.
    pub truncated: bool,
    /// More than 500 sections or 200 steps: the rest was not kept.
    pub over_limits: bool,
    /// Every `id` attribute in page order (duplicates kept).
    pub ids: Vec<String>,
    /// Ids of `<section>` elements.
    pub section_ids: Vec<String>,
    /// Heading levels in page order (1-6).
    pub headings: Vec<u8>,
    /// `<img>` elements without an `alt` attribute.
    pub images_without_alt: usize,
    /// `<script>` elements without `src`.
    pub inline_scripts: usize,
    /// `on*` attributes found (`onclick on <button>`), at most 20.
    pub event_attributes: Vec<String>,
    /// Every href/src/poster with its element.
    pub references: Vec<Reference>,
    /// URLs from `@import` and `url()` in `<style>` elements and `style` attributes.
    pub style_urls: Vec<String>,
    /// Text of the element marked `data-field="status"`.
    pub visible_status: Option<String>,
    /// `datetime` (else text) of the element marked `data-field="modified"`.
    pub visible_modified: Option<String>,
    /// Step markup problems (HB-E05), plain words.
    pub step_problems: Vec<String>,
    /// Raw `data-status` of each step, in step order (`None`: missing).
    pub step_raw: Vec<Option<String>>,
    /// `handbook.*` meta names this reader does not know.
    pub unknown_fields: Vec<String>,
    /// Lite pages: front matter keys or values that could not be used, plain words (hints, HB-I04).
    pub lite_problems: Vec<String>,
    /// Input markup that breaks an input (HB-E10), plain words.
    pub input_problems: Vec<String>,
    /// Input markup that works but is incomplete (HB-W17), plain words.
    pub input_warnings: Vec<String>,
    /// `data-input` types this reader does not know (HB-E10, or HB-I01 on a page from a newer version).
    pub unknown_input_types: Vec<String>,
}

/// A page read for validation: facts plus detail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageScan {
    /// What the UI sees.
    pub facts: PageFacts,
    /// What the validator checks.
    pub detail: PageDetail,
}

/// Most sections kept per page.
pub(crate) const MAX_SECTIONS: usize = 500;
/// Most steps kept per page.
pub(crate) const MAX_STEPS: usize = 200;
