//! The HTML reader of a handbook page (Handbook Format 1.2 §5, §6): one pass over the tokens collects the head
//! fields, the visible header markers, steps, sections, ids, headings, images, references, style URLs and script
//! hooks into a [`PageScan`]. The types and the invariants are in `page`.
//!
//! Depends on: `html` (tokens), `page` (types), `handbook::format` (step states). Called by: `page` (re-exports),
//! `handbook::files`, `handbook::overview`, tests.

use crate::html::{Tag, Token, clip, squash, tokens};
use crate::page::{
    LegacyHead, MAX_SECTIONS, MAX_STEPS, PageDetail, PageFacts, PageScan, Reference, Relation,
    STEP_STATES, Section, Step,
};

const MAX_LINKS: usize = 2000;
const HEADINGS: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];
const KNOWN_FIELDS: &[&str] = &[
    "handbook.format",
    "handbook.kind",
    "handbook.status",
    "handbook.project",
    "handbook.outcome",
    "handbook.decision-makers",
];

/// Containers whose children mean something (§5 relations, §6.3 steps).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    None,
    /// `nav.hb-related` (or the pre-1.0 `nav.vw-related`).
    RelatedNav,
    /// `ol.hb-steps` (or the pre-1.0 `ol.vw-steps`).
    StepsList,
}

/// An open element that collects text.
pub(crate) struct Open {
    pub(crate) name: String,
    pub(crate) id: Option<String>,
    step: Option<(String, Option<String>)>,
    text: String,
    heading: Option<String>,
    is_heading: bool,
    /// A list the reader treats specially (`nav.hb-related`, `ol.hb-steps`).
    container: Container,
    field: Option<String>,
    datetime: Option<String>,
    /// This element is an input (§6.6): the index of its entry in `facts.inputs`.
    pub(crate) input: Option<usize>,
    /// `data-option` of this element (an option of the enclosing input).
    pub(crate) option: Option<String>,
    /// `class="hb-question"`: the enclosing input's question.
    pub(crate) question: bool,
}

/// Reads an HTML page (facts only).
#[must_use]
pub fn parse_html(source: &str) -> PageFacts {
    scan_html(source).facts
}

/// Reads an HTML page for validation.
#[must_use]
pub fn scan_html(source: &str) -> PageScan {
    let mut scan = PageScan {
        detail: PageDetail {
            truncated: source.len() > crate::html::MAX_INPUT,
            ..PageDetail::default()
        },
        ..PageScan::default()
    };
    let mut stack: Vec<Open> = Vec::new();
    let mut title: Option<String> = None;
    let mut in_title = false;
    for token in tokens(source) {
        match token {
            Token::Text(text) => {
                if in_title {
                    title.get_or_insert_with(String::new).push_str(&text);
                }
                for open in &mut stack {
                    if open.text.len() < 2000 {
                        open.text.push_str(&text);
                        open.text.push(' ');
                    }
                }
            }
            Token::Style(css) => css_urls(&css, &mut scan.detail.style_urls),
            Token::Tag(tag) if tag.closing => {
                if tag.name == "title" {
                    in_title = false;
                }
                // Pop up to and including the matching element (tolerates unclosed children).
                if let Some(pos) = stack.iter().rposition(|o| o.name == tag.name) {
                    while stack.len() > pos {
                        if let Some(open) = stack.pop() {
                            close(open, &mut stack, &mut scan);
                        }
                    }
                }
            }
            Token::Tag(tag) => {
                // The page's title is the FIRST HTML `<title>`; an inline chart's `<svg><title>` (the accessible
                // name of a chart or a bar) is not it. Taking the last `<title>` made every report
                // with a chart fail HB-W09 ("<title> differs from <h1>") and show a bar's tooltip as the page title.
                if tag.name == "title" && title.is_none() && !in_foreign_content(&stack) {
                    in_title = true;
                    title = Some(String::new());
                }
                open_tag(&tag, &stack, &mut scan);
                let input = crate::page_inputs::open_input(&tag, &stack, &mut scan);
                if tag.self_closing {
                    continue;
                }
                let mut open = new_open(tag, &stack);
                open.input = input;
                stack.push(open);
            }
        }
    }
    while let Some(open) = stack.pop() {
        close(open, &mut stack, &mut scan);
    }
    scan.facts.title = title
        .map(|t| clip(&squash(&t), 200))
        .filter(|t| !t.is_empty());
    scan
}

/// True inside `<svg>` or `<math>`: their `<title>` elements belong to the drawing, not to the page.
fn in_foreign_content(stack: &[Open]) -> bool {
    stack.iter().any(|o| o.name == "svg" || o.name == "math")
}

/// Records what a start tag says (meta, links, attributes the validator checks).
fn open_tag(tag: &Tag, stack: &[Open], scan: &mut PageScan) {
    let (facts, detail) = (&mut scan.facts, &mut scan.detail);
    match tag.name.as_str() {
        "meta" => read_meta(tag, facts, detail),
        "img" if tag.attr("alt").is_none() => detail.images_without_alt += 1,
        "script" if tag.attr("src").is_none() => detail.inline_scripts += 1,
        _ => {}
    }
    if let Some(id) = tag.attr("id").map(|v| clip(v.trim(), 100)) {
        detail.ids.push(id.clone());
        if tag.name == "section" && !id.is_empty() {
            detail.section_ids.push(id);
        }
    }
    if let Some(style) = tag.attr("style") {
        css_urls(style, &mut detail.style_urls);
    }
    for (name, _) in &tag.attrs {
        if name.len() > 2 && name.starts_with("on") && detail.event_attributes.len() < 20 {
            detail
                .event_attributes
                .push(format!("{name} on <{}>", tag.name));
        }
    }
    let rel: Vec<String> = tag
        .attr("rel")
        .map(|r| r.split_whitespace().map(str::to_ascii_lowercase).collect())
        .unwrap_or_default();
    for attr in ["href", "src", "poster"] {
        let Some(url) = tag.attr(attr).map(str::trim).filter(|l| !l.is_empty()) else {
            continue;
        };
        if facts.links.len() >= MAX_LINKS {
            break;
        }
        let url = clip(url, 2000);
        facts.links.push(url.clone());
        if tag.name == "a" {
            for r in rel
                .iter()
                .filter(|r| matches!(r.as_str(), "related" | "supersedes" | "superseded-by"))
            {
                facts.relations.push(Relation {
                    rel: r.clone(),
                    href: url.clone(),
                });
            }
            // Pre-1.0 pages list related pages in `nav.vw-related` without `rel`.
            if rel.is_empty() && stack.iter().any(|o| o.container == Container::RelatedNav) {
                facts.relations.push(Relation {
                    rel: "related".to_owned(),
                    href: url.clone(),
                });
            }
        }
        detail.references.push(Reference {
            tag: tag.name.clone(),
            attr,
            url,
            rel: if matches!(tag.name.as_str(), "a" | "link") {
                rel.clone()
            } else {
                Vec::new()
            },
        });
    }
}

fn has_class(tag: &Tag, class: &str) -> bool {
    tag.attr("class")
        .is_some_and(|c| c.split_whitespace().any(|c| c == class))
}

fn new_open(tag: Tag, stack: &[Open]) -> Open {
    let id = tag
        .attr("id")
        .map(|v| clip(v.trim(), 100))
        .filter(|v| !v.is_empty());
    let in_steps_list = stack
        .last()
        .is_some_and(|o| o.container == Container::StepsList);
    let step = match tag.attr("data-step") {
        Some(v) => Some((
            clip(v.trim(), 100),
            tag.attr("data-status").map(|s| clip(s.trim(), 40)),
        )),
        // A list item of `ol.hb-steps` without `data-step` is a step markup problem (HB-E05).
        None if tag.name == "li" && in_steps_list => Some((String::new(), None)),
        None => None,
    };
    Open {
        is_heading: HEADINGS.contains(&tag.name.as_str()),
        container: if tag.name == "nav"
            && (has_class(&tag, "hb-related") || has_class(&tag, "vw-related"))
        {
            Container::RelatedNav
        } else if tag.name == "ol" && (has_class(&tag, "hb-steps") || has_class(&tag, "vw-steps")) {
            Container::StepsList
        } else {
            Container::None
        },
        field: tag
            .attr("data-field")
            .map(|f| f.trim().to_ascii_lowercase()),
        datetime: tag.attr("datetime").map(|d| clip(d.trim(), 40)),
        input: None,
        option: tag.attr("data-option").map(|o| clip(o.trim(), 100)),
        question: has_class(&tag, "hb-question"),
        name: tag.name,
        id,
        step,
        text: String::new(),
        heading: None,
    }
}

/// An element ended: record its section/step/marker and pass its heading text up.
fn close(open: Open, stack: &mut [Open], scan: &mut PageScan) {
    let text = squash(&open.text);
    let (facts, detail) = (&mut scan.facts, &mut scan.detail);
    if open.is_heading {
        let level = open.name.as_bytes()[1] - b'0';
        // Headings close in document order of their END; record them by start order instead: a heading never
        // contains another heading, so end order equals start order.
        detail.headings.push(level);
        if level == 1 && facts.h1.is_none() && !text.is_empty() {
            facts.h1 = Some(clip(&text, 200));
        }
        for parent in stack.iter_mut().rev() {
            if parent.heading.is_none() {
                parent.heading = Some(clip(&text, 120));
            }
        }
    }
    match open.field.as_deref() {
        Some("status") if detail.visible_status.is_none() => {
            detail.visible_status = Some(clip(&text, 120));
        }
        Some("modified") if detail.visible_modified.is_none() => {
            detail.visible_modified =
                Some(open.datetime.clone().unwrap_or_else(|| clip(&text, 40)));
        }
        _ => {}
    }
    crate::page_inputs::close_input_part(&open, &text, stack, facts, detail);
    if let Some((id, raw)) = open.step {
        record_step(id, raw, open.id.as_deref(), &text, facts, detail);
    }
    if let Some(id) = open.id {
        if facts.sections.len() >= MAX_SECTIONS {
            detail.over_limits = true;
        } else if !facts.sections.iter().any(|s| s.id == id) {
            let label = open
                .heading
                .filter(|h| !h.is_empty() && !open.is_heading)
                .unwrap_or_else(|| clip(&text, 120));
            facts.sections.push(Section { id, label });
        }
    }
}

fn record_step(
    id: String,
    raw: Option<String>,
    element_id: Option<&str>,
    text: &str,
    facts: &mut PageFacts,
    detail: &mut PageDetail,
) {
    if id.is_empty() {
        detail
            .step_problems
            .push(format!("a step has no data-step: \"{}\"", clip(text, 60)));
        return;
    }
    if facts.steps.iter().any(|s| s.id == id) {
        detail
            .step_problems
            .push(format!("step id \"{id}\" is used twice"));
        return;
    }
    if facts.steps.len() >= MAX_STEPS {
        detail.over_limits = true;
        return;
    }
    match raw.as_deref() {
        Some(s) if STEP_STATES.contains(&s) => {}
        Some(s) => detail.step_problems.push(format!(
            "step {id} has data-status \"{s}\" (todo, doing, done, blocked, skipped)"
        )),
        None => detail
            .step_problems
            .push(format!("step {id} has no data-status")),
    }
    // A step is also a section a comment can cite (by its step id when it has no element id).
    if element_id.is_none() && !facts.sections.iter().any(|s| s.id == id) {
        facts.sections.push(Section {
            id: id.clone(),
            label: clip(text, 120),
        });
    }
    let status = normalize_step(raw.as_deref().unwrap_or(""));
    detail.step_raw.push(raw);
    facts.steps.push(Step {
        id,
        status,
        text: clip(text, 300),
    });
}

fn read_meta(tag: &Tag, facts: &mut PageFacts, detail: &mut PageDetail) {
    let (Some(name), Some(content)) = (tag.attr("name"), tag.attr("content")) else {
        return;
    };
    let value = clip(&squash(content), 300);
    let name = name.trim().to_ascii_lowercase();
    if value.is_empty() {
        return;
    }
    match name.as_str() {
        "handbook.format" => facts.format = Some(value),
        "handbook.kind" => facts.kind = Some(value.to_ascii_lowercase()),
        "handbook.status" => facts.status = Some(value),
        "handbook.project" => facts.project = Some(value),
        "handbook.outcome" => facts.outcome = Some(value),
        "dcterms.created" => facts.created = Some(value),
        "dcterms.modified" => facts.modified = Some(value),
        "description" => facts.description = Some(value),
        "author" => facts.authors.push(value),
        "vw-kind" => {
            facts.legacy.get_or_insert_with(LegacyHead::default).kind =
                Some(value.to_ascii_lowercase());
        }
        "vw-status" => facts.legacy.get_or_insert_with(LegacyHead::default).status = Some(value),
        "vw-date" => facts.legacy.get_or_insert_with(LegacyHead::default).date = Some(value),
        "vw-generated" => facts.generated = value.eq_ignore_ascii_case("hub"),
        other if other.starts_with("handbook.") && !KNOWN_FIELDS.contains(&other) => {
            detail.unknown_fields.push(clip(other, 60));
        }
        _ => {}
    }
}

/// Collects the targets of `@import` and `url()` in CSS text (at most 50).
fn css_urls(css: &str, out: &mut Vec<String>) {
    let lower = css.to_ascii_lowercase();
    let mut push = |raw: &str| {
        let url = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        if !url.is_empty() && out.len() < 50 {
            out.push(clip(url, 300));
        }
    };
    let mut from = 0;
    while let Some(pos) = lower[from..].find("url(") {
        let start = from + pos + 4;
        let end = lower[start..].find(')').map_or(lower.len(), |e| start + e);
        push(&css[start..end]);
        from = end.min(lower.len());
    }
    let mut from = 0;
    while let Some(pos) = lower[from..].find("@import") {
        let start = from + pos + 7;
        let rest = css[start..].trim_start();
        // `@import url(...)` is already counted by the url() pass.
        if !rest.to_ascii_lowercase().starts_with("url(") {
            let end = rest.find([';', ' ', '\n']).unwrap_or(rest.len());
            push(&rest[..end]);
        }
        from = start;
    }
}

/// Maps a `data-status` value (and the todo tool's words) to the step states; unknown → `todo`.
#[must_use]
pub fn normalize_step(status: &str) -> String {
    let lower = status.trim().to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "done" | "completed" | "complete" | "finished" => "done",
        "doing" | "in_progress" | "in-progress" | "active" | "working" => "doing",
        "blocked" => "blocked",
        "skipped" | "cancelled" | "canceled" | "dropped" => "skipped",
        _ => "todo",
    };
    mapped.to_owned()
}
