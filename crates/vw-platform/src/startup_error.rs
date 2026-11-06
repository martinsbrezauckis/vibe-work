//! Last-resort startup error dialog without a running Tauri app or an existing window.
//! Depends on GTK, `AppKit` or Win32. Called on the main thread by app setup and the binary entry point.
//! Blocks until dismissed; the caller then exits normally. Always writes the reason to stderr too.

const TITLE: &str = "Vibe Work could not start";

/// Shows a native startup failure with its reason and a recovery instruction.
///
/// Call on the main thread. If no graphical display is available, stderr carries the reason.
pub fn show(reason: &str) {
    eprintln!("{TITLE}: {reason}");
    native(&format!(
        "{reason}\n\nQuit and try opening Vibe Work again."
    ));
}

#[cfg(target_os = "linux")]
fn native(body: &str) {
    use gtk::prelude::*;

    if gtk::init().is_err() {
        return;
    }
    let dialog = gtk::MessageDialog::builder()
        .title(TITLE)
        .text(TITLE)
        .secondary_text(body)
        .message_type(gtk::MessageType::Error)
        .modal(true)
        .build();
    dialog.add_button("Quit", gtk::ResponseType::Close);
    dialog.set_default_response(gtk::ResponseType::Close);
    dialog.run();
    dialog.close();
}

#[cfg(target_os = "macos")]
fn native(body: &str) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAlert, NSAlertStyle, NSApplication};
    use objc2_foundation::NSString;

    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(main);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    let alert = NSAlert::new(main);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str(TITLE));
    alert.setInformativeText(&NSString::from_str(body));
    alert.addButtonWithTitle(&NSString::from_str("Quit"));
    alert.runModal();
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "native Win32 message box; unsafe stays in vw-platform"
)]
fn native(body: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TASKMODAL, MessageBoxW,
    };
    use windows::core::PCWSTR;

    let title: Vec<u16> = TITLE.encode_utf16().chain(Some(0)).collect();
    let text: Vec<u16> = body
        .replace('\0', "�")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: both strings are terminated and live for the entire modal call; there is no parent window.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TASKMODAL,
        );
    }
}
