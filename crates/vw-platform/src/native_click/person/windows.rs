//! Windows watch of the person's pointer presses and releases on a preview webview: a low-level mouse hook
//! (`WH_MOUSE_LL`) on a thread of its own, so a busy UI thread can never make Windows skip it. The hook sees
//! input from the input system (the person's devices, and other programs' injected input, which also counts
//! as not ours). Our own checked click is posted straight to the browser window (`PostMessageW`) and never
//! passes this hook, so it is never taken for the person's. Page script cannot reach any of this.
//!
//! Depends on: Win32 hooks and window queries, the `WebView2` controller (bounds, host window). Called by
//! vw-webview on the UI thread (watch at mount, refresh and query at the click); the hook runs on its thread.

use std::sync::{Mutex, OnceLock, PoisonError};

use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetDoubleClickTime, VIRTUAL_KEY, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
    VK_XBUTTON1, VK_XBUTTON2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GA_ROOT, GetAncestor, GetCursorPos, GetForegroundWindow,
    GetMessageW, HC_ACTION, IsChild, IsWindowVisible, MSG, MSLLHOOKSTRUCT, SetWindowsHookExW,
    TranslateMessage, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, WindowFromPoint,
};
use windows::core::Interface;

/// Receives one press (`true`) or release (`false`) of the person's; answering `false` means the view is gone and
/// the watch ends.
pub type Observed = Box<dyn Fn(bool) -> bool + Send + Sync>;

struct Watched {
    key: usize,
    /// The controller's host window and the view's bounds in its client area (refreshed at every click).
    host: isize,
    bounds: RECT,
    pressed_inside: bool,
    observed: Observed,
}

static WATCHED: Mutex<Vec<Watched>> = Mutex::new(Vec::new());
static HOOK: OnceLock<Result<(), String>> = OnceLock::new();

fn place(controller: &ICoreWebView2Controller) -> Result<(isize, RECT), String> {
    let (mut host, mut bounds) = (HWND::default(), RECT::default());
    // SAFETY: the live controller on its UI thread; both getters only read it.
    unsafe {
        controller
            .ParentWindow(&raw mut host)
            .and_then(|()| controller.Bounds(&raw mut bounds))
    }
    .map_err(|e| format!("WebView2 placement: {e}"))?;
    Ok((host.0 as isize, bounds))
}

/// Whether a screen point is over this view: inside its bounds and over its host window's own children, not
/// over another window covering it.
fn over(host: isize, bounds: &RECT, at: POINT) -> bool {
    let host = HWND(host as *mut _);
    let mut origin = POINT::default();
    // SAFETY: window queries by handle; a destroyed host fails the visibility check or the mapping.
    unsafe {
        if host.is_invalid()
            || !IsWindowVisible(host).as_bool()
            || !ClientToScreen(host, &raw mut origin).as_bool()
        {
            return false;
        }
        let (x, y) = (at.x - origin.x, at.y - origin.y);
        if x < bounds.left || x >= bounds.right || y < bounds.top || y >= bounds.bottom {
            return false;
        }
        let hit = WindowFromPoint(at);
        hit == host || IsChild(host, hit).as_bool()
    }
}

fn observe(at: POINT, press: bool) {
    let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
    watched.retain_mut(|entry| {
        let within = over(entry.host, &entry.bounds, at);
        // A release counts where it happens, or wherever it ends a press that began in the view.
        if within || (!press && entry.pressed_inside) {
            entry.pressed_inside = press && within;
            return (entry.observed)(press);
        }
        true
    });
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION.cast_signed() {
        let message = u32::try_from(wparam.0).unwrap_or_default();
        let press = matches!(
            message,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        );
        if press
            || matches!(
                message,
                WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP
            )
        {
            // SAFETY: for HC_ACTION, Windows passes a valid MSLLHOOKSTRUCT for the duration of this call.
            let at = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
            observe(at, press);
        }
    }
    // SAFETY: passes the event on unchanged; the hook never swallows input.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION.cast_signed() {
        let message = u32::try_from(wparam.0).unwrap_or_default();
        let press = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        if press || matches!(message, WM_KEYUP | WM_SYSKEYUP) {
            // SAFETY: reads only the foreground window; no keyboard content is collected.
            let active = unsafe { GetForegroundWindow() };
            let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
            watched.retain(|entry| {
                // SAFETY: queries the owning top-level window; a stale handle yields no matching window.
                let host = unsafe { GetAncestor(HWND(entry.host as *mut _), GA_ROOT) };
                host != active || (entry.observed)(press)
            });
        }
    }
    // SAFETY: passes the keyboard event on unchanged; only its occurrence was observed.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn start() -> Result<(), String> {
    HOOK.get_or_init(|| {
        let (ready, started) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("vw-pointer-watch".into())
            .spawn(move || {
                // SAFETY: a process-wide low-level hook installed on this thread, which then only pumps its
                // messages so Windows can call the hook; the hook procedure lives for the process.
                let installed = unsafe {
                    GetModuleHandleW(None).and_then(|module| {
                        SetWindowsHookExW(WH_MOUSE_LL, Some(hook), Some(module.into()), 0)?;
                        SetWindowsHookExW(
                            WH_KEYBOARD_LL,
                            Some(keyboard_hook),
                            Some(module.into()),
                            0,
                        )
                    })
                };
                let _ = ready.send(installed.map(drop).map_err(|e| e.to_string()));
                let mut message = MSG::default();
                // SAFETY: the standard message loop of this thread.
                unsafe {
                    while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&raw const message);
                        DispatchMessageW(&raw const message);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        started.recv().map_err(|e| e.to_string())?
    })
    .clone()
}

/// Calls `observed` for every press or release of the person's over this webview from now on.
///
/// # Errors
/// The hook or the webview's placement is unavailable.
pub fn watch(controller: &ICoreWebView2Controller, observed: Observed) -> Result<(), String> {
    start().map_err(|e| format!("pointer watch: {e}"))?;
    let (host, bounds) = place(controller)?;
    let key = controller.as_raw() as usize;
    let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
    watched.retain(|entry| entry.key != key);
    watched.push(Watched {
        key,
        host,
        bounds,
        pressed_inside: false,
        observed,
    });
    Ok(())
}

/// Takes the view's current place (it moves with the layout); called before every checked click.
pub fn refresh(controller: &ICoreWebView2Controller) {
    let key = controller.as_raw() as usize;
    if let Ok((host, bounds)) = place(controller) {
        let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
        for entry in watched.iter_mut().filter(|entry| entry.key == key) {
            (entry.host, entry.bounds) = (host, bounds);
        }
    }
}

fn down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: reads the asynchronous (physical) key state; no handles involved.
    unsafe { GetAsyncKeyState(i32::from(key.0)) }.cast_unsigned() & 0x8000 != 0
}

/// Whether a mouse button is down right now with the pointer over this webview.
#[must_use]
pub fn holding(controller: &ICoreWebView2Controller) -> bool {
    if place(controller).is_ok_and(|(host, _)| {
        // SAFETY: compares native window ownership; no keyboard data leaves this predicate.
        unsafe { GetAncestor(HWND(host as *mut _), GA_ROOT) == GetForegroundWindow() }
    }) && (8..=255).any(|key| down(VIRTUAL_KEY(key)))
    {
        return true;
    }
    if ![VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
        .into_iter()
        .any(down)
    {
        return false;
    }
    let mut at = POINT::default();
    // SAFETY: reads the cursor position into a local.
    if unsafe { GetCursorPos(&raw mut at) }.is_err() {
        // Unknown position with a button down: treat as holding.
        return true;
    }
    place(controller).is_ok_and(|(host, bounds)| over(host, &bounds, at))
}

/// The person's double-click time from Windows.
#[must_use]
pub fn recent() -> std::time::Duration {
    // SAFETY: reads a system setting.
    let ms = unsafe { GetDoubleClickTime() };
    super::recent_from(Some(std::time::Duration::from_millis(u64::from(ms))))
}
