//! Bounded native HTML clipboard reads absent from arboard; called only by the serialized clipboard worker.
use super::ClipboardError;
#[cfg_attr(not(target_os = "linux"), allow(clippy::unnecessary_wraps))] // Uniform fallible interface; Wayland is unavailable.
pub(super) fn available() -> Result<(), ClipboardError> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return Err(ClipboardError::Unsupported(
            "HTML clipboard access is unavailable through this Wayland clipboard backend. Use plain text or PNG, or run in an X11 session.",
        ));
    }
    Ok(())
}
#[cfg(target_os = "macos")]
pub(super) fn read() -> Result<String, ClipboardError> {
    let value = objc2_app_kit::NSPasteboard::generalPasteboard()
        .stringForType(&objc2_foundation::NSString::from_str("public.html"))
        .ok_or(ClipboardError::Operation)?
        .to_string();
    if value.len() > 262_144 {
        return Err(ClipboardError::Operation);
    }
    Ok(value)
}
#[cfg(windows)]
pub(super) fn read() -> Result<String, ClipboardError> {
    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    };
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            /* SAFETY: this guard owns the successful OpenClipboard call on this thread. */
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
    // SAFETY: all clipboard handles remain owned by Windows; bytes are copied while the clipboard is open and locked.
    let bytes = unsafe {
        OpenClipboard(None).map_err(|_| ClipboardError::Operation)?;
        let _open = Open;
        let format = RegisterClipboardFormatW(windows::core::w!("HTML Format"));
        let handle = GetClipboardData(format).map_err(|_| ClipboardError::Operation)?;
        let memory = HGLOBAL(handle.0);
        let size = GlobalSize(memory);
        if size == 0 || size > 263_168 {
            return Err(ClipboardError::Operation);
        }
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            return Err(ClipboardError::Operation);
        }
        let bytes = std::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec();
        let _ = GlobalUnlock(memory);
        bytes
    };
    let text = String::from_utf8(bytes).map_err(|_| ClipboardError::Operation)?;
    let offset = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|n| n.trim().parse::<usize>().ok())
    };
    let (start, end) = (
        offset("StartFragment:").ok_or(ClipboardError::Operation)?,
        offset("EndFragment:").ok_or(ClipboardError::Operation)?,
    );
    text.get(start..end)
        .filter(|s| s.len() <= 262_144)
        .map(str::to_owned)
        .ok_or(ClipboardError::Operation)
}
#[cfg(target_os = "linux")]
pub(super) fn read() -> Result<String, ClipboardError> {
    use x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{AtomEnum, ConnectionExt, CreateWindowAux, WindowClass},
        },
    };
    available()?;
    let run = || -> Result<String, Box<dyn std::error::Error>> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let window = conn.generate_id()?;
        conn.create_window(
            0,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new(),
        )?
        .check()?;
        let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        let clipboard = atom(b"CLIPBOARD")?;
        let html = atom(b"text/html")?;
        let property = atom(b"VW_CLIPBOARD_READ")?;
        conn.convert_selection(window, clipboard, html, property, x11rb::CURRENT_TIME)?
            .check()?;
        conn.flush()?;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < until {
            if let Some(Event::SelectionNotify(event)) = conn.poll_for_event()? {
                if event.requestor != window || event.selection != clipboard || event.target != html
                {
                    continue;
                }
                if event.property == 0 {
                    return Err("HTML unavailable".into());
                }
                let reply = conn
                    .get_property(true, window, property, AtomEnum::ANY, 0, 65_536)?
                    .reply()?;
                if reply.type_ != html || reply.format != 8 || reply.bytes_after != 0 {
                    return Err("HTML too large or requires incremental transfer".into());
                }
                return Ok(String::from_utf8(reply.value)?);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err("clipboard timed out".into())
    };
    run().map_err(|_| ClipboardError::Operation)
}
