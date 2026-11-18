//! Serialized Safari Accessibility/native input. Portable checks bind page coordinates to a verified native viewport.
use serde_json::Value;
use std::sync::{Arc, atomic::AtomicBool};
#[cfg(any(target_os = "macos", test))]
mod geometry;
#[cfg(any(target_os = "macos", test))]
mod lifecycle;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac;
#[cfg(any(target_os = "macos", test))]
mod plan;
/// A refused or interrupted native Safari input. `posted` is false while no macOS event has been posted for this
/// request (window proof, viewport match, coordinate mapping, cancellation, layout and plan checks), so the caller can
/// report a definite "nothing was sent" instead of an unknown outcome. Once the first event is posted it is true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    /// The instruction for the agent.
    pub message: String,
    /// Whether at least one native event was posted before the failure.
    pub posted: bool,
}
impl InputError {
    /// A failure before any native event: the page cannot have received input from this request.
    #[must_use]
    pub fn not_sent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            posted: false,
        }
    }
    /// A failure after at least one native event was posted: the page may have received part of the input.
    #[must_use]
    pub fn after_post(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            posted: true,
        }
    }
}
/// Perform one preparation or dispatch from an already approved, operation-bound app request.
/// Cancellation is checked before effects; callers can drop the future without waiting for OS accessibility calls.
///
/// # Errors
/// Refuses missing Accessibility permission, stale window proofs, cancellation, unsupported keys or unavailable platforms.
/// The error says whether any native event was posted before the failure.
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        clippy::unused_async,
        reason = "same asynchronous boundary on every platform"
    )
)]
pub async fn request(
    stage: &str,
    prepared: Value,
    action: &str,
    args: Value,
    url: &str,
    cancel: Arc<AtomicBool>,
) -> Result<Value, InputError> {
    if action == "page.key" {
        vw_ask::chrome::keys::validate_args(&args).map_err(InputError::not_sent)?;
        if stage == "dispatch"
            && vw_ask::chrome::keys::requires_editor(&args).map_err(InputError::not_sent)?
            && prepared["editorFocused"] != true
        {
            return Err(InputError::not_sent(
                "This editing shortcut has no checked focused editor and could invoke a browser command. Focus an editable field before sending it.",
            ));
        }
    }
    #[cfg(target_os = "macos")]
    {
        mac::request(stage, prepared, action, args, url, cancel).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (stage, prepared, action, args, url, cancel);
        Err(InputError::not_sent(
            "Native Safari input requires macOS. Run this action in Vibe Work on a Mac.",
        ))
    }
}

/// Show the macOS Accessibility prompt when the app is not trusted yet; returns whether it is trusted.
/// Call this only when the user completes Safari pairing: agent actions never prompt and refuse with the
/// System Settings instruction instead. Blocks briefly on a system query; other platforms return `false`.
#[must_use]
pub fn request_accessibility() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::request_accessibility()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Snapshot the native pasteboard generation without making a caller wait for its worker on Stop.
/// Non-macOS fixtures use generation zero; actual input remains macOS-only.
/// # Errors
/// Returns an instruction if the pasteboard worker stops before answering.
pub async fn clipboard_revision() -> Result<i64, String> {
    tokio::task::spawn_blocking(|| {
        #[cfg(target_os = "macos")]
        { objc2_app_kit::NSPasteboard::generalPasteboard().changeCount() as i64 }
        #[cfg(not(target_os = "macos"))]
        { 0 }
    }).await.map_err(|_| "The clipboard generation could not be checked. Copy ordinary content before trying Paste again.".into())
}
