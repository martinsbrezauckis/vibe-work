//! Quick edits in the File tab for files the agent touched.
//!
//! `write {sessionId, file, text, fingerprint}` saves the whole text of a file the tab shows. Rules:
//! - the same handle and scope check as `read` (the session's folder or a folder the user picked; a symlink that
//!   leaves it is refused);
//! - only a text file the tab showed whole: valid UTF-8, no NUL, at most [`TEXT_LIMIT`] before and after;
//! - **conflict check:** `fingerprint` is what the tab read; when the file's fingerprint differs now (the agent or
//!   another program changed it), nothing is written and the answer is `stale`;
//! - the file keeps its line endings (a text area gives `\n`; a file that used `\r\n` gets them back) and its UTF-8
//!   byte-order mark; it is written whole, keeping its permissions, through `save_file::replace_file` (a new temporary
//!   file under a random name, never through a link planted at a predictable name);
//! - hostile folders (a cloned repository): a file or folder that is a link leading out of the session's folders is
//!   refused by the handle's scope check (canonical path inside a canonical root), and checked again right
//!   before the write (the canonical path must still be the same).
//!
//! The edit is an ordinary change of the working tree, so the Changes tab shows it and reverts it per hunk (the
//! normal undo path); telling the agent is the window's part (a context chip with the diff, sent with the
//! next message).
//!
//! Depends on: `files` (`checked`), `files_read` (fingerprint, limit). Called by: `files` (`FilesRequest::Write`).
//! Invariants: nothing is written unless every rule passed; a failed write leaves the old file and no temporary file.

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::files_read::{TEXT_LIMIT, fingerprint};
use crate::pages::blocking;

/// UTF-8 byte-order mark.
const BOM: &[u8] = b"\xEF\xBB\xBF";

fn refuse(why: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, why.to_owned())
}

/// Line feeds in `bytes`.
fn line_feeds(bytes: &[u8]) -> usize {
    bytes.split(|b| *b == b'\n').count().saturating_sub(1)
}

/// `text` as the file stores it: `\r\n` when the old bytes used them (most line ends), the BOM when they had one.
fn as_stored(old: &[u8], text: &str) -> Vec<u8> {
    let crlf = old.windows(2).filter(|w| w == b"\r\n").count();
    let lf = line_feeds(old);
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let body = if crlf > 0 && crlf * 2 >= lf && !body.contains('\r') {
        body.replace('\n', "\r\n")
    } else {
        body.to_owned()
    };
    let mut out = Vec::with_capacity(body.len() + 3);
    if old.starts_with(BOM) {
        out.extend_from_slice(BOM);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

impl Core {
    /// `write` (see the module header). Returns the new fingerprint and size.
    pub(crate) async fn files_write(
        &self,
        session_id: &str,
        file: &str,
        text: String,
        expected: &str,
    ) -> Result<(String, u64), CoreError> {
        if text.len() as u64 > TEXT_LIMIT {
            return Err(refuse("the text is too long to save here (over 256 KB)"));
        }
        if text.contains('\0') {
            return Err(refuse("the text holds a NUL character"));
        }
        let hit = self.checked(session_id, file).await?;
        let path = hit.canonical.clone();
        let expected = expected.to_owned();
        blocking(move || {
            let meta = std::fs::metadata(&path).map_err(|_| refuse("the file is not there any more"))?;
            let stale = || {
                CoreError::new(
                    ErrorCode::Stale,
                    "the file changed on disk since it was opened; reload it, then make the edit again",
                )
            };
            if meta.len() > TEXT_LIMIT {
                // Too large to edit here; a file that grew past the limit since it was opened changed, too.
                return Err(if fingerprint(meta.len(), meta.modified().ok(), None) == expected {
                    refuse("only files shown whole (up to 256 KB) can be edited here")
                } else {
                    stale()
                });
            }
            let old = std::fs::read(&path).map_err(|e| {
                CoreError::new(ErrorCode::Internal, format!("the file could not be read: {e}"))
            })?;
            // The content is part of the check (see `files_read::fingerprint`): a rewrite that kept the size and the
            // modification time is still refused, so an older draft never replaces the newer text.
            if fingerprint(old.len() as u64, meta.modified().ok(), Some(&old)) != expected {
                return Err(stale());
            }
            if old.contains(&0) || std::str::from_utf8(&old).is_err() {
                return Err(refuse("only text files can be edited here"));
            }
            let bytes = as_stored(&old, &text);
            // The scope check resolved `path` (canonical, inside the session's folders). Check again right before the
            // write that no part of it became a link since (a folder swapped for a link to elsewhere).
            if std::fs::canonicalize(&path).ok().as_deref() != Some(path.as_path()) {
                return Err(refuse("the file's folder changed since it was opened; reload it"));
            }
            crate::save_file::replace_file(&path, &bytes, Some(&meta.permissions())).map_err(|e| {
                CoreError::new(ErrorCode::Internal, format!("the file could not be saved: {e}"))
            })?;
            let now = std::fs::metadata(&path).map_err(|e| {
                CoreError::new(ErrorCode::Internal, format!("the saved file could not be read: {e}"))
            })?;
            // The same rule as a read: the content hash only for a file small enough to be read whole.
            let whole = (bytes.len() as u64 <= TEXT_LIMIT).then_some(bytes.as_slice());
            Ok((fingerprint(now.len(), now.modified().ok(), whole), now.len()))
        })
        .await?
    }
}
