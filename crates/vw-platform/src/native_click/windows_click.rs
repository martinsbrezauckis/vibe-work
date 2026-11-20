//! Win32 message delivery to the visible `WebView2` child HWND selected from the controller's own host window.
//! Depends on: `WebView2` controller and Win32 window APIs. Called by: `native_click` on Tauri's UI thread.

use std::sync::atomic::{AtomicBool, Ordering};

use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetClientRect, GetForegroundWindow, IsChild, IsIconic, IsWindowVisible,
    PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WindowFromPoint,
};

/// Posts a left-button click only to the browser child under a point in this controller's host `HWND`.
///
/// # Errors
/// [`super::NOT_IN_FRONT`] for a hidden, minimized or background window; refuses occluded points, missing browser
/// children and invalid native geometry.
pub fn click(
    controller: &ICoreWebView2Controller,
    point: (f64, f64),
    sent: &AtomicBool,
) -> Result<(), String> {
    let (target, packed) = target(controller, point)?;
    // SAFETY: target rechecked this live controller's visible child HWND on the UI thread.
    unsafe {
        PostMessageW(Some(target), WM_MOUSEMOVE, WPARAM(0), packed)
            .map_err(|e| format!("native mouse move: {e}"))?;
        PostMessageW(Some(target), WM_LBUTTONDOWN, WPARAM(1), packed)
            .map_err(|e| format!("Not run: native mouse press could not be queued: {e}"))?;
        sent.store(true, Ordering::Release);
        if let Err(error) = PostMessageW(Some(target), WM_LBUTTONUP, WPARAM(0), packed) {
            let recovery = PostMessageW(Some(target), WM_LBUTTONUP, WPARAM(0), packed);
            return Err(format!(
                "Native input may have been sent, so a repeat could click twice; Do not repeat: mouse release: {error}; recovery release {}",
                if recovery.is_ok() { "queued" } else { "failed" }
            ));
        }
        Ok(())
    }
}

pub(super) fn target(
    controller: &ICoreWebView2Controller,
    point: (f64, f64),
) -> Result<(HWND, LPARAM), String> {
    // SAFETY: the live controller comes from Tauri's with_webview on its UI thread. Every HWND is rechecked before
    // posting; messages target only a child of this controller's host, never the desktop or another application.
    unsafe {
        let mut host = HWND::default();
        controller
            .ParentWindow(&raw mut host)
            .map_err(|e| format!("WebView2 host window: {e}"))?;
        let root = GetAncestor(host, GA_ROOT);
        // Hidden to the tray or the pane not shown: the host is not visible; minimized: the root is iconic.
        super::require_front(
            !host.is_invalid()
                && IsWindowVisible(host).as_bool()
                && !root.is_invalid()
                && !IsIconic(root).as_bool(),
            !root.is_invalid() && root == GetForegroundWindow(),
        )?;
        let mut rect = RECT::default();
        GetClientRect(host, &raw mut rect).map_err(|e| format!("WebView2 host bounds: {e}"))?;
        let width = f64::from(rect.right - rect.left);
        let height = f64::from(rect.bottom - rect.top);
        let (x, y) = super::map_point(point, (width, height))?;
        // The mapped values are finite and strictly inside a positive HWND client rect (i32-sized pixels).
        // Flooring keeps the chosen pixel inside that rect. Win32 POINT requires i32 coordinates.
        #[allow(clippy::cast_possible_truncation)]
        let mut at = POINT {
            x: x.floor() as i32,
            y: y.floor() as i32,
        };
        if !ClientToScreen(host, &raw mut at).as_bool() {
            return Err("cannot map native preview point to screen".to_owned());
        }
        let target = WindowFromPoint(at);
        if target.is_invalid()
            || !IsChild(host, target).as_bool()
            || !IsWindowVisible(target).as_bool()
        {
            return Err("native preview point is not over its browser child".to_owned());
        }
        if !ScreenToClient(target, &raw mut at).as_bool()
            || at.x < 0
            || at.y < 0
            || at.x > i32::from(i16::MAX)
            || at.y > i32::from(i16::MAX)
        {
            return Err("native browser child point is invalid".to_owned());
        }
        let packed = LPARAM(((at.y as isize) << 16) | (at.x as isize & 0xffff));
        Ok((target, packed))
    }
}
