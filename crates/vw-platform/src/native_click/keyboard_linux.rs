//! GTK/WebKit keyboard delivery to the checked visible webview; no global keyboard injection.
//! Called on the UI thread after frame receipt arming, dispatch mark and permission recheck.
use super::Keyboard;
use gdk::prelude::*;
use glib::translate::{IntoGlib, ToGlibPtr};
use gtk::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use webkit2gtk::WebViewExt;

/// Sends one bounded keyboard operation to this native view.
/// # Errors
/// Refuses unavailable or unfocused native views before any input.
pub fn send(view: &webkit2gtk::WebView, input: &Keyboard, sent: &AtomicBool) -> Result<(), String> {
    let widget: &gtk::Widget = view.upcast_ref();
    let top = widget
        .toplevel()
        .and_then(|top| top.downcast::<gtk::Window>().ok())
        .ok_or("Native keyboard has no app window. Open the browser pane and try again.")?;
    if !top.is_active() || !widget.is_mapped() || !widget.is_visible() || !widget.is_child_visible()
    {
        return Err(
            "Native keyboard needs the visible active browser pane. Open it and try again.".into(),
        );
    }
    widget.grab_focus();
    if !widget.has_focus() {
        return Err(
            "The browser pane could not take keyboard focus. Focus it and try again.".into(),
        );
    }
    let window = widget
        .window()
        .filter(gdk::Window::is_viewable)
        .ok_or("The native browser window is unavailable. Open the browser pane and try again.")?;
    let device = window
        .display()
        .default_seat()
        .and_then(|seat| seat.keyboard())
        .ok_or("Native keyboard device is unavailable. Try again after restoring the keyboard.")?;
    match input {
        Keyboard::Text(text) => {
            sent.store(true, Ordering::Release);
            // WebKit's editor command inserts one Unicode string through the browser's native editing path.
            view.execute_editing_command_with_argument("InsertText", text);
        }
        Keyboard::Key(name, modifiers) => {
            let key = key(name);
            let entries = gdk::Keymap::for_display(&window.display())
                .map(|map| map.entries_for_keyval(key.into_glib()))
                .unwrap_or_default();
            let hardware = entries
                .first()
                .and_then(|entry| u16::try_from(entry.keycode()).ok())
                .unwrap_or(0);
            super::super::person::own_input(|| {
                sent.store(true, Ordering::Release);
                for kind in [gdk::EventType::KeyPress, gdk::EventType::KeyRelease] {
                    let mut event = gdk::Event::new(kind);
                    event.set_device(Some(&device));
                    // SAFETY: Event::new allocated a GdkEventKey for this key event type. GDK owns its window/device.
                    unsafe {
                        let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none().0;
                        let fields = &mut (*raw).key;
                        fields.window = window.to_glib_none().0;
                        glib::gobject_ffi::g_object_ref(fields.window.cast());
                        fields.send_event = 1;
                        fields.time = 0;
                        fields.keyval = key.into_glib();
                        fields.hardware_keycode = hardware;
                        fields.state = state(*modifiers).bits();
                    }
                    gtk::main_do_event(&mut event);
                }
            });
        }
    }
    Ok(())
}
fn key(name: &str) -> gdk::keys::Key {
    if name.chars().count() == 1 {
        return gdk::keys::Key::from_unicode(name.chars().next().unwrap_or(' '));
    }
    gdk::keys::Key::from_name(match name {
        "Backspace" => "BackSpace",
        "Enter" => "Return",
        "PageUp" => "Page_Up",
        "PageDown" => "Page_Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "Space" => "space",
        "Shift" => "Shift_L",
        "Control" => "Control_L",
        "Alt" => "Alt_L",
        "Meta" => "Meta_L",
        other => other,
    })
}
fn state(bits: u8) -> gdk::ModifierType {
    [
        (1, gdk::ModifierType::MOD1_MASK),
        (2, gdk::ModifierType::CONTROL_MASK),
        (4, gdk::ModifierType::META_MASK),
        (8, gdk::ModifierType::SHIFT_MASK),
    ]
    .into_iter()
    .filter(|(bit, _)| bits & bit != 0)
    .fold(gdk::ModifierType::empty(), |mask, (_, bit)| mask | bit)
}
