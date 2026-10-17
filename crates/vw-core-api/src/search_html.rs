//! Bounded visible HTML text for file-search snippets, retaining source lines for navigation.
//! Uses the shared entity decoder on short candidates; `search_files` supplies the search deadline.

const MAX_OUTPUT_BYTES: usize = 1 << 20;
const MAX_OUTPUT_LINES: usize = 16_384;
const ENTITY_LOOKAHEAD: usize = 12;

/// A prefix of the visible source lines; budgets stop conversion and mark it truncated.
pub(crate) struct Visible {
    pub lines: Vec<String>,
    pub truncated: bool,
    #[cfg(test)]
    work: usize,
}

/// Removes markup with linear work, bounded output and regular cancellation checks.
pub(crate) fn visible_lines(source: &str, cancelled: impl FnMut() -> bool) -> Visible {
    let mut work = Work {
        used: 0,
        limit: source.len().saturating_mul(64).saturating_add(1024),
        next_check: 0,
        cancelled,
    };
    let mut output = Output {
        lines: vec![String::new()],
        bytes: 0,
        space: false,
    };
    let truncated = convert(source, &mut output, &mut work).is_err();
    Visible {
        lines: output.lines,
        truncated,
        #[cfg(test)]
        work: work.used,
    }
}

// Charge scans as well as output work, so a rescanned suffix cannot hide behind an iteration count.
struct Work<F> {
    used: usize,
    limit: usize,
    next_check: usize,
    cancelled: F,
}

impl<F: FnMut() -> bool> Work<F> {
    fn spend(&mut self, amount: usize) -> Result<(), ()> {
        if amount > self.limit.saturating_sub(self.used) {
            return Err(());
        }
        self.used += amount;
        if self.used >= self.next_check {
            self.next_check = self.used.saturating_add(1024);
            if (self.cancelled)() {
                return Err(());
            }
        }
        Ok(())
    }
}

// Collapse whitespace while appending: no intermediate word or per-line pointer vectors.
struct Output {
    lines: Vec<String>,
    bytes: usize,
    space: bool,
}

impl Output {
    fn newline(&mut self) -> Result<(), ()> {
        if self.lines.len() >= MAX_OUTPUT_LINES {
            return Err(());
        }
        self.lines.push(String::new());
        self.space = false;
        Ok(())
    }

    fn push(&mut self, c: char) -> Result<(), ()> {
        let Some(line) = self.lines.last_mut() else {
            return Err(());
        };
        if c.is_whitespace() {
            self.space = !line.is_empty();
            return Ok(());
        }
        let bytes = c.len_utf8() + usize::from(self.space);
        if self.bytes + bytes > MAX_OUTPUT_BYTES {
            return Err(());
        }
        if self.space {
            line.push(' ');
        }
        line.push(c);
        self.bytes += bytes;
        self.space = false;
        Ok(())
    }
}

fn convert(
    source: &str,
    output: &mut Output,
    work: &mut Work<impl FnMut() -> bool>,
) -> Result<(), ()> {
    work.spend(0)?;
    let mut at = 0;
    let mut hidden: Option<&[u8]> = None;
    let mut head = false;
    while at < source.len() {
        // Includes the fixed-length prefix comparisons below.
        work.spend(16)?;
        let rest = &source[at..];
        if let Some(close) = hidden {
            if !rest
                .as_bytes()
                .get(..close.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(close))
            {
                let Some(c) = rest.chars().next() else {
                    break;
                };
                if c == '\n' {
                    output.newline()?;
                }
                at += c.len_utf8();
                continue;
            }
            hidden = None;
        }
        if rest.starts_with("<!--") {
            let end = comment_end(rest, work)?;
            advance(&rest[..end], output, work)?;
            at += end;
            continue;
        }
        if rest.starts_with('<') {
            let Some(end) = tag_end(rest, work)? else {
                // An incomplete tag owns the remaining suffix; never retry it at the next '<'.
                advance(rest, output, work)?;
                break;
            };
            // Trimming and extracting the name touch only this already-consumed tag.
            work.spend(4 * (end + 1))?;
            let inside = rest[1..end].trim();
            let closing = inside.starts_with('/');
            let name = inside
                .trim_start_matches('/')
                .split(|c: char| !c.is_ascii_alphanumeric())
                .next()
                .unwrap_or("");
            if name.eq_ignore_ascii_case("head") {
                head = !closing;
            } else if !closing && name.eq_ignore_ascii_case("script") {
                hidden = Some(b"</script");
            } else if !closing && name.eq_ignore_ascii_case("style") {
                hidden = Some(b"</style");
            }
            if !head
                && [
                    "p", "div", "br", "li", "h1", "h2", "h3", "section", "td", "tr",
                ]
                .iter()
                .any(|tag| name.eq_ignore_ascii_case(tag))
            {
                output.push(' ')?;
            }
            advance(&rest[..=end], output, work)?;
            at += end + 1;
            continue;
        }
        if !head
            && rest.starts_with('&')
            && let Some((end, decoded)) = entity(rest, work)?
        {
            for c in decoded.chars() {
                output.push(c)?;
            }
            at += end;
            continue;
        }
        let Some(c) = rest.chars().next() else {
            break;
        };
        if c == '\n' {
            output.newline()?;
        } else if !head {
            output.push(c)?;
        }
        at += c.len_utf8();
    }
    Ok(())
}

fn advance(
    text: &str,
    output: &mut Output,
    work: &mut Work<impl FnMut() -> bool>,
) -> Result<(), ()> {
    for c in text.chars() {
        work.spend(1)?;
        if c == '\n' {
            output.newline()?;
        }
    }
    Ok(())
}

fn comment_end(text: &str, work: &mut Work<impl FnMut() -> bool>) -> Result<usize, ()> {
    for (at, bytes) in text.as_bytes().windows(3).enumerate().skip(4) {
        work.spend(3)?;
        if bytes == b"-->" {
            return Ok(at + 3);
        }
    }
    Ok(text.len())
}

/// A quoted greater-than sign belongs to an attribute rather than ending the tag.
fn tag_end(text: &str, work: &mut Work<impl FnMut() -> bool>) -> Result<Option<usize>, ()> {
    let mut quote = None;
    for (at, c) in text.char_indices().skip(1) {
        work.spend(1)?;
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, '>') => return Ok(Some(at)),
            _ => {}
        }
    }
    Ok(None)
}

fn entity(
    text: &str,
    work: &mut Work<impl FnMut() -> bool>,
) -> Result<Option<(usize, String)>, ()> {
    // The shared decoder accepts semicolons at byte offsets up to 12. Limit the scan before decoding.
    for (at, &byte) in text
        .as_bytes()
        .iter()
        .take(ENTITY_LOOKAHEAD + 1)
        .enumerate()
        .skip(1)
    {
        work.spend(1)?;
        if byte == b'&' {
            break;
        }
        if byte == b';' {
            let candidate = &text[..=at];
            work.spend(4 * candidate.len())?;
            let decoded = vw_pages::decode_entities(candidate);
            return Ok((decoded != candidate).then_some((at + 1, decoded)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MIB: usize = 1 << 20;

    #[test]
    fn adversarial_tags_have_linear_work() {
        let result = visible_lines(&"<".repeat(MIB), || false);
        assert!(
            !result.truncated,
            "tag parsing exceeded the linear work budget"
        );
        assert!(result.work <= 64 * MIB + 1024);
        assert_eq!(result.lines, [""]);
        eprintln!("1 MiB tags: {} work units", result.work);
    }

    #[test]
    fn adversarial_entities_have_linear_work() {
        let source = "&".repeat(MIB);
        let result = visible_lines(&source, || false);
        assert!(
            !result.truncated,
            "entity parsing exceeded the linear work budget"
        );
        assert!(result.work <= 64 * MIB + 1024);
        assert_eq!(result.lines, [source]);
        eprintln!("1 MiB entities: {} work units", result.work);
    }

    #[test]
    fn adversarial_newlines_have_bounded_output_and_work() {
        let result = visible_lines(&"\n".repeat(MIB), || false);
        let retained = result.lines.len() * size_of::<String>()
            + result.lines.iter().map(String::capacity).sum::<usize>();
        assert!(
            retained <= MIB,
            "newline output exceeded 1 MiB: {retained} bytes"
        );
        assert!(result.truncated);
        assert!(result.work <= 64 * MIB + 1024);
        assert_eq!(result.lines.len(), MAX_OUTPUT_LINES);
        eprintln!(
            "1 MiB newlines: {} work units, {retained} retained bytes",
            result.work
        );
    }

    #[test]
    fn cancellation_interrupts_each_scan() {
        for prefix in ["<", "<!--", "<script>", "<style>", "&", "\n", "text"] {
            let source = format!("{prefix}{}", "x".repeat(MIB));
            let mut checks = 0;
            let result = visible_lines(&source, || {
                checks += 1;
                checks == 3
            });
            assert!(result.truncated, "{prefix:?}");
            assert_eq!(checks, 3, "{prefix:?}");
            assert!(result.work <= 2 * 1024 + 64, "{prefix:?}: {}", result.work);
        }
    }

    #[test]
    fn markup_entities_and_unicode_keep_source_lines() {
        let source = "<HEAD>hidden\n</HEAD><div title='>é'> café <b>au</b> lait &amp; &#x1F600; &nbsp; &#10; fin &bogus; &amp &amp;</div>\n<!-- hidden\n--><SCRIPT>if (a < b) {}\n</SCRIPT><STYLE>hidden\n</STYLE><p>last</p>";
        let result = visible_lines(source, || false);
        assert!(!result.truncated);
        assert_eq!(
            result.lines,
            [
                "",
                "café au lait & 😀 fin &bogus; &amp &",
                "",
                "",
                "",
                "last"
            ]
        );
    }

    #[test]
    fn output_bytes_are_capped_without_splitting_unicode() {
        let result = visible_lines(&"é".repeat(MIB), || false);
        assert!(result.truncated);
        assert_eq!(result.lines[0].len(), MAX_OUTPUT_BYTES);
    }
}
