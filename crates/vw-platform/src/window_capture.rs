//! Native window calls for whole-window screenshots that Tauri does not offer, each behind a safe
//! function. Adapted from ShellX's screenshot handling (Windows `PrintWindow` with
//! `PW_RENDERFULLCONTENT`, the restore-and-minimize-again guard for a minimized window).
//!
//! - Windows: [`print_window`] copies one of this process's windows into a bitmap with `PrintWindow`
//!   (`PW_RENDERFULLCONTENT`, the only flag that includes `WebView2`'s composition surface), also when other
//!   windows cover it; it hands back the display's ICC profile (`GetICMProfileW`), because `WebView2` renders in
//!   the display's colour space and the caller converts to sRGB. [`own_windows`] lists this
//!   process's visible top-level windows (the main window and native dialogs such as the confirmation box).
//! - macOS: [`view_place`] reports where a `WKWebView` sits in its window (top-left origin, points), where its
//!   page's viewport starts ([`viewport_top`]: below the title bar for a webview that reaches under it), whether it
//!   is hidden, and the window's backing scale, so the caller can compose the webviews' own snapshots and place
//!   child webviews in the main page's coordinates.
//!
//! Feature `webview`. Depends on: windows (Windows), objc2 / objc2-app-kit (macOS). Called by: vw-webview
//! (`window_shot`).
//! Invariants:
//! - Only windows of this process are listed or captured (`GetWindowThreadProcessId` is checked for every
//!   window handed in), never the screen or another app.
//! - Every GDI handle is released on every path; a size that does not fit the pixel cap is refused before any
//!   handle is made.
//! - `unsafe` is confined to the platform calls, each with its SAFETY comment.

/// One raw capture: top-down RGBA8 pixels and the display profile they are in (if Windows reported one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCapture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes, alpha 255.
    pub rgba: Vec<u8>,
    /// The ICC profile of the display the window is on (Windows), `None` when there is none.
    pub icc: Option<Vec<u8>>,
}

/// Largest capture accepted (an 8K x 8K window).
pub const MAX_CAPTURE_PIXELS: u64 = 8192 * 8192;

#[cfg(windows)]
pub use windows_impl::{OwnWindow, own_windows, print_window};

#[cfg(target_os = "macos")]
pub use macos_impl::{ViewPlace, view_place};

/// Top edge of a `WKWebView`'s page viewport in its window's content view (top-left origin, points).
///
/// `view_top` is the view's top edge there. `title_bar_bottom` is the bottom of the title bar in the same
/// coordinates (the top of the window's `contentLayoutRect`) when the window has a full-size content view under an
/// opaque title bar (Tauri's default macOS title bar style), `None` otherwise. In that case `WebKit` gives the view
/// an automatic top content inset (`WebViewImpl::updateContentInsetsIfAutomatic`: the part of the view above the
/// content layout rect), so the page's CSS `y = 0` is at the title bar's bottom, not at the view's top: the app's
/// own page fills the content view from its top and starts 32 pt lower (the title bar), and a view parked
/// 10,000 pt above the window got a 10,032 pt inset.
#[must_use]
pub fn viewport_top(view_top: f64, title_bar_bottom: Option<f64>) -> f64 {
    title_bar_bottom.map_or(view_top, |bottom| view_top.max(bottom))
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use std::mem::size_of;

    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::ColorSystem::GetICMProfileW;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetClassNameW, GetClientRect, GetWindow, GetWindowRect,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_MINIMIZE,
        SW_RESTORE, ShowWindow,
    };
    use windows::core::{BOOL, PWSTR};

    use super::{MAX_CAPTURE_PIXELS, RawCapture};

    /// `PW_CLIENTONLY`: the client area only (no frame).
    const CLIENT_ONLY: u32 = 0x1;
    /// `PW_RENDERFULLCONTENT` (not in the `windows` crate's constants): includes `DirectComposition` content.
    const RENDER_FULL_CONTENT: u32 = 0x2;

    /// One of this process's visible top-level windows.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct OwnWindow {
        /// The handle as a number (stable while the window lives).
        pub hwnd: isize,
        /// The title.
        pub title: String,
        /// The window class (`#32770` for a standard dialog box).
        pub class: String,
        /// The owner window's handle (0 when none): a modal dialog is owned by the window it belongs to.
        pub owner: isize,
    }

    fn hwnd_of(value: isize) -> HWND {
        HWND(value as *mut core::ffi::c_void)
    }

    fn is_own(hwnd: HWND) -> bool {
        let mut pid = 0u32;
        // SAFETY: a valid out pointer; an invalid handle only yields 0.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        pid == std::process::id()
    }

    fn text_of(hwnd: HWND, class: bool) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: the buffer outlives the call and its length is passed.
        let n = unsafe {
            if class {
                GetClassNameW(hwnd, &mut buf)
            } else {
                GetWindowTextW(hwnd, &mut buf)
            }
        };
        String::from_utf16_lossy(&buf[..usize::try_from(n.max(0)).unwrap_or(0)])
    }

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Vec<OwnWindow>` `own_windows` passed for the duration of EnumWindows.
        let out = unsafe { &mut *(lparam.0 as *mut Vec<OwnWindow>) };
        // SAFETY: plain query on a handle the system just handed out.
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        if visible && is_own(hwnd) {
            // SAFETY: plain query; a window without an owner yields an error, read as 0.
            let owner = unsafe { GetWindow(hwnd, GW_OWNER) }.map_or(0, |o| o.0 as isize);
            out.push(OwnWindow {
                hwnd: hwnd.0 as isize,
                title: text_of(hwnd, false),
                class: text_of(hwnd, true),
                owner,
            });
        }
        BOOL(1)
    }

    /// This process's visible top-level windows, in z-order (front first).
    #[must_use]
    pub fn own_windows() -> Vec<OwnWindow> {
        let mut out: Vec<OwnWindow> = Vec::new();
        // SAFETY: the callback only touches `out` through `lparam`, which lives across the call.
        let _ = unsafe { EnumWindows(Some(collect), LPARAM(&raw mut out as isize)) };
        out
    }

    /// Restores a minimized window for the capture and minimizes it again afterwards (ShellX's guard).
    struct MinimizedGuard(Option<HWND>);

    impl Drop for MinimizedGuard {
        fn drop(&mut self) {
            if let Some(hwnd) = self.0 {
                // SAFETY: the handle was valid when the guard was made and the capture holds no lock on it.
                let _ = unsafe { ShowWindow(hwnd, SW_MINIMIZE) };
            }
        }
    }

    fn display_profile(hdc: windows::Win32::Graphics::Gdi::HDC) -> Option<Vec<u8>> {
        let mut size = 0u32;
        // SAFETY: first call asks for the buffer size (no buffer passed).
        let _ = unsafe { GetICMProfileW(hdc, &raw mut size, None) };
        if size == 0 || size > 4096 {
            return None;
        }
        let mut name = vec![0u16; size as usize];
        // SAFETY: the buffer has the size Windows asked for and outlives the call.
        let ok = unsafe { GetICMProfileW(hdc, &raw mut size, Some(PWSTR(name.as_mut_ptr()))) };
        if !ok.as_bool() {
            return None;
        }
        let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        let path = String::from_utf16_lossy(&name[..end]);
        std::fs::read(path)
            .ok()
            .filter(|bytes| bytes.len() <= 4 * 1024 * 1024)
    }

    /// Copies window `hwnd_value` (one of this process's) into RGBA pixels. `client_only`: the client area (the
    /// main window); otherwise the whole window (a dialog with its frame). A minimized window is restored for the
    /// capture and minimized again.
    ///
    /// # Errors
    /// A text reason: not this process's window, an empty or too large size, a failed GDI step.
    pub fn print_window(hwnd_value: isize, client_only: bool) -> Result<RawCapture, String> {
        let hwnd = hwnd_of(hwnd_value);
        if hwnd_value == 0 || !is_own(hwnd) {
            return Err("not a window of this app".to_owned());
        }
        // SAFETY: plain queries and a show command on a window of this process.
        let _guard = unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                std::thread::sleep(std::time::Duration::from_millis(80));
                MinimizedGuard(Some(hwnd))
            } else {
                MinimizedGuard(None)
            }
        };
        let mut rect = RECT::default();
        // SAFETY: valid out pointer.
        unsafe {
            if client_only {
                GetClientRect(hwnd, &raw mut rect)
            } else {
                GetWindowRect(hwnd, &raw mut rect)
            }
        }
        .map_err(|e| format!("window size: {e}"))?;
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let width = u32::try_from(w).map_err(|_| "empty window")?;
        let height = u32::try_from(h).map_err(|_| "empty window")?;
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_CAPTURE_PIXELS {
            return Err(format!("window size {width}x{height} cannot be captured"));
        }
        let mut buf = vec![0u8; (width as usize) * (height as usize) * 4];
        let flags =
            PRINT_WINDOW_FLAGS(RENDER_FULL_CONTENT | if client_only { CLIENT_ONLY } else { 0 });
        // SAFETY: every handle made below is checked and released on every path before returning; the bitmap
        // buffer has exactly width * height * 4 bytes for the top-down 32-bit DIB requested.
        unsafe {
            let screen = GetDC(Some(hwnd));
            if screen.is_invalid() {
                return Err("GetDC failed".to_owned());
            }
            let icc = display_profile(screen);
            let memory = CreateCompatibleDC(Some(screen));
            if memory.is_invalid() {
                ReleaseDC(Some(hwnd), screen);
                return Err("CreateCompatibleDC failed".to_owned());
            }
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            if bitmap.is_invalid() {
                let _ = DeleteDC(memory);
                ReleaseDC(Some(hwnd), screen);
                return Err("CreateCompatibleBitmap failed".to_owned());
            }
            let old = SelectObject(memory, HGDIOBJ(bitmap.0));
            let printed = PrintWindow(hwnd, memory, flags).as_bool();
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(40);
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;
            let lines = GetDIBits(
                memory,
                bitmap,
                0,
                height,
                Some(buf.as_mut_ptr().cast()),
                &raw mut info,
                DIB_RGB_COLORS,
            );
            SelectObject(memory, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(memory);
            ReleaseDC(Some(hwnd), screen);
            if !printed {
                return Err("PrintWindow failed".to_owned());
            }
            if lines == 0 {
                return Err("GetDIBits failed".to_owned());
            }
            // BGRA from GDI, alpha usually 0: to RGBA, opaque.
            for px in buf.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
                px[3] = 255;
            }
            Ok(RawCapture {
                width,
                height,
                rgba: buf,
                icc,
            })
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos_impl {
    use std::ffi::c_void;

    use objc2_app_kit::{NSView, NSWindowStyleMask};

    /// Where a webview's view sits in its window.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct ViewPlace {
        /// Left edge in the window's content, points.
        pub x: f64,
        /// Top edge in the window's content (top-left origin), points.
        pub y: f64,
        /// Width, points.
        pub width: f64,
        /// Height, points.
        pub height: f64,
        /// Top edge of the page's viewport in the window's content (top-left origin), points: `y` plus the
        /// automatic title bar inset `WebKit` gives a view that reaches under an opaque title bar ([`super::viewport_top`]).
        pub viewport_top: f64,
        /// The content view's height, points (the composite's height).
        pub content_height: f64,
        /// The content view's width, points.
        pub content_width: f64,
        /// Hidden itself or through an ancestor.
        pub hidden: bool,
        /// Device pixels per point of the window.
        pub scale: f64,
    }

    /// The place of the `WKWebView` (Tauri's `PlatformWebview::inner()`, on the main thread) in its window, or
    /// `None` when it is not in a window.
    #[must_use]
    pub fn view_place(webview: *mut c_void) -> Option<ViewPlace> {
        if webview.is_null() {
            return None;
        }
        // SAFETY: Tauri hands us its live `WKWebView` (an `NSView`) on the main thread inside `with_webview`; it
        // stays alive for this call. Only read-only geometry queries are made.
        unsafe {
            let view = &*webview.cast::<NSView>();
            let window = view.window()?;
            let content = window.contentView()?;
            let content_frame = content.frame();
            let bounds = view.bounds();
            let in_content = view.convertRect_toView(bounds, Some(&content));
            // AppKit's content view is not flipped: origin bottom-left. The composite uses top-left.
            let top_of = |origin_y: f64, height: f64| {
                if content.isFlipped() {
                    origin_y
                } else {
                    content_frame.size.height - origin_y - height
                }
            };
            let top = top_of(in_content.origin.y, in_content.size.height);
            // WebKit's condition for the automatic inset: a full-size content view under an opaque title bar.
            let title_bar_bottom = (window
                .styleMask()
                .contains(NSWindowStyleMask::FullSizeContentView)
                && !window.titlebarAppearsTransparent())
            .then(|| {
                let layout = content.convertRect_fromView(window.contentLayoutRect(), None);
                top_of(layout.origin.y, layout.size.height)
            });
            Some(ViewPlace {
                x: in_content.origin.x,
                y: top,
                width: in_content.size.width,
                height: in_content.size.height,
                viewport_top: super::viewport_top(top, title_bar_bottom),
                content_height: content_frame.size.height,
                content_width: content_frame.size.width,
                hidden: view.isHiddenOrHasHiddenAncestor(),
                scale: window.backingScaleFactor(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::viewport_top;

    #[test]
    fn a_view_under_an_opaque_full_size_title_bar_starts_its_page_below_the_title_bar() {
        // The app's own page: fills the content view from its top, the title bar is 32 pt.
        assert!((viewport_top(0.0, Some(32.0)) - 32.0).abs() < f64::EPSILON);
        // A view parked above the window: its page starts at the title bar too (the 10,032 pt inset).
        assert!((viewport_top(-10_000.0, Some(32.0)) - 32.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_view_below_the_title_bar_or_without_one_starts_at_its_top() {
        assert!((viewport_top(296.0, Some(32.0)) - 296.0).abs() < f64::EPSILON);
        assert!((viewport_top(0.0, None)).abs() < f64::EPSILON);
        assert!((viewport_top(-40.0, None) + 40.0).abs() < f64::EPSILON);
    }
}
