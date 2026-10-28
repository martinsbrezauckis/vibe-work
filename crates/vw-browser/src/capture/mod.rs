//! Capture from a page: the `browser_capture` agent
//! tool and the Browser tab's Capture menu save what a page is made of into the session folder, under
//! `capture/<slug>-<time>/`:
//! - `design`: `DESIGN.md` (Google DESIGN.md format) + `tokens.json` (W3C Design Tokens 2025.10) from the live page;
//! - `structure`: `elements.json` (elements with layout boxes) + `summary.md`;
//! - `media`: logos, icons, images and fonts the page loaded, under `media/`, with `media.json`;
//! - `page`: Markdown, text, passive HTML or PNG, plus an explicit PDF or packed HTML choice (`formats`);
//! - `bundle`: all of the above in one folder;
//! - `before` / `after`: a full-page shot now, a matching one later, and their difference.
//!
//! Ordinary captures also write `capture.json`; one-file choices return their metadata without a sidecar.
//!
//! Depends on: `driver` (page scripts, screenshots), `files`, `design*`, `structure`, `media`, `page`, `shot`.
//! Called by: vw-core-api (the agent tool, the `browser` command for the window and the Debug API), tests.
//! Invariants:
//! - Writes stay inside the session folder (`files`); page content never chooses a path.
//! - The agent is refused on a local-network page the user opened and after the user's Stop; the user
//!   and the Debug API (acting as the user) are not.
//! - The answer keeps page words under `untrustedPageContent` with the prompt guard's verdict; the files say in their
//!   first lines that the page's content is data, not instructions.

mod colour;
pub mod design;
mod design_out;
mod design_roles;
pub mod files;
pub mod media;
mod one_file;
pub mod page;
#[cfg(target_os = "linux")]
mod pdf_links;
mod restore;
mod run;
pub mod shot;
pub mod single_html;
pub mod structure;
pub mod svg;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

pub use colour::{Rgba as Colour, contrast};
pub use design_out::{Source, design_md, tokens_json};
pub use media::{Fetch, FetchFuture, Fetched};
pub use page::Format;
pub use shot::Method;

/// What to capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum What {
    /// The design system.
    Design,
    /// Elements with layout boxes.
    Structure,
    /// Logos, icons, images, fonts.
    Media,
    /// The whole page (`formats`).
    Page,
    /// Everything.
    Bundle,
    /// A full-page shot to compare with later.
    Before,
    /// The matching shot and the difference.
    After,
}

impl What {
    /// The wire name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Structure => "structure",
            Self::Media => "media",
            Self::Page => "page",
            Self::Bundle => "bundle",
            Self::Before => "before",
            Self::After => "after",
        }
    }
}

/// A whole-page format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFormat {
    /// A text format.
    Text(Format),
    /// `page.png`.
    Png,
    /// A paginated PDF.
    Pdf,
    /// Passive HTML with packed resources.
    SingleHtml,
}

impl PageFormat {
    /// The fixed output filename for this format.
    #[must_use]
    pub fn file(self) -> &'static str {
        match self {
            Self::Text(f) => f.file(),
            Self::Png => "page.png",
            Self::Pdf => "page.pdf",
            Self::SingleHtml => "page-one-file.html",
        }
    }
}

/// One capture request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// What.
    pub what: What,
    /// Whole-page formats (`page`; default all four).
    pub formats: Vec<PageFormat>,
    /// Force one full-page method (`native` / `stitch`; the comparison only, default: the platform's best).
    pub method: Option<Method>,
}

/// Parses the tool's / command's arguments: `{what, formats?, shotMethod?}`.
///
/// # Errors
/// A plain reason naming the accepted values.
pub fn parse(args: &Value) -> Result<Request, String> {
    let what = match args.get("what").and_then(Value::as_str) {
        Some("design") => What::Design,
        Some("structure") => What::Structure,
        Some("media") => What::Media,
        Some("page") => What::Page,
        Some("bundle") => What::Bundle,
        Some("before") => What::Before,
        Some("after") => What::After,
        _ => {
            return Err(
                "what must be design, structure, media, page, bundle, before or after".to_owned(),
            );
        }
    };
    let mut formats = Vec::new();
    if let Some(list) = args.get("formats") {
        let list = list
            .as_array()
            .ok_or("formats must be a list of markdown, text, html, png, pdf, single_html")?;
        for f in list {
            let format = match f.as_str() {
                Some("markdown") => PageFormat::Text(Format::Markdown),
                Some("text") => PageFormat::Text(Format::Text),
                Some("html") => PageFormat::Text(Format::Html),
                Some("png") => PageFormat::Png,
                Some("pdf") => PageFormat::Pdf,
                Some("single_html") => PageFormat::SingleHtml,
                _ => {
                    return Err(
                        "formats must be a list of markdown, text, html, png, pdf, single_html"
                            .to_owned(),
                    );
                }
            };
            if !formats.contains(&format) {
                formats.push(format);
            }
        }
    }
    if formats.is_empty() {
        formats = vec![
            PageFormat::Text(Format::Markdown),
            PageFormat::Text(Format::Text),
            PageFormat::Text(Format::Html),
            PageFormat::Png,
        ];
    }
    let method = match args.get("shotMethod").and_then(Value::as_str) {
        None => None,
        Some("native") => Some(Method::Native),
        Some("stitch") => Some(Method::Stitch),
        Some("stitchPlain") => Some(Method::StitchPlain),
        Some(_) => return Err("shotMethod must be native, stitch or stitchPlain".to_owned()),
    };
    Ok(Request {
        what,
        formats,
        method,
    })
}

/// A `before` shot waiting for its `after`.
#[derive(Clone, Debug)]
pub(crate) struct Before {
    /// Relative to the session folder.
    pub(crate) relative: String,
    /// The page it was taken of.
    pub(crate) url: String,
    /// The method used (the after shot uses the same).
    pub(crate) method: Method,
}

/// Per-session `before` shots.
#[derive(Clone, Default)]
pub struct Captures {
    before: Arc<Mutex<HashMap<String, Before>>>,
    flights: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl Captures {
    pub(super) fn begin(&self, session: &str) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        self.flights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(session.to_owned())
            .or_default()
            .clone()
            .try_lock_owned()
            .map_err(|_| "A capture is already running in this session.".to_owned())
    }

    pub(crate) fn set_before(&self, session_id: &str, before: Before) {
        self.before
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session_id.to_owned(), before);
    }

    pub(crate) fn before(&self, session_id: &str) -> Option<Before> {
        self.before
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .cloned()
    }

    /// Forgets a session's `before` shot (session ended).
    pub fn forget(&self, session_id: &str) {
        self.before
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session_id);
    }
}

/// Where a capture goes and how media are fetched (from the core).
pub struct Target<'a> {
    /// The session folder (the first folder of the session).
    pub session_folder: &'a Path,
    /// The media fetcher.
    pub fetch: &'a dyn Fetch,
}

pub use run::OUTCOME_KEY;
