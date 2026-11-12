//! Consumes the browser driver's private GIF bytes before output fitting, then returns a verified chat-file receipt.
//! Agent tools and the browser command share this path; the browser never chooses the destination filename.
use crate::core::Shared;
use base64::Engine as _;
use serde_json::{Value, json};
pub(crate) fn save(shared: &Shared, owner: &str, receipt: &mut Value) -> Result<(), String> {
    let Some(encoded) = receipt
        .as_object_mut()
        .and_then(|o| o.remove("_vwRecordingGif"))
    else {
        return Ok(());
    };
    let text = encoded
        .as_str()
        .filter(|s| s.len() <= 45 * 1024 * 1024)
        .ok_or("The recording bytes are unavailable or too large; record fewer actions.")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| "The recording bytes are invalid; start a new recording.")?;
    if !bytes.starts_with(b"GIF89a") && !bytes.starts_with(b"GIF87a") {
        return Err("The recording is not a GIF; start a new recording.".into());
    }
    let saved = crate::chrome_control::page_files::save(shared, owner, "gif", "image/gif", &bytes)?;
    receipt["recording"]["file"] = saved;
    receipt["Outcome"] = json!(
        "Action GIF stopped and saved in the chat folder. It contains action screenshots without audio; frames and omissions are listed in recording."
    );
    Ok(())
}
