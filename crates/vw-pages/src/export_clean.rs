//! Share a page as one file, passes 2 and 3 (see `export`): the allowlist sanitizer (`ammonia`, an
//! html5ever tree) that serializes the exported body, and the document written around it (our head with the
//! Content-Security-Policy `<meta>` first).
//!
//! Depends on: `ammonia`, `html` (escaping), `handbook::format`, base64, getrandom. Called by: `export`.
//! Invariants: every value written into the head is escaped for its context (text, or raw text with `</` broken);
//! a prototype's scripts are allowed only by this file's random nonce; nothing in the file may reach the network.

use std::collections::HashSet;
use std::fmt::Write as _;

use base64::Engine as _;

use crate::html::escape;

/// HTML tags allowed beyond `ammonia`'s defaults.
const TAGS: &[&str] = &[
    "section",
    "header",
    "main",
    "nav",
    "footer",
    "article",
    "aside",
    "figure",
    "figcaption",
    "details",
    "summary",
    "time",
    "mark",
    "video",
    "audio",
    "source",
    "picture",
];

/// SVG drawing elements (no `script`, no `foreignObject`, no `use`, no `image`, no animation).
const SVG: &[&str] = &[
    "svg",
    "g",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "title",
    "desc",
    "defs",
    "linearGradient",
    "radialGradient",
    "stop",
    "marker",
];

/// Attributes of the SVG elements (no `href`, no event handlers).
const SVG_ATTRS: &[&str] = &[
    "viewBox",
    "width",
    "height",
    "xmlns",
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "opacity",
    "fill-opacity",
    "transform",
    "d",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "points",
    "dx",
    "dy",
    "text-anchor",
    "font-size",
    "font-weight",
    "font-family",
    "offset",
    "stop-color",
    "gradientUnits",
    "markerWidth",
    "markerHeight",
    "refX",
    "refY",
    "orient",
    "preserveAspectRatio",
];

/// Per-tag attributes beyond the generic ones.
const TAG_ATTRS: &[(&str, &[&str])] = &[
    ("time", &["datetime"]),
    ("details", &["open"]),
    (
        "video",
        &[
            "src", "poster", "controls", "width", "height", "loop", "muted",
        ],
    ),
    ("audio", &["src", "controls", "loop"]),
    ("source", &["src", "type"]),
    ("ol", &["start", "reversed", "type"]),
    ("td", &["colspan", "rowspan"]),
    ("th", &["colspan", "rowspan", "scope"]),
];

/// Pass 2: the allowlist sanitizer (see the `export` module header). The HTML `<title>` never reaches it (pass 1
/// takes it into our head); `title` is allowed for SVG. `ammonia` panics on a contradictory configuration (a tag both
/// allowed and content-cleaned): this one is fixed, and `vw_handbook_export` builds it in every test.
pub(crate) fn sanitizer() -> ammonia::Builder<'static> {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(TAGS.iter().copied())
        .add_tags(SVG.iter().copied())
        .add_generic_attributes(["class", "id", "title", "style", "role", "lang", "dir"])
        .add_generic_attribute_prefixes(["data-", "aria-"])
        .clean_content_tags(HashSet::from(["script", "style", "template", "noscript"]))
        .url_relative(ammonia::UrlRelative::PassThrough)
        .add_url_schemes(["data"])
        .strip_comments(true)
        .attribute_filter(|_element, attribute, value| {
            url_allowed(attribute, value).then(|| value.into())
        });
    for (tag, attrs) in TAG_ATTRS {
        builder.add_tag_attributes(tag, attrs.iter().copied());
    }
    for tag in SVG {
        builder.add_tag_attributes(tag, SVG_ATTRS.iter().copied());
    }
    builder
}

/// The URL rule of the sanitizer (other attributes pass): `href` only to a same-page anchor, `http(s):` or `mailto:`;
/// `src`/`poster` only `data:` images, video and audio (everything inside the handbook was inlined in pass 1); `cite`
/// only `http(s):`. ammonia's own URL checks run as well (schemes `http`, `https`, `mailto`, `data`, ...).
pub(crate) fn url_allowed(attribute: &str, value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    match attribute {
        "href" => {
            v.starts_with('#')
                || v.starts_with("https://")
                || v.starts_with("http://")
                || v.starts_with("mailto:")
        }
        "src" | "poster" => ["data:image/", "data:video/", "data:audio/"]
            .iter()
            .any(|p| v.starts_with(p)),
        "cite" => v.starts_with("https://") || v.starts_with("http://"),
        _ => true,
    }
}

/// Pass 3: the document around the sanitized body.
pub(crate) fn document(
    title: &str,
    css: &str,
    body: &str,
    scripts: Option<(&str, &[(String, bool)])>,
) -> String {
    let script_src = scripts.map_or_else(
        || "'none'".to_owned(),
        |(nonce, _)| format!("'nonce-{nonce}'"),
    );
    let mut out = format!(
        "<!doctype html>\n<html>\n<head>\n<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         script-src {script_src}; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src data:; \
         form-action 'none'; base-uri 'none'\">\n<meta charset=\"utf-8\">\n<meta http-equiv=\"x-dns-prefetch-control\" \
         content=\"off\">\n<meta name=\"viewport\" \
         content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<!-- Exported from a project \
         handbook (Project Handbook Format {}): one self-contained file. -->\n<style>\n{}\n</style>\n</head>\n\
         <body>\n{body}\n",
        escape(title),
        crate::handbook::format::FORMAT,
        raw_text_safe(css, "style")
    );
    if let Some((nonce, scripts)) = scripts {
        for (text, module) in scripts {
            let _ = writeln!(
                out,
                "<script nonce=\"{nonce}\"{}>{}</script>",
                if *module { " type=\"module\"" } else { "" },
                raw_text_safe(text, "script").replace("<!--", "<\\!--")
            );
        }
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// `text` with every `</<name>` (any case) broken as `<\/<name>`, so it cannot end the raw-text element early.
pub(crate) fn raw_text_safe(text: &str, name: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let needle = format!("</{name}");
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in lower.match_indices(&needle) {
        out.push_str(&text[last..at]);
        out.push_str("<\\/");
        last = at + 2;
    }
    out.push_str(&text[last..]);
    out
}

pub(crate) fn nonce() -> Result<String, String> {
    let mut bytes = [0u8; 18];
    getrandom::fill(&mut bytes).map_err(|e| format!("no random nonce: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// `body` with a visible note saying what was left out of the file, when anything was: handbook files by path (too
/// large, missing), and only a COUNT of outside references (their addresses are not repeated in the file).
pub(crate) fn with_note(body: &str, left_out: &[String]) -> String {
    let outside = left_out
        .iter()
        .filter(|l| l.ends_with("(network resource)") || l.ends_with("(not inside the handbook)"))
        .count();
    let files: Vec<&String> = left_out
        .iter()
        .filter(|l| {
            !(l.ends_with("(network resource)") || l.ends_with("(not inside the handbook)"))
        })
        .collect();
    if left_out.is_empty() {
        return body.to_owned();
    }
    let mut out = String::with_capacity(body.len() + 256);
    out.push_str(body);
    out.push_str("\n<aside class=\"hb-export-note\"><p>Left out of this file:</p><ul>");
    for item in files.iter().take(50) {
        let _ = write!(out, "<li>{}</li>", escape(item));
    }
    if files.len() > 50 {
        let _ = write!(out, "<li>and {} more files</li>", files.len() - 50);
    }
    if outside > 0 {
        let _ = write!(
            out,
            "<li>{outside} reference(s) to addresses outside the handbook (a shared file loads nothing from the network)</li>"
        );
    }
    out.push_str("</ul></aside>");
    out
}
