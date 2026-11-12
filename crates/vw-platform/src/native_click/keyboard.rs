//! Bounded native keyboard payloads. Browser admission uses the shared key policy before platform translation.
//! Called by checked frame dispatch; adapters target only its current native webview.
use serde_json::Value;

/// One native text insertion or page key, already bounded and normalized by the shared policy.
#[derive(Clone)]
pub enum Keyboard {
    /// Unicode text replacing the page's prepared selection.
    Text(String),
    /// Page key and Alt/Control/Meta/Shift bits (the same order as native pointer modifiers).
    Key(String, u8),
}
impl Keyboard {
    /// Parse one host-owned keyboard payload.
    /// # Errors
    /// Refuses malformed text or a key outside the shared browser key allowlist.
    pub fn parse(value: &Value) -> Result<Self, String> {
        if let Some(text) = value.get("text") {
            let text = text
                .as_str()
                .filter(|text| text.chars().count() <= 10_000 && !text.contains('\0'))
                .ok_or("Native text is invalid. Supply at most 10000 characters without NUL.")?;
            return Ok(Self::Text(text.to_owned()));
        }
        vw_ask::chrome::keys::validate_args(value)?;
        if vw_ask::chrome::keys::clipboard_operation(value)?.is_some() {
            return Err("Native clipboard shortcuts bypass the app's protected clipboard. Use clipboard.read or clipboard.write, then insert_text for text entry.".into());
        }
        let names: Vec<_> = value["modifiers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let (key, modifiers) =
            vw_ask::chrome::keys::canonical(value["key"].as_str().unwrap_or(""), &names)?;
        let flags = modifiers.iter().fold(0, |bits, name| {
            bits | match *name {
                "Alt" => 1,
                "Control" => 2,
                "Meta" => 4,
                "Shift" => 8,
                _ => 0,
            }
        });
        Ok(Self::Key(key, flags))
    }
}

#[cfg(target_os = "linux")]
#[path = "keyboard_linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::send as linux_send;
#[cfg(windows)]
#[path = "keyboard_windows.rs"]
mod windows;
#[cfg(windows)]
pub use windows::send as windows_send;
#[cfg(target_os = "macos")]
#[path = "keyboard_macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::send as macos_send;
