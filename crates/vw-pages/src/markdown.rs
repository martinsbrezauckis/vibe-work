//! Markdown (lite) pages, Handbook Format 1.2 §7: bounded flat front matter, the outline the app reads (headings with
//! their ids, task items, links) and the HTML the page server sends, in the same frame as an HTML page (kind label,
//! title, status and date line, related links, body).
//!
//! Depends on: pulldown-cmark 0.13, `template`, `html`, `lite` (the page's facts for the frame). Called by: `lite`
//! (outline, front matter), `server` (rendering).
//! Invariants:
//! - Raw HTML in a Markdown page is shown as text, never as markup (`Event::Html`/`InlineHtml` become `Text`).
//! - Link targets with a scheme other than `http`, `https` or `mailto` are replaced by `#` (no `javascript:` or
//!   `data:` URLs); image targets must be relative; the server also sends Markdown pages with `script-src 'none'`.
//! - Front matter is never interpreted as YAML (see [`split_front_matter`]).
//! - Heading ids are slugs of the heading text, made unique with `-2`, `-3`, ...; the outline and the renderer
//!   use the same function, so a comment's section id matches the rendered element.

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::template;

/// What the app reads from a Markdown page body.
#[derive(Debug, Default)]
pub struct Outline {
    /// `(level, id, text)` of every heading.
    pub headings: Vec<(u8, String, String)>,
    /// `(done, text)` of every task list item.
    pub tasks: Vec<(bool, String)>,
    /// Link and image targets.
    pub links: Vec<String>,
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Front matter of a lite page (§7): flat `key: value` lines, read as plain text (no YAML features).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrontMatter {
    /// `(key, value)` in order; keys lowercased, values trimmed and unquoted.
    pub pairs: Vec<(String, String)>,
    /// What could not be read, plain words (lite pages get hints only, HB-I04).
    pub problems: Vec<String>,
}

impl FrontMatter {
    /// The first value of `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Most front matter lines read.
pub const FRONT_MATTER_LINES: usize = 64;
/// Most front matter bytes read.
pub const FRONT_MATTER_BYTES: usize = 8 * 1024;

/// Splits `---` front matter from the body. Bounded and flat on purpose: at most [`FRONT_MATTER_LINES`] lines and
/// [`FRONT_MATTER_BYTES`] bytes, one `key: value` per line; indented lines, anchors, aliases, tags and flow values
/// are never interpreted (a value is its plain text), so no input can expand or nest. Without a closing `---` in
/// those bounds the file has no front matter (a problem is reported).
#[must_use]
pub fn split_front_matter(source: &str) -> (FrontMatter, &str) {
    let text = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut front = FrontMatter::default();
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (front, text);
    };
    let mut offset = 0;
    for (n, line) in rest.split_inclusive('\n').enumerate() {
        if n >= FRONT_MATTER_LINES || offset + line.len() > FRONT_MATTER_BYTES {
            break;
        }
        offset += line.len();
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.trim_end() == "---" {
            return (front, &rest[offset..]);
        }
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        if trimmed.starts_with([' ', '\t']) {
            front.problems.push(format!(
                "front matter line {} is indented; nested values are not read",
                n + 2
            ));
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            front
                .problems
                .push(format!("front matter line {} is not \"key: value\"", n + 2));
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        if !key.is_empty() {
            front
                .pairs
                .push((crate::html::clip(&key, 60), crate::html::clip(value, 300)));
        }
    }
    let problems = vec![format!(
        "front matter has no closing --- within {FRONT_MATTER_LINES} lines; it is read as text"
    )];
    (
        FrontMatter {
            pairs: Vec::new(),
            problems,
        },
        text,
    )
}

/// A heading's slug: lowercase letters and digits, other runs become `-`; empty → `section`.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(80).collect();
    if trimmed.is_empty() {
        "section".to_owned()
    } else {
        trimmed
    }
}

/// Unique ids for heading texts in order.
fn unique_ids(texts: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    texts
        .iter()
        .map(|t| {
            let base = slug(t);
            let n = seen.entry(base.clone()).or_insert(0);
            *n += 1;
            if *n == 1 { base } else { format!("{base}-{n}") }
        })
        .collect()
}

fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// The text of every heading, in order (first pass of both the outline and the renderer).
fn heading_texts(events: &[Event<'_>]) -> Vec<String> {
    let mut texts = Vec::new();
    let mut current: Option<String> = None;
    for event in events {
        match event {
            Event::Start(Tag::Heading { .. }) => current = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => texts.extend(current.take()),
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = current.as_mut() {
                    c.push_str(t);
                }
            }
            _ => {}
        }
    }
    texts
}

/// True for link targets the renderer keeps (relative ones and `http`, `https`, `mailto`).
fn safe_target(target: &str) -> bool {
    match target.split_once(':') {
        Some((scheme, _))
            if !scheme.is_empty()
                && !scheme.contains(['/', '?', '#'])
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "mailto"
            )
        }
        _ => true,
    }
}

/// The outline of a Markdown body.
#[must_use]
pub fn outline(body: &str) -> Outline {
    let events: Vec<Event<'_>> = Parser::new_ext(body, options()).collect();
    let texts = heading_texts(&events);
    let ids = unique_ids(&texts);
    let mut out = Outline::default();
    let mut heading_index = 0;
    let mut task: Option<(bool, String)> = None;
    for event in &events {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if let (Some(id), Some(text)) = (ids.get(heading_index), texts.get(heading_index)) {
                    out.headings
                        .push((level_number(*level), id.clone(), text.clone()));
                }
                heading_index += 1;
            }
            Event::TaskListMarker(done) => task = Some((*done, String::new())),
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, text)) = task.as_mut() {
                    text.push_str(t);
                }
            }
            Event::End(TagEnd::Item) => out.tasks.extend(task.take()),
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                out.links.push(dest_url.to_string());
            }
            _ => {}
        }
    }
    out
}

/// One body event made safe: raw HTML as text, heading ids from `ids` (the next one at `*next`), link targets with
/// an unsafe scheme and non-relative image targets replaced by `#`.
fn sanitize<'a>(event: Event<'a>, ids: &[String], next: &mut usize) -> Event<'a> {
    match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::Start(Tag::Heading {
            level,
            classes,
            attrs,
            ..
        }) => {
            let id = ids.get(*next).cloned().map(CowStr::from);
            *next += 1;
            Event::Start(Tag::Heading {
                level,
                id,
                classes,
                attrs,
            })
        }
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: if safe_target(&dest_url) {
                dest_url
            } else {
                CowStr::from("#")
            },
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: if relative_target(&dest_url) {
                dest_url
            } else {
                CowStr::from("#")
            },
            title,
            id,
        }),
        other => other,
    }
}

/// True for image targets the renderer keeps: relative paths only (§7 "images load only from relative paths").
fn relative_target(target: &str) -> bool {
    !crate::links::has_scheme(target) && !target.starts_with("//") && !target.starts_with('\\')
}

/// Renders a Markdown page to a whole HTML document in the handbook frame (§7 "Display"): kind label, title,
/// status and date line, related links, then the body. `fallback_title` is used when the page names none;
/// `stylesheet` is the relative link to the handbook's `handbook.css` when it exists (else the built-in style is
/// inlined). See the module invariants.
#[must_use]
pub fn render(source: &str, fallback_title: &str, stylesheet: Option<&str>) -> String {
    render_facts(
        source,
        fallback_title,
        stylesheet,
        &crate::lite::parse_markdown(source),
    )
}

/// Like [`render`] for the page at `path` (relative to the handbook folder): the page's folder can give it a kind
/// (§7), so the frame shows the same kind label as the hub.
#[must_use]
pub fn render_at(
    source: &str,
    fallback_title: &str,
    stylesheet: Option<&str>,
    path: &str,
) -> String {
    render_facts(
        source,
        fallback_title,
        stylesheet,
        &crate::lite::parse_markdown_at(source, path),
    )
}

fn render_facts(
    source: &str,
    fallback_title: &str,
    stylesheet: Option<&str>,
    facts: &crate::page::PageFacts,
) -> String {
    let (_, body) = split_front_matter(source);
    let mut events: Vec<Event<'_>> = Parser::new_ext(body, options()).collect();
    let ids = unique_ids(&heading_texts(&events));
    // A leading `# Title` becomes the frame's title instead of a second h1 under it.
    let mut skip_ids = 0;
    let first_text = heading_texts(&events).into_iter().next();
    if let Some(Event::Start(Tag::Heading {
        level: HeadingLevel::H1,
        ..
    })) = events.first()
        && first_text.is_some_and(|t| {
            facts.title.as_deref()
                == Some(crate::html::clip(&crate::html::squash(&t), 200).as_str())
        })
        && let Some(end) = events
            .iter()
            .position(|e| matches!(e, Event::End(TagEnd::Heading(_))))
    {
        events.drain(..=end);
        skip_ids = 1;
    }
    let mut heading_index = skip_ids;
    let safe = events
        .into_iter()
        .map(|event| sanitize(event, &ids, &mut heading_index));
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, safe);
    let title = facts
        .title
        .clone()
        .unwrap_or_else(|| fallback_title.to_owned());
    let header = template::header(&template::HeaderParts {
        kind: facts
            .kind
            .as_deref()
            .and_then(crate::handbook::format::kind)
            .map_or("Document", |k| k.label),
        title: &title,
        title_id: (skip_ids == 1)
            .then(|| ids.first().map(String::as_str))
            .flatten(),
        status: facts.status.as_deref(),
        modified: facts.modified.as_deref(),
        related: &facts
            .relations
            .iter()
            .filter(|r| relative_target(&r.href))
            .map(|r| (r.rel.as_str(), r.href.as_str()))
            .collect::<Vec<_>>(),
    });
    template::document(
        &title,
        stylesheet,
        &format!("{header}<main>\n{html}</main>"),
    )
}
