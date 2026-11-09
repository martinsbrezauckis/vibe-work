//! Explicit browser clipboard actions through the app's existing serialized clipboard owner.
//! Called after owned-page selection by agent and person routes; vault copies and registered values never leave.
use crate::core::Shared;
use serde_json::{Value, json};

pub(crate) async fn run(shared: &Shared, args: &Value) -> Result<Value, String> {
    let clipboard = shared
        .config
        .clipboard
        .as_ref()
        .ok_or("Clipboard is unavailable; check the system clipboard before trying again.")?;
    match args["action"].as_str() {
        Some("clipboard.read") => {
            let text=clipboard.read_text().await.map_err(|_| "Clipboard text is unavailable or contains a copied vault value; use ordinary page text before trying again.")?;
            let text = vw_secrets::scrub(text.as_bytes())
                .map_or(text, |(v, _)| String::from_utf8_lossy(&v).into_owned());
            let mut end = text.len().min(65_536);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Ok(
                json!({"Outcome":"Clipboard text read.","untrustedPageContent":{"text":&text[..end],"truncated":end<text.len()}}),
            )
        }
        Some("clipboard.write") => {
            let text = args["text"].as_str().filter(|v| v.len() <= 65_536).ok_or(
                "Clipboard text must be text up to 65536 UTF-8 bytes. Send a smaller text value.",
            )?;
            clipboard.write_text(text.to_owned()).await.map_err(
                |_| "Clipboard write failed; check the system clipboard before trying again.",
            )?;
            Ok(json!({"Outcome":"Clipboard text written.","written":true,"bytes":text.len()}))
        }
        _ => Err("Unknown clipboard action; use clipboard.read or clipboard.write.".into()),
    }
}
