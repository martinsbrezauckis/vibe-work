//! The SVG allowlist of the media capture (a deny-list scan of the
//! page's markup was bypassed by a CSS escape). A captured SVG is never saved as the page wrote it: it is parsed as
//! XML and re-serialised by us with only allowlisted elements and attributes, so what lands on disk is drawing
//! markup and nothing a browser or an SVG consumer could run, fetch or import, whatever the spelling.
//!
//! - Elements: shapes, paths, text, groups, gradients, clip paths, masks, patterns, symbols, `use`, `image`, `title`,
//!   `desc` (exact, case-sensitive SVG names). Anything else is dropped with its whole subtree (`script`,
//!   `foreignObject`, `style`, `a`, animation, filters, `iframe`, namespaced elements, an upper-case `SCRIPT`).
//! - Attributes: geometry and presentation attributes from a fixed list; values decoded by the XML parser (character
//!   references resolved) and then judged: no `javascript`, `data:`, `http`, `//`, backslash or control characters,
//!   `url()` only as `url(#id)`; `href` only `#id` (on `use`) or a base64 PNG / JPEG / GIF / WebP (on `image`).
//! - Refused outright (`None`): no `<svg>` root, a DTD (`<!DOCTYPE`, entities), a processing instruction other than
//!   the XML declaration, an unknown entity, malformed or mismatched tags.
//! - Comments, CDATA and the XML declaration are dropped; text is escaped again on output.
//!
//! Depends on: quick-xml. Called by: `media`, tests.

use std::fmt::Write as _;

use quick_xml::Reader;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};

const ELEMENTS: &[&str] = &[
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
    "clipPath",
    "mask",
    "pattern",
    "symbol",
    "use",
    "image",
];

const ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "viewBox",
    "width",
    "height",
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
    "fx",
    "fy",
    "dx",
    "dy",
    "d",
    "points",
    "transform",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "opacity",
    "clip-rule",
    "clip-path",
    "mask",
    "offset",
    "stop-color",
    "stop-opacity",
    "gradientUnits",
    "gradientTransform",
    "spreadMethod",
    "patternUnits",
    "patternContentUnits",
    "patternTransform",
    "clipPathUnits",
    "maskUnits",
    "maskContentUnits",
    "preserveAspectRatio",
    "xmlns",
    "xmlns:xlink",
    "version",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-anchor",
    "dominant-baseline",
    "letter-spacing",
    "aria-label",
    "aria-hidden",
    "role",
    "visibility",
    "display",
    "href",
    "xlink:href",
];

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// `text` escaped for XML text and double-quoted attribute values.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// An `#id` fragment.
fn fragment(value: &str) -> bool {
    value.len() > 1
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c))
}

/// A base64 raster image `data:` URL.
fn raster_data(value: &str) -> bool {
    [
        "data:image/png;base64,",
        "data:image/jpeg;base64,",
        "data:image/gif;base64,",
        "data:image/webp;base64,",
    ]
    .iter()
    .find_map(|p| value.strip_prefix(p))
    .is_some_and(|b64| {
        b64.chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=\n\r ".contains(c))
    })
}

/// The attribute's decoded value if it may be kept on element `element`.
fn keep_value(element: &str, name: &str, value: &str) -> bool {
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        || value.len() > 200_000
    {
        return false;
    }
    match name {
        "href" | "xlink:href" => {
            return (element == "use" && fragment(value))
                || (element == "image" && raster_data(value));
        }
        "xmlns" => return value == SVG_NS,
        "xmlns:xlink" => return value == XLINK_NS,
        _ => {}
    }
    let lower = value.to_ascii_lowercase();
    if [
        "javascript",
        "vbscript",
        "data:",
        "http",
        "//",
        "\\",
        "expression",
        "@import",
    ]
    .iter()
    .any(|b| lower.contains(b))
    {
        return false;
    }
    // `url()` only as a reference inside the drawing: url(#id).
    let mut rest = lower.as_str();
    while let Some(at) = rest.find("url(") {
        let after = &rest[at + 4..];
        let Some(end) = after.find(')') else {
            return false;
        };
        if !fragment(after[..end].trim().trim_matches(['"', '\''])) {
            return false;
        }
        rest = &after[end + 1..];
    }
    true
}

fn open_tag(out: &mut String, element: &BytesStart<'_>, name: &str) -> Result<(), ()> {
    let _ = write!(out, "<{name}");
    for attribute in element.attributes() {
        let attribute: Attribute<'_> = attribute.map_err(|_| ())?;
        let key = attribute.key.as_ref().to_owned();
        if !ATTRIBUTES.contains(&key.as_str()) {
            continue;
        }
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|_| ())?;
        if keep_value(name, &key, &value) {
            let _ = write!(out, " {key}=\"{}\"", escape(&value));
        }
    }
    Ok(())
}

/// The allowlisted re-serialisation of `markup`, or `None` when it is not a plain SVG document (see the module).
#[must_use]
pub fn sanitize(markup: &str) -> Option<String> {
    let mut reader = Reader::from_str(markup);
    reader.config_mut().check_end_names = true;
    let mut out = String::with_capacity(markup.len().min(1 << 20));
    // Open elements we keep (to close them) and the depth of a dropped subtree (0: not inside one).
    let mut open: Vec<String> = Vec::new();
    let mut skip = 0usize;
    let mut root_seen = false;
    loop {
        match reader.read_event().ok()? {
            Event::Eof => break,
            Event::DocType(_) | Event::PI(_) => return None,
            // Inside a dropped subtree: only its depth is followed.
            Event::Start(_) if skip > 0 => skip += 1,
            Event::Empty(_) if skip > 0 => {}
            Event::Start(e) => {
                let name = e.name().as_ref().to_owned();
                if !root_seen && name != "svg" {
                    return None;
                }
                root_seen = true;
                if ELEMENTS.contains(&name.as_str()) {
                    open_tag(&mut out, &e, &name).ok()?;
                    out.push('>');
                    open.push(name);
                } else {
                    skip = 1;
                }
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_owned();
                if !root_seen && name != "svg" {
                    return None;
                }
                root_seen = true;
                if ELEMENTS.contains(&name.as_str()) {
                    open_tag(&mut out, &e, &name).ok()?;
                    out.push_str("/>");
                }
            }
            Event::End(_) if skip > 0 => skip -= 1,
            Event::End(_) => {
                let name = open.pop()?;
                let _ = write!(out, "</{name}>");
            }
            Event::Text(t) if skip == 0 && !open.is_empty() => {
                out.push_str(&escape(&t.xml_content(quick_xml::XmlVersion::Implicit1_0)));
            }
            Event::GeneralRef(r) if skip == 0 && !open.is_empty() => {
                let ch = match r.resolve_char_ref().ok()? {
                    Some(c) => c,
                    None => match r.as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return None,
                    },
                };
                out.push_str(&escape(&ch.to_string()));
            }
            // The XML declaration, comments, CDATA, and text outside kept elements or inside a dropped subtree.
            Event::Decl(_)
            | Event::Comment(_)
            | Event::Text(_)
            | Event::CData(_)
            | Event::GeneralRef(_) => {}
        }
    }
    (root_seen && open.is_empty() && skip == 0).then_some(out)
}
