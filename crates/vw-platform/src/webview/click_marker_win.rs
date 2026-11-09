//! Windows click feedback: a layered, input-transparent popup owned by the app's main HWND.
//! Depends on Win32/GDI and WebView2; vw-webview calls `show` on the UI thread after input.
//! No page evaluation, capture compositing or global pointer hook. Owner destruction owns cleanup.

use std::{
    cell::RefCell,
    collections::HashMap,
    time::{Duration, Instant},
};
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller;
use windows::{
    Win32::{
        Foundation::{
            COLORREF, ERROR_CLASS_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HWND, LPARAM,
            LRESULT, POINT, RECT, SIZE, SetLastError, WPARAM,
        },
        Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            ClientToScreen, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
            DeleteObject, HGDIOBJ, SelectObject,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GA_ROOT, GetAncestor,
                HTTRANSPARENT, HWND_TOP, IsIconic, IsWindowVisible, KillTimer, MA_NOACTIVATE,
                RegisterClassW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer, SetWindowPos, ULW_ALPHA,
                UpdateLayeredWindow, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_NCHITTEST, WM_TIMER,
                WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
                WS_POPUP,
            },
        },
    },
    core::w,
};

#[path = "click_marker_win_calls.rs"]
mod calls;
use super::click_marker_win_math::{Position, position};
use calls::{error_text, win32};

const CLASS: windows::core::PCWSTR = w!("VibeWorkClickMarker");
struct Marker {
    controller: ICoreWebView2Controller,
    parent: HWND,
    owner: HWND,
    origin: POINT,
    bounds: RECT,
    until: Instant,
    dpi: u32,
}
thread_local! {
    static MARKERS: RefCell<HashMap<isize, Marker>> = RefCell::default();
}

/// Shows the macOS-style 28 point ring for 700 ms at a normalized viewport point.
///
/// # Errors
/// Invalid geometry, a hidden view, or a failed native window/bitmap operation.
pub fn show(controller: &ICoreWebView2Controller, point: (f64, f64)) -> Result<(), String> {
    show_for(controller, point, Duration::from_millis(700))
}

/// Paints the shared action marker for its remaining host-clock lifetime.
///
/// # Errors
/// Invalid geometry, a hidden view, or a failed native window/bitmap operation.
pub fn show_for(
    controller: &ICoreWebView2Controller,
    point: (f64, f64),
    remaining: Duration,
) -> Result<(), String> {
    if remaining.is_zero() {
        return Ok(());
    }
    // SAFETY: the controller is supplied on its UI thread; all out pointers are live stack values.
    unsafe { show_inner(controller, point, remaining) }
        .map_err(|e| format!("drawing the click marker: {e}"))
}

unsafe fn show_inner(
    controller: &ICoreWebView2Controller,
    point: (f64, f64),
    remaining: Duration,
) -> Result<(), String> {
    let mut parent = HWND::default();
    let mut bounds = RECT::default();
    let mut visible = windows::core::BOOL::default();
    // SAFETY: called on the controller's UI thread; output values remain live across each call.
    unsafe {
        controller
            .ParentWindow(&raw mut parent)
            .map_err(|e| e.to_string())?;
        controller
            .Bounds(&raw mut bounds)
            .map_err(|e| e.to_string())?;
        controller
            .IsVisible(&raw mut visible)
            .map_err(|e| e.to_string())?;
        if !visible.as_bool() || !IsWindowVisible(parent).as_bool() {
            return Err("the preview is not visible".into());
        }
        let (x, y) = crate::native_click::map_point(
            point,
            (
                f64::from(bounds.right - bounds.left),
                f64::from(bounds.bottom - bounds.top),
            ),
        )?;
        let dpi = GetDpiForWindow(parent);
        if dpi == 0 {
            return Err("the preview has no display scale".into());
        }
        let scale = f64::from(dpi) / 96.0;
        let mut origin = POINT::default();
        win32("ClientToScreen", || {
            ClientToScreen(parent, &raw mut origin).ok()
        })?;
        let position = position((origin.x, origin.y), (bounds.left, bounds.top), (x, y), dpi);
        let owner = GetAncestor(parent, GA_ROOT);
        if owner.is_invalid() || !IsWindowVisible(owner).as_bool() || IsIconic(owner).as_bool() {
            return Err("the preview has no visible app window".into());
        }
        let window = create_popup(parent, owner, &position)?;
        let result = paint(
            window,
            &position,
            (position.left - origin.x, position.top - origin.y),
            scale,
            bounds,
        );
        if let Err(error) = result {
            let _ = DestroyWindow(window);
            return Err(error);
        }
        MARKERS.with(|markers| {
            markers.borrow_mut().insert(
                window.0 as isize,
                Marker {
                    controller: controller.clone(),
                    parent,
                    owner,
                    origin,
                    bounds,
                    until: Instant::now() + remaining.min(Duration::from_millis(700)),
                    dpi,
                },
            )
        });
        if let Err(error) = win32("SetTimer", || {
            if SetTimer(Some(window), 1, 16, None) == 0 {
                Err(windows::core::Error::empty())
            } else {
                Ok(())
            }
        }) {
            let _ = DestroyWindow(window);
            return Err(error);
        }
        let result = win32("SetWindowPos", || {
            SetWindowPos(
                window,
                Some(HWND_TOP),
                position.left,
                position.top,
                position.side,
                position.side,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        });
        if let Err(error) = result {
            let _ = DestroyWindow(window);
            return Err(error);
        }
    }
    Ok(())
}

unsafe fn create_popup(parent: HWND, owner: HWND, position: &Position) -> Result<HWND, String> {
    // SAFETY: the owner and host HWNDs are live on this UI thread; the registered procedure owns cleanup.
    unsafe {
        let instance = win32("GetModuleHandleW", || GetModuleHandleW(None))?;
        SetLastError(ERROR_SUCCESS);
        let class = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        });
        let class_error = GetLastError();
        if class == 0 && class_error != ERROR_CLASS_ALREADY_EXISTS {
            return Err(error_text("RegisterClassW", class_error));
        }
        // Owned popups are not children, so FindWindowEx(parent) cannot find the previous ring.
        // Release the map borrow before destruction reenters window_proc for WM_NCDESTROY.
        let old: Vec<_> = MARKERS.with(|markers| {
            markers
                .borrow()
                .iter()
                .filter_map(|(&handle, marker)| {
                    (marker.parent == parent).then_some(HWND(handle as *mut _))
                })
                .collect()
        });
        for window in old {
            let _ = DestroyWindow(window);
        }
        win32("CreateWindowExW", || {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                CLASS,
                w!(""),
                WS_POPUP,
                position.left,
                position.top,
                position.side,
                position.side,
                Some(owner),
                None,
                Some(instance.into()),
                None,
            )
        })
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: Windows calls this procedure on the creating thread for a live HWND.
    unsafe {
        match message {
            WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
            WM_MOUSEACTIVATE => return LRESULT(isize::try_from(MA_NOACTIVATE).unwrap_or(3)),
            WM_TIMER => {
                let valid = MARKERS.with(|markers| {
                    markers
                        .borrow()
                        .get(&(window.0 as isize))
                        .is_some_and(|marker| {
                            let mut visible = windows::core::BOOL::default();
                            let mut bounds = RECT::default();
                            let mut origin = POINT::default();
                            Instant::now() < marker.until
                                && IsWindowVisible(marker.parent).as_bool()
                                && IsWindowVisible(marker.owner).as_bool()
                                && !IsIconic(marker.owner).as_bool()
                                && GetDpiForWindow(marker.parent) == marker.dpi
                                && ClientToScreen(marker.parent, &raw mut origin).as_bool()
                                && origin == marker.origin
                                && marker.controller.IsVisible(&raw mut visible).is_ok()
                                && visible.as_bool()
                                && marker.controller.Bounds(&raw mut bounds).is_ok()
                                && bounds == marker.bounds
                        })
                });
                if !valid {
                    let _ = DestroyWindow(window);
                }
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                let _ = KillTimer(Some(window), 1);
                MARKERS.with(|markers| markers.borrow_mut().remove(&(window.0 as isize)));
            }
            _ => {}
        }
        DefWindowProcW(window, message, w, l)
    }
}

unsafe fn paint(
    window: HWND,
    position: &Position,
    (left, top): (i32, i32),
    scale: f64,
    bounds: RECT,
) -> Result<(), String> {
    // SAFETY: bounded 32-bit top-down DIB. Every allocated GDI object is restored/deleted below,
    // including on UpdateLayeredWindow failure; pixels are copied only into this owned bitmap.
    unsafe {
        let side = position.side;
        let dc = win32("CreateCompatibleDC", || {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                Err(windows::core::Error::empty())
            } else {
                Ok(dc)
            }
        })?;
        let mut bits = std::ptr::null_mut();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(40),
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let bitmap = match win32("CreateDIBSection", || {
            CreateDIBSection(
                Some(dc),
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )
        }) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = DeleteDC(dc);
                return Err(error);
            }
        };
        let old = match win32("SelectObject", || {
            let old = SelectObject(dc, HGDIOBJ(bitmap.0));
            if old.is_invalid() {
                Err(windows::core::Error::empty())
            } else {
                Ok(old)
            }
        }) {
            Ok(old) => old,
            Err(error) => {
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(dc);
                return Err(error);
            }
        };
        let pixels = ring_pixels(side, scale, left, top, bounds);
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        let origin = POINT {
            x: position.left,
            y: position.top,
        };
        let result = win32("UpdateLayeredWindow", || {
            UpdateLayeredWindow(
                window,
                None,
                Some(&raw const origin),
                Some(&SIZE { cx: side, cy: side }),
                Some(dc),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&BLENDFUNCTION {
                    BlendOp: u8::try_from(AC_SRC_OVER).unwrap_or(0),
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: u8::try_from(AC_SRC_ALPHA).unwrap_or(1),
                }),
                ULW_ALPHA,
            )
        });
        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        result
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ring_pixels(side: i32, scale: f64, left: i32, top: i32, bounds: RECT) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let distance = (f64::from(x) + 0.5 - f64::from(side) / 2.0)
                .hypot(f64::from(y) + 0.5 - f64::from(side) / 2.0);
            let outer = (13.5 * scale - distance + 0.5).clamp(0.0, 1.0);
            let stroke = (distance - 10.5 * scale + 0.5).clamp(0.0, 1.0);
            let alpha = if (bounds.left..bounds.right).contains(&(left + x))
                && (bounds.top..bounds.bottom).contains(&(top + y))
            {
                (outer * (0.25 + 0.75 * stroke) * 255.0).round() as u8
            } else {
                0
            };
            // Premultiplied BGRA: same blue and quarter-alpha fill as the AppKit marker.
            pixels.extend_from_slice(&[alpha, (u16::from(alpha) * 122 / 255) as u8, 0, alpha]);
        }
    }
    pixels
}
