//! Value-free decoding errors for typed core commands; used by the Debug API.
//! Depends on serde diagnostics and core error types; input values never enter the message.

use crate::error::{CoreError, ErrorCode, clip};

/// Maps a request body that is not a valid command (vw-debug-api uses it for decode errors).
///
/// The text names keys only, never values: serde's messages can quote the caller's values
/// (`invalid type: string "..."`), and refusals are logged and broadcast. Messages about a
/// missing or unknown field, or an unknown command type, name our own schema's keys (or the
/// caller's key, clipped) and are kept; anything else becomes a fixed text.
#[must_use]
pub fn invalid_command(error: &serde_json::Error) -> CoreError {
    let text = error.to_string();
    let keep = ["missing field `", "unknown field `", "unknown variant `"]
        .iter()
        .any(|prefix| text.starts_with(prefix));
    let detail = if keep {
        // Drop serde's "expected one of ..." tail and the position: the key is what matters.
        let head = text.split(", expected").next().unwrap_or(&text);
        clip(head.split(" at line ").next().unwrap_or(head), 200)
    } else if error.is_syntax() || error.is_eof() {
        "the body is not valid JSON".to_owned()
    } else {
        "a field has the wrong type or value".to_owned()
    };
    CoreError::new(
        ErrorCode::InvalidCommand,
        format!("not a valid command: {detail}"),
    )
}
