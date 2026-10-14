//! A Vibe background process in the fake server (the fixture): `VW_FAKE_BACKGROUND=<processId>=<output text>` gives
//! every started session one running background process of that id in its state (`backgroundProcesses`), and
//! `session/backgroundProcess/output` answers the text from the asked cursor on (bytes), like Vibe's reader:
//! `outputStartCursor` = the cursor, `nextCursor` = the end, `hasMore` false, `isFinal` false.
//! `VW_FAKE_BACKGROUND_LOG=<file>` appends one line per output request (`<sessionId> <processId> <cursor>`), so
//! a test can see which cursors the core asked for.

use std::io::Write as _;

use base64::Engine as _;
use serde_json::{Value, json};

/// The configured `(processId, text)`.
pub fn fixture() -> Option<(String, String)> {
    let spec = std::env::var("VW_FAKE_BACKGROUND").ok()?;
    let (id, text) = spec.split_once('=')?;
    Some((id.to_owned(), text.replace("\\n", "\n")))
}

/// Adds the fixture's process to a fresh session state.
pub fn add_to(state: &mut Value) {
    if let Some((id, _)) = fixture() {
        state["backgroundProcesses"] =
            json!([{"processId": id, "command": "dev-server", "status": "running"}]);
    }
}

/// Answers `session/backgroundProcess/output`.
pub fn output(params: &Value) -> Value {
    let id = params["processId"].as_str().unwrap_or_default();
    let cursor = usize::try_from(params["cursor"].as_i64().unwrap_or(0)).unwrap_or(0);
    if let Some(path) = std::env::var_os("VW_FAKE_BACKGROUND_LOG")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = writeln!(
            file,
            "{} {id} {cursor}",
            params["sessionId"].as_str().unwrap_or_default()
        );
    }
    match fixture() {
        Some((fixture_id, text)) if fixture_id == id => {
            let bytes = text.as_bytes();
            let from = cursor.min(bytes.len());
            json!({
                "availability": "available", "processId": id,
                "outputBase64": base64::engine::general_purpose::STANDARD.encode(&bytes[from..]),
                "outputStartCursor": from, "nextCursor": bytes.len(),
                "bytesAvailable": bytes.len() - from, "hasMore": false,
                "truncatedBefore": false, "isFinal": false,
            })
        }
        _ => json!({"availability": "unavailable", "processId": id}),
    }
}
