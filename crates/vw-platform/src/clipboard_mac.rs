//! Extra macOS clipboard privacy marker absent from the cross-platform clipboard API.
//! Depends on: the macOS pasteboard API. Called by: the native clipboard backend after writing text.

use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

/// Marks the current clipboard item as short-lived after the concealed marker has been written.
pub(crate) fn mark_transient() -> Result<(), crate::clipboard::ClipboardError> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let empty = NSString::from_str("");
    let marker = NSString::from_str("org.nspasteboard.TransientType");
    // The retained strings live throughout the Objective-C call (objc2 marks it safe). The call only adds a
    // marker to the general pasteboard item, without exposing the clipboard text to Rust callers.
    let transient_set = pasteboard.setString_forType(&empty, &marker);
    if transient_set {
        Ok(())
    } else {
        Err(crate::clipboard::ClipboardError::Operation)
    }
}
