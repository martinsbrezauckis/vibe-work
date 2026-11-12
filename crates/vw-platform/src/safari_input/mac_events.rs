//! CoreGraphics events with bounded chords, clicks and drag; held keys/buttons are released on cancellation.
use super::geometry::Point;
use serde_json::Value;
use std::{ffi::c_void, ptr};
type Ref = *const c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Location {
    x: f64,
    y: f64,
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateMouseEvent(source: Ref, kind: u32, point: Location, button: u32) -> Ref;
    fn CGEventCreateScrollWheelEvent2(
        source: Ref,
        units: u32,
        count: u32,
        vertical: i32,
        horizontal: i32,
        depth: i32,
    ) -> Ref;
    fn CGEventCreateKeyboardEvent(source: Ref, key: u16, down: bool) -> Ref;
    fn CGEventKeyboardSetUnicodeString(event: Ref, length: usize, text: *const u16);
    fn CGEventSetFlags(event: Ref, flags: u64);
    fn CGEventSetIntegerValueField(event: Ref, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: Ref);
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
}
struct Event(Ref);
impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
fn event(p: Ref) -> Result<Event, String> {
    if p.is_null() {
        Err(
            "macOS could not create the native input event. Read the page before trying again."
                .into(),
        )
    } else {
        Ok(Event(p))
    }
}
fn key(code: u16, down: bool, flags: u64, text: &[u16]) -> Result<(), String> {
    // SAFETY: CoreGraphics creates an owned event; UTF-16 data lives until its copying setter returns.
    let e = event(unsafe { CGEventCreateKeyboardEvent(ptr::null(), code, down) })?;
    unsafe {
        CGEventSetFlags(e.0, flags);
        if !text.is_empty() {
            CGEventKeyboardSetUnicodeString(e.0, text.len(), text.as_ptr());
        }
        CGEventPost(0, e.0);
    }
    Ok(())
}
fn mouse(kind: u32, p: Point, button: u32, count: i64, flags: u64) -> Result<(), String> {
    // SAFETY: finite in-window coordinates were proved by the portable mapper; event owns no borrowed memory.
    let e = event(unsafe {
        CGEventCreateMouseEvent(ptr::null(), kind, Location { x: p.x, y: p.y }, button)
    })?;
    unsafe {
        CGEventSetFlags(e.0, flags);
        CGEventSetIntegerValueField(e.0, 1, count);
        CGEventPost(0, e.0);
    }
    Ok(())
}
struct Held {
    keys: Vec<u16>,
    button: Option<(Point, u32)>,
    flags: u64,
}
impl Drop for Held {
    fn drop(&mut self) {
        if let Some((p, b)) = self.button {
            let _ = mouse(
                if b == 0 {
                    2
                } else if b == 1 {
                    4
                } else {
                    26
                },
                p,
                b,
                1,
                0,
            );
        }
        for k in self.keys.iter().rev() {
            let _ = key(*k, false, 0, &[]);
        }
    }
}
pub(super) fn dispatch(
    action: &str,
    args: &Value,
    point: Point,
    end: Option<Point>,
    check: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let mut held = Held {
        keys: Vec::new(),
        button: None,
        flags: 0,
    };
    let plan = super::super::plan::plan(action, args)?;
    if plan.layout {
        super::ax::require_us_layout()?;
    }
    check()?;
    mouse(5, point, 0, 0, 0)?;
    for (code, flag) in plan.modifiers {
        check()?;
        held.flags |= flag;
        held.keys.push(code);
        key(code, true, held.flags, &[])?;
    }
    if let Some(code) = plan.key {
        check()?;
        held.keys.push(code);
        key(code, true, held.flags, &plan.text)?;
        key(code, false, held.flags, &plan.text)?;
        held.keys.pop();
        return Ok(());
    }
    if let Some((horizontal, vertical)) = plan.wheel {
        check()?;
        // SAFETY: bounded signed pixel deltas, two axes and an owned CoreGraphics event.
        let e = event(unsafe {
            CGEventCreateScrollWheelEvent2(ptr::null(), 0, 2, vertical, horizontal, 0)
        })?;
        unsafe {
            CGEventSetFlags(e.0, held.flags);
            CGEventPost(0, e.0);
        }
        return Ok(());
    }
    if plan.motion {
        return Ok(());
    }
    let button = plan.button;
    let down = if button == 0 {
        1
    } else if button == 1 {
        3
    } else {
        25
    };
    let up = if button == 0 {
        2
    } else if button == 1 {
        4
    } else {
        26
    };
    let count = plan.clicks;
    for click in 1..=count {
        check()?;
        held.button = Some((point, button));
        mouse(down, point, button, click, held.flags)?;
        if plan.drag {
            let end =
                end.ok_or("Native drag has no checked destination. Read both target references.")?;
            for i in 1..=20 {
                check()?;
                let fraction = f64::from(i) / 20.;
                let p = Point {
                    x: point.x + (end.x - point.x) * fraction,
                    y: point.y + (end.y - point.y) * fraction,
                };
                held.button = Some((p, button));
                mouse(6, p, button, 1, held.flags)?;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let p = held.button.map_or(point, |(p, _)| p);
        mouse(up, p, button, click, held.flags)?;
        held.button = None;
        if click < count {
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    }
    Ok(())
}
