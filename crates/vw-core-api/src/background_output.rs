//! Reads the output of Vibe's background processes into the process monitor (`note_output`), so a
//! dev server's "Local: `http://localhost:5173/`" line corroborates the listener it names.
//!
//! Depends on: `core::Shared` (channels, store), `processes` (session contexts), vw-procmon (`note_output`),
//! base64. Called by: `processes::spawn_tick` (one pass per monitor tick).
//! Invariants:
//! - Only sessions with an open channel are read; a read never opens one (it would take the session's lease
//!   from the Vibe CLI). A session with a running background process is never idle-closed anyway.
//! - The core keeps one cursor per (session, process) and reads on from it, so a byte is read once.
//! - A read is `waitMs: 0` and bounded (`READ_BYTES`, `MAX_READS` per pass and process): it never waits
//!   on the process and a chatty process cannot hold the pass.
//! - Reading is evidence gathering only: a failure is skipped (and retried next pass), never reported.
//! - A process stops being read once Vibe says its output is final or unavailable, or it is no longer
//!   running and a read returned nothing new.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::core::Shared;
use crate::processes::contexts;

/// Vibe's reader method.
const METHOD: &str = "session/backgroundProcess/output";
/// Largest chunk asked per read.
const READ_BYTES: i64 = 16 * 1024;
/// Most reads per process and pass (a process printing faster is caught up on the next pass).
const MAX_READS: usize = 8;
/// Longest one read may take.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Where reading a process stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Progress {
    /// Next read starts at this cursor.
    At(i64),
    /// Nothing more to read.
    Done,
}

/// The cursors of every background process seen so far; lives in the monitor tick task.
#[derive(Default)]
pub(crate) struct OutputReader {
    progress: HashMap<(String, String), Progress>,
}

/// What one read gave.
enum Chunk {
    /// Text (possibly empty) and the cursor to continue from; `more` = Vibe has more ready now.
    Text {
        text: String,
        next: i64,
        more: bool,
        is_final: bool,
    },
    /// Vibe has no output for the process.
    Unavailable,
}

impl OutputReader {
    /// One pass over every session with an open channel. Never fails; read errors are retried next pass.
    pub(crate) async fn pass(&mut self, shared: &Arc<Shared>) {
        let sessions = contexts(&shared.store);
        let live: Vec<(String, String)> = sessions
            .iter()
            .flat_map(|s| {
                s.background
                    .iter()
                    .map(|b| (s.session_id.clone(), b.process_id.clone()))
            })
            .collect();
        // Forget processes Vibe no longer lists.
        self.progress.retain(|key, _| live.contains(key));
        for session in &sessions {
            if !shared.channels.has_channel(&session.session_id) {
                continue;
            }
            for background in &session.background {
                let key = (session.session_id.clone(), background.process_id.clone());
                let running = background.status == "running";
                self.read_process(shared, key, running).await;
            }
        }
    }

    async fn read_process(&mut self, shared: &Arc<Shared>, key: (String, String), running: bool) {
        let mut cursor = match self.progress.get(&key) {
            Some(Progress::Done) => return,
            Some(Progress::At(cursor)) => *cursor,
            None => 0,
        };
        for _ in 0..MAX_READS {
            let Some(chunk) = read(shared, &key, cursor).await else {
                return;
            };
            match chunk {
                Chunk::Unavailable => {
                    self.progress.insert(key, Progress::Done);
                    return;
                }
                Chunk::Text {
                    text,
                    next,
                    more,
                    is_final,
                } => {
                    let fresh = !text.is_empty();
                    if fresh {
                        shared.procmon.note_output(&key.0, &key.1, &text);
                    }
                    cursor = next.max(cursor);
                    let finished = is_final || (!running && !fresh);
                    self.progress.insert(
                        key.clone(),
                        if finished {
                            Progress::Done
                        } else {
                            Progress::At(cursor)
                        },
                    );
                    if finished || !more {
                        return;
                    }
                }
            }
        }
    }
}

/// One read from `cursor`; `None` on any failure (no channel answer, bad shape).
async fn read(shared: &Arc<Shared>, key: &(String, String), cursor: i64) -> Option<Chunk> {
    let params = json!({
        "sessionId": key.0, "processId": key.1,
        "fromEnd": false, "cursor": cursor, "waitMs": 0, "maxBytes": READ_BYTES,
    });
    let answer = tokio::time::timeout(READ_TIMEOUT, shared.channels.request(METHOD, params))
        .await
        .ok()?
        .ok()?;
    parse(&answer)
}

/// Decodes an answer of `session/backgroundProcess/output`.
fn parse(answer: &Value) -> Option<Chunk> {
    if answer.get("availability").and_then(Value::as_str) != Some("available") {
        return Some(Chunk::Unavailable);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(answer.get("outputBase64")?.as_str()?)
        .ok()?;
    Some(Chunk::Text {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        next: answer.get("nextCursor")?.as_i64()?,
        more: answer.get("hasMore").and_then(Value::as_bool) == Some(true),
        is_final: answer.get("isFinal").and_then(Value::as_bool) == Some(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available(text: &str, next: i64, more: bool, is_final: bool) -> Value {
        json!({
            "availability": "available", "processId": "p",
            "outputBase64": base64::engine::general_purpose::STANDARD.encode(text),
            "outputStartCursor": 0, "nextCursor": next, "bytesAvailable": 0,
            "hasMore": more, "truncatedBefore": false, "isFinal": is_final,
        })
    }

    #[test]
    fn an_available_answer_gives_text_and_the_next_cursor() {
        let Some(Chunk::Text {
            text,
            next,
            more,
            is_final,
        }) = parse(&available("Local: http://localhost:5173/", 29, true, false))
        else {
            panic!("not text")
        };
        assert_eq!(text, "Local: http://localhost:5173/");
        assert_eq!((next, more, is_final), (29, true, false));
    }

    #[test]
    fn an_unavailable_answer_ends_reading() {
        let answer = json!({"availability": "unavailable", "processId": "p"});
        assert!(matches!(parse(&answer), Some(Chunk::Unavailable)));
    }

    #[test]
    fn a_malformed_answer_is_skipped_not_fatal() {
        let mut answer = available("x", 1, false, false);
        answer["outputBase64"] = json!("!!not base64!!");
        assert!(parse(&answer).is_none());
        let mut answer = available("x", 1, false, false);
        answer["nextCursor"] = json!("one");
        assert!(parse(&answer).is_none());
    }

    #[test]
    fn invalid_utf8_is_replaced_not_dropped() {
        let mut answer = available("", 2, false, false);
        answer["outputBase64"] =
            json!(base64::engine::general_purpose::STANDARD.encode([b'a', 0xFF]));
        let Some(Chunk::Text { text, .. }) = parse(&answer) else {
            panic!("not text")
        };
        assert_eq!(text, "a\u{FFFD}");
    }
}
