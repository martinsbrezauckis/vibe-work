//! NDJSON framing: one JSON value per line, read with a hard upper bound per frame.
//!
//! Depends on: tokio's buffered async reader. Called by: the connection reader (stdout) and the
//! stderr pump (`stderr_ring`).
//! Invariant: memory per frame never exceeds `max_frame_bytes + 1`, whatever the peer writes; an
//! oversized line is drained to its newline so the stream resynchronizes on the next frame.
//!
//! Extracted from ShellX `src-tauri/src/provider_sessions/stream_reader.rs:31-63 @ e38c2b5c`
//! (`read_provider_bounded_line`, introduced in ShellX `6900bb85` "bound streamed cli records").
//! Changed on the way: ShellX reported a line of exactly `max` bytes plus its newline as overflow
//! (its `take(max)` stops before the newline); here `max` is the largest accepted content length,
//! and the overflow result carries how many bytes were discarded (diagnostics only).

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

/// One read from a bounded NDJSON stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedLine {
    /// A complete line without its trailing `\n` (and without a `\r` before it). At end of stream
    /// a final line without newline is also returned here.
    Line(Vec<u8>),
    /// The line was longer than the limit. It was read and discarded up to and including its
    /// newline (or to end of stream); `discarded` is the number of bytes thrown away.
    Overflow {
        /// Bytes read and dropped for this line, newline included.
        discarded: u64,
    },
    /// End of stream, nothing more to read.
    Eof,
}

/// Reads one line of at most `max_line_bytes` content bytes.
///
/// # Errors
/// Returns the underlying I/O error from the reader. Content problems (too long, not UTF-8,
/// not JSON) are never errors here: length is reported as [`BoundedLine::Overflow`] and decoding
/// is the caller's job.
pub async fn read_bounded_line<R>(
    reader: &mut R,
    max_line_bytes: usize,
) -> std::io::Result<BoundedLine>
where
    R: AsyncBufRead + Unpin,
{
    // One byte of headroom so a line of exactly `max_line_bytes` still fits with its newline.
    let budget = u64::try_from(max_line_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut buf = Vec::new();
    let read = reader.take(budget).read_until(b'\n', &mut buf).await?;
    if read == 0 {
        return Ok(BoundedLine::Eof);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        return Ok(BoundedLine::Line(buf));
    }
    if buf.len() <= max_line_bytes {
        // End of stream inside a line: hand the partial line over; it will usually fail to parse.
        return Ok(BoundedLine::Line(buf));
    }
    // Overflow: drain the rest of this line in bounded chunks, never holding more than one chunk.
    let mut discarded = read as u64;
    loop {
        let mut scratch = Vec::new();
        let chunk = reader.take(budget).read_until(b'\n', &mut scratch).await?;
        discarded = discarded.saturating_add(chunk as u64);
        if chunk == 0 || scratch.last() == Some(&b'\n') {
            return Ok(BoundedLine::Overflow { discarded });
        }
    }
}

/// Serializes one JSON value as an NDJSON frame (compact JSON plus `\n`).
///
/// `serde_json` never emits a raw newline inside compact output (newlines in strings are
/// escaped), so the frame is always exactly one line.
///
/// # Errors
/// Returns the serialization error (only possible for values with non-string map keys or
/// failing custom `Serialize` impls).
pub fn encode_frame<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}
