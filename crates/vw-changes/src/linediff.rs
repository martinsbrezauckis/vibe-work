//! Line diff of two byte buffers in Rust (git never reads working-tree content here, and no
//! repository filter or diff driver runs): hunks with three lines of context, and the splice
//! that applies one hunk forward (keep: into the baseline) or backward (revert: into the working file).
//!
//! Depends on: `similar` 3.2.0 (Apache-2.0; Myers diff with a deadline), `model`. Called by: `diff`,
//! `review`.
//! Invariants:
//! - Lines keep their terminators (`\n`, `\r\n`, none at the end), so a splice writes back exactly the
//!   bytes it took: CRLF files and non-UTF-8 text stay byte-identical outside the hunk.
//! - A diff that would take longer than `DEADLINE` degrades to a coarser (still correct) diff instead of
//!   blocking the refresh.

use std::time::{Duration, Instant};

use similar::{Algorithm, DiffTag};

use crate::model::{DiffLine, LineKind};

/// Lines of context around a change (git's default).
pub(crate) const CONTEXT: usize = 3;
/// Longest a single file's line diff may run.
const DEADLINE: Duration = Duration::from_millis(250);

/// Splits `bytes` into lines, each with its terminator.
pub(crate) fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (at, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&bytes[start..=at]);
            start = at + 1;
        }
    }
    if start < bytes.len() {
        lines.push(&bytes[start..]);
    }
    lines
}

/// True when `bytes` looks binary (a NUL in the first 8000 bytes, git's own test).
pub(crate) fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0)
}

/// One hunk found by [`hunks`]: 0-based line ranges in both versions and its display lines.
pub(crate) struct LineHunk {
    pub(crate) old_at: usize,
    pub(crate) old_len: usize,
    pub(crate) new_at: usize,
    pub(crate) new_len: usize,
    pub(crate) lines: Vec<DiffLine>,
}

fn display(line: &[u8]) -> (String, bool) {
    let no_eol = !line.ends_with(b"\n");
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    (String::from_utf8_lossy(body).into_owned(), no_eol)
}

fn line(kind: LineKind, old_no: Option<usize>, new_no: Option<usize>, raw: &[u8]) -> DiffLine {
    let (text, no_eol) = display(raw);
    let n = |v: Option<usize>| v.map(|v| u32::try_from(v + 1).unwrap_or(u32::MAX));
    DiffLine {
        kind,
        old_no: n(old_no),
        new_no: n(new_no),
        text,
        no_eol,
    }
}

/// The hunks that turn `old` into `new`, with [`CONTEXT`] lines around each change.
pub(crate) fn hunks(old: &[&[u8]], new: &[&[u8]]) -> Vec<LineHunk> {
    let ops = similar::capture_diff_slices_deadline(
        Algorithm::Myers,
        old,
        new,
        Some(Instant::now() + DEADLINE),
    );
    similar::group_diff_ops(ops, CONTEXT)
        .into_iter()
        .filter_map(|group| {
            let (first, last) = (group.first()?, group.last()?);
            let (old_at, new_at) = (first.old_range().start, first.new_range().start);
            let (old_end, new_end) = (last.old_range().end, last.new_range().end);
            let mut lines = Vec::new();
            for op in &group {
                let (tag, o, n) = op.as_tag_tuple();
                match tag {
                    DiffTag::Equal => {
                        for (i, j) in o.clone().zip(n.clone()) {
                            lines.push(line(LineKind::Ctx, Some(i), Some(j), old[i]));
                        }
                    }
                    DiffTag::Delete | DiffTag::Insert | DiffTag::Replace => {
                        for i in o.clone() {
                            lines.push(line(LineKind::Del, Some(i), None, old[i]));
                        }
                        for j in n.clone() {
                            lines.push(line(LineKind::Add, None, Some(j), new[j]));
                        }
                    }
                }
            }
            Some(LineHunk {
                old_at,
                old_len: old_end - old_at,
                new_at,
                new_len: new_end - new_at,
                lines,
            })
        })
        .collect()
}

/// `target` with lines `at..at + len` replaced by `replacement` (exact bytes).
pub(crate) fn splice(target: &[&[u8]], at: usize, len: usize, replacement: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in target.iter().take(at) {
        out.extend_from_slice(part);
    }
    for part in replacement {
        out.extend_from_slice(part);
    }
    for part in target.iter().skip(at + len) {
        out.extend_from_slice(part);
    }
    out
}
