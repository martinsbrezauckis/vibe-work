//! Link resolution inside a handbook folder: where a relative link of a page points (another page or file of the
//! folder, an `#id` on the same page), or that it leaves the folder or uses a scheme.
//!
//! Depends on: `paths`. Called by: `handbook::refs` (the validator's link rules), tests.
//! Invariant: a link is only ever resolved inside the handbook folder (`..` past its top, a leading `/` (the site
//! root, not the handbook) and hidden segments are "outside", never followed); links with a scheme (`http:`,
//! `https:`, `mailto:`, any other) or `//` are external and not resolved.

/// Where a link points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Another scheme (`https://...`, `mailto:`): not checked.
    External,
    /// Only a fragment on the same page (`#steps`).
    SamePage(String),
    /// A file of the pages folder (`/`-separated relative path) and an optional fragment.
    Local {
        /// Relative path inside the pages folder.
        path: String,
        /// The `#id`, if any.
        fragment: Option<String>,
    },
    /// Leaves the pages folder (`../../x`) or is not a valid path.
    Outside,
}

/// True when `href` starts with a URL scheme (`https:`, `mailto:`, `javascript:` ...).
#[must_use]
pub fn has_scheme(href: &str) -> bool {
    href.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Resolves `href` found on the page at `page` (a relative path like `plans/x.html`).
#[must_use]
pub fn resolve(page: &str, href: &str) -> Target {
    let href = href.trim();
    if has_scheme(href) || href.starts_with("//") {
        return Target::External;
    }
    let (rest, fragment) = match href.split_once('#') {
        Some((p, f)) => (p, Some(f.to_owned()).filter(|f| !f.is_empty())),
        None => (href, None),
    };
    let rest = rest.split(['?']).next().unwrap_or("");
    if rest.is_empty() {
        return fragment.map_or(Target::SamePage(String::new()), Target::SamePage);
    }
    let decoded = percent_decode(rest);
    if decoded.starts_with('/') || decoded.starts_with('\\') {
        return Target::Outside;
    }
    let mut parts: Vec<String> = page
        .split('/')
        .rev()
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(str::to_owned)
        .collect();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Target::Outside;
                }
            }
            other => {
                if !crate::paths::segment_allowed(other) {
                    return Target::Outside;
                }
                parts.push(other.to_owned());
            }
        }
    }
    if parts.is_empty() {
        return Target::Local {
            path: crate::paths::INDEX.to_owned(),
            fragment,
        };
    }
    Target::Local {
        path: parts.join("/"),
        fragment,
    }
}

/// Percent-decodes a path (invalid escapes stay as written).
#[must_use]
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b @ b'0'..=b'9' => Some(b - b'0'),
        b @ b'a'..=b'f' => Some(b - b'a' + 10),
        b @ b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
