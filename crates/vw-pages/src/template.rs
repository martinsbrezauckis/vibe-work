//! The handbook look: the bundled stylesheet installed by the handbook tool as `handbook.css` (Handbook Format 1.2 §6.4; colours
//! are the app's design tokens, `ui/src/styles/tokens.css`, tuned so text and links reach WCAG AA on both
//! backgrounds; dark under the system setting, and either theme forced by `<html data-theme>`, which Vibe Work's
//! viewer sets to the app's theme), and the
//! frame the app puts around what it renders itself: a Markdown page (§7 "Display") and the live index of a folder
//! without `index.html` (§10, shown, never written).
//!
//! Depends on: `html` (escaping), `handbook::format` (status labels). Called by: `markdown` (frame), `hub` (live
//! index), the stylesheet tests.
//! Invariants: every value put into the frame is escaped here; the body given to [`document`] must already be safe
//! HTML; a stylesheet link is used only when the caller found `handbook.css` inside the handbook.

use std::fmt::Write as _;

use crate::html::escape;

/// The canonical Handbook Format 1.2 stylesheet, embedded in the app and installed by `handbook_check`.
pub const STYLE: &str = include_str!("handbook/handbook.css");

/// The visible header of a page the app renders (a Markdown page, the live index).
pub struct HeaderParts<'a> {
    /// Kind label ("Decision", "Document", "Handbook").
    pub kind: &'a str,
    /// The title (the page's h1).
    pub title: &'a str,
    /// An id for the h1 (the slug of a Markdown page's leading heading), if any.
    pub title_id: Option<&'a str>,
    /// Status token, shown capitalised.
    pub status: Option<&'a str>,
    /// `YYYY-MM-DD` of the last update.
    pub modified: Option<&'a str>,
    /// `(rel, href)` links, already checked by the caller to be relative.
    pub related: &'a [(&'a str, &'a str)],
}

/// The `<header>` of §6.1 for `parts` (every value escaped).
#[must_use]
pub fn header(parts: &HeaderParts<'_>) -> String {
    let mut out = format!(
        "<header>\n<p class=\"hb-kind\">{}</p>\n<h1{}>{}</h1>\n",
        escape(parts.kind),
        parts
            .title_id
            .map(|id| format!(" id=\"{}\"", escape(id)))
            .unwrap_or_default(),
        escape(parts.title)
    );
    let mut meta = Vec::new();
    if let Some(status) = parts.status {
        meta.push(format!(
            "<span data-field=\"status\">{}</span>",
            escape(&crate::handbook::format::capitalize(status))
        ));
    }
    if let Some(date) = parts.modified {
        meta.push(format!(
            "updated <time data-field=\"modified\" datetime=\"{0}\">{0}</time>",
            escape(date)
        ));
    }
    if !meta.is_empty() {
        let _ = writeln!(out, "<p class=\"hb-meta\">{}</p>", meta.join(" · "));
    }
    if !parts.related.is_empty() {
        out.push_str("<nav class=\"hb-related\" aria-label=\"Related pages\">");
        for (rel, href) in parts.related {
            let _ = write!(
                out,
                "<a rel=\"{}\" href=\"{}\">{}</a>",
                escape(rel),
                escape(href),
                escape(href)
            );
        }
        out.push_str("</nav>\n");
    }
    out.push_str("</header>\n");
    out
}

/// A whole document: `body` (already safe HTML: header + main), the escaped `title`, and either a link to the
/// handbook's own `stylesheet` (a relative href) or the built-in [`STYLE`] inline.
#[must_use]
pub fn document(title: &str, stylesheet: Option<&str>, body: &str) -> String {
    let style = stylesheet.map_or_else(
        || format!("<style>\n{STYLE}\n</style>"),
        |href| format!("<link rel=\"stylesheet\" href=\"{}\">", escape(href)),
    );
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" \
         content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n{style}\n</head>\n<body>\n{body}\n\
         </body>\n</html>\n",
        escape(title)
    )
}
