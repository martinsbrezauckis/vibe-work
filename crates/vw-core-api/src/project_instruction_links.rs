//! Markdown link detection and draft edits for instruction files.
//! Depends on pulldown-cmark; called by the list and `LinkDraft` handler.
//! Parsed link events skip code and recognize inline, titled, angle-destination and reference links.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::ops::Range;

fn links_to(text: &str, name: &str) -> Vec<Range<usize>> {
    let mut open: Vec<(usize, bool)> = Vec::new();
    let mut matches = Vec::new();
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                let destination = dest_url.strip_prefix("./").unwrap_or(&dest_url);
                open.push((range.start, destination == name));
            }
            Event::End(TagEnd::Link) => {
                if let Some((start, true)) = open.pop() {
                    matches.push(start..range.end);
                }
            }
            _ => {}
        }
    }
    matches
}

fn whole_link_line(text: &str, span: Range<usize>) -> Range<usize> {
    let start = text[..span.start].rfind('\n').map_or(0, |n| n + 1);
    let end = text[span.end..]
        .find('\n')
        .map_or(text.len(), |n| span.end + n);
    if text[start..span.start].trim().is_empty() && text[span.end..end].trim().is_empty() {
        start..if end < text.len() { end + 1 } else { end }
    } else {
        span
    }
}

/// Whether AGENTS.md contains an actual Markdown link to the relative file.
pub(crate) fn linked_from_agents(text: &str, name: &str) -> bool {
    !links_to(text, name).is_empty()
}

/// Removes matching Markdown hyperlinks from a draft while retaining the rest of each line.
pub(crate) fn unlink_draft(text: &str, name: &str) -> String {
    let spans = links_to(text, name);
    if spans.is_empty() {
        return text.to_owned();
    }
    let mut draft = String::with_capacity(text.len());
    let mut from = 0;
    for span in spans.into_iter().map(|span| whole_link_line(text, span)) {
        if span.start < from {
            continue;
        }
        draft.push_str(&text[from..span.start]);
        from = span.end;
    }
    draft.push_str(&text[from..]);
    draft
}
