//! Content anchors for line comments on the diff: where a comment sits, found again after the diff
//! was refreshed, edited above, kept or reverted.
//!
//! An anchor is the commented lines' `kind` + text hashes in order, the hash of the line before and the line
//! after (inside the hunk; `None` at a hunk edge), and a line-number hint (the baseline line when the first
//! line has one, else the current line). [`locate`] searches a file's current hunks for the same run of lines,
//! so numbers that shifted because of edits above do not matter; a run that no longer occurs (the line was
//! edited, the hunk kept or reverted, the file no longer changed) is `None`: the comment is **outdated**, never
//! dropped. Ties (the same `}` twice) are broken by matching neighbours, then by distance to the hint.
//!
//! Depends on: `model` (hunks, lines), `parse::fnv_hex`. Called by: `line_comments` (store), vw-core-api.
//! Invariants: pure functions over the core's own diff, no I/O; the quoted lines come from the diff, never from
//! a client (the caller passes a hunk it just read).

use serde::{Deserialize, Serialize};

use crate::model::{DiffLine, FileChange, Hunk, LineKind};
use crate::parse::fnv_hex;

/// Most lines one comment can quote.
pub const MAX_LINES: usize = 40;

/// Why an anchor could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnchorError {
    /// The line range is empty, reversed, outside the hunk or longer than [`MAX_LINES`].
    #[error("{0}")]
    Range(String),
}

/// Where a comment sits, independent of line numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LineAnchor {
    /// `kind` + text hash of each commented line, in order.
    pub lines: Vec<String>,
    /// Hash of the line before the first one in its hunk (`None`: it was the hunk's first line).
    pub before: Option<String>,
    /// Hash of the line after the last one in its hunk (`None`: it was the hunk's last line).
    pub after: Option<String>,
    /// The first line's baseline number, else its current number (0: unknown). Only breaks ties.
    pub hint: u32,
}

/// Where an anchor was found now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Located {
    /// The hunk (its id in the current diff).
    pub hunk_id: String,
    /// Index of the first commented line in the hunk's `lines`.
    pub from: u32,
    /// Index of the last commented line (inclusive).
    pub to: u32,
}

/// Hash of one diff line: its kind and its text (not its numbers).
#[must_use]
pub fn line_hash(line: &DiffLine) -> String {
    let kind: &[u8] = match line.kind {
        LineKind::Ctx => b"c",
        LineKind::Add => b"a",
        LineKind::Del => b"d",
    };
    fnv_hex(&[kind, line.text.as_bytes()])
}

fn number(line: &DiffLine) -> u32 {
    line.old_no.or(line.new_no).unwrap_or(0)
}

/// The anchor and the quoted lines for lines `from..=to` (indexes into `hunk.lines`).
///
/// # Errors
/// [`AnchorError::Range`] for an empty or reversed range, one past the hunk, or longer than [`MAX_LINES`].
pub fn anchor_for(
    hunk: &Hunk,
    from: usize,
    to: usize,
) -> Result<(Vec<DiffLine>, LineAnchor), AnchorError> {
    if from > to || to >= hunk.lines.len() {
        return Err(AnchorError::Range(
            "the lines are not inside the hunk".to_owned(),
        ));
    }
    if to - from + 1 > MAX_LINES {
        return Err(AnchorError::Range(format!(
            "a comment can quote at most {MAX_LINES} lines"
        )));
    }
    let quoted = hunk.lines[from..=to].to_vec();
    let anchor = LineAnchor {
        lines: quoted.iter().map(line_hash).collect(),
        before: from.checked_sub(1).map(|i| line_hash(&hunk.lines[i])),
        after: hunk.lines.get(to + 1).map(line_hash),
        hint: number(&quoted[0]),
    };
    Ok((quoted, anchor))
}

/// Finds the commented run in `file`'s current hunks, or `None` when it is gone.
#[must_use]
pub fn locate(file: &FileChange, anchor: &LineAnchor) -> Option<Located> {
    let wanted = anchor.lines.len();
    if wanted == 0 {
        return None;
    }
    // (neighbour score, distance to the hint, hunk position, start index): the best has the highest score,
    // then the smallest distance, then the earliest place.
    let mut best: Option<((u8, u32), &Hunk, usize)> = None;
    for hunk in &file.hunks {
        if hunk.lines.len() < wanted {
            continue;
        }
        let hashes: Vec<String> = hunk.lines.iter().map(line_hash).collect();
        for start in 0..=(hunk.lines.len() - wanted) {
            if hashes[start..start + wanted] != anchor.lines[..] {
                continue;
            }
            let before = start.checked_sub(1).map(|i| &hashes[i]);
            let after = hashes.get(start + wanted);
            let score = u8::from(before == anchor.before.as_ref()) * 2
                + u8::from(after == anchor.after.as_ref());
            let distance = number(&hunk.lines[start]).abs_diff(anchor.hint);
            let key = (score, u32::MAX - distance);
            if best.as_ref().is_none_or(|(k, _, _)| key > *k) {
                best = Some((key, hunk, start));
            }
        }
    }
    best.map(|(_, hunk, start)| Located {
        hunk_id: hunk.id.clone(),
        from: u32::try_from(start).unwrap_or(u32::MAX),
        to: u32::try_from(start + wanted - 1).unwrap_or(u32::MAX),
    })
}
