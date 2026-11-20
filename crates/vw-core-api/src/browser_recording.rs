//! Consumes the browser driver's private GIF bytes before output fitting, then returns a verified chat-file receipt.
//! Agent tools and the browser command share this path; the browser never chooses the destination filename.
use crate::core::Shared;
use base64::Engine as _;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use vw_browser::recording::{VideoBridge, VideoCommand, VideoFuture};

struct GuardedVideoBridge {
    shared: Weak<Shared>,
    encoder: Arc<dyn VideoBridge>,
}
impl VideoBridge for GuardedVideoBridge {
    fn send(&self, request: VideoCommand) -> VideoFuture<'_> {
        self.encoder.send(request)
    }
    fn capture_allowed(&self, page: &str) -> bool {
        self.shared.upgrade().is_some_and(|shared| {
            !crate::browser_input_block::person_has_browser(&shared, page)
                && self.encoder.capture_allowed(page)
        })
    }
}

impl crate::Core {
    /// Connects the host's trusted `WebCodecs` encoder to the built-in browser after startup.
    pub fn set_browser_video_bridge(&self, encoder: Arc<dyn VideoBridge>) {
        self.shared
            .chrome_control
            .external_video
            .set_bridge(encoder.clone());
        if let Some(hub) = self.shared.preview.as_ref() {
            hub.browser.set_video_bridge(Arc::new(GuardedVideoBridge {
                shared: Arc::downgrade(&self.shared),
                encoder,
            }));
        }
    }
}
pub(crate) fn save(shared: &Shared, owner: &str, receipt: &mut Value) -> Result<(), String> {
    if let Some(encoded) = receipt
        .as_object_mut()
        .and_then(|o| o.remove("_vwRecordingWebm"))
    {
        let text = encoded
            .as_str()
            .filter(|s| s.len() <= 45 * 1024 * 1024)
            .ok_or_else(|| copy("browserVideoSaveUnavailable"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| copy("browserVideoSaveInvalid"))?;
        if bytes.len() > 32 * 1024 * 1024 || !bytes.starts_with(b"\x1a\x45\xdf\xa3") {
            return Err(copy("browserVideoSaveNotWebm").into());
        }
        let saved =
            crate::chrome_control::save_page_file(shared, owner, "webm", "video/webm", &bytes)?;
        receipt["recording"]["file"] = saved;
        receipt["Outcome"] = json!(copy("browserVideoSaved"));
        return Ok(());
    }
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
    let saved = crate::chrome_control::save_page_file(shared, owner, "gif", "image/gif", &bytes)?;
    receipt["recording"]["file"] = saved;
    receipt["Outcome"] = json!(
        "Action GIF stopped and saved in the chat folder. It contains action screenshots without audio; frames and omissions are listed in recording."
    );
    Ok(())
}

fn copy(key: &str) -> &'static str {
    vw_vocabulary::text(key).unwrap_or_default()
}
