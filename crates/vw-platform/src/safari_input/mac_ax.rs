//! Owned Accessibility values and bounded Safari window/web-area discovery. Called only on the native-input worker.
//! SAFETY: all CF/AX references stay on that worker, copy/create results are released once, borrowed values are retained,
//! and type IDs precede array/string/value access. C strings and output storage remain live through each synchronous call.
use super::geometry::{Rect, same_window as same, window_index};
use std::{
    ffi::{CString, c_void},
    ptr,
};
type Ref = *const c_void;
#[repr(C)]
#[derive(Default)]
struct Pair {
    x: f64,
    y: f64,
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: Ref) -> u8;
    static kAXTrustedCheckOptionPrompt: Ref;
    fn AXUIElementCreateApplication(pid: i32) -> Ref;
    fn AXUIElementCopyAttributeValue(el: Ref, key: Ref, value: *mut Ref) -> i32;
    fn AXUIElementSetAttributeValue(el: Ref, key: Ref, value: Ref) -> i32;
    fn AXUIElementPerformAction(el: Ref, action: Ref) -> i32;
    fn AXUIElementSetMessagingTimeout(el: Ref, seconds: f32) -> i32;
    fn AXValueGetValue(value: Ref, kind: i32, out: *mut c_void) -> u8;
    fn AXValueGetTypeID() -> usize;
    fn AXUIElementGetTypeID() -> usize;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFRetain(value: Ref) -> Ref;
    fn CFEqual(a: Ref, b: Ref) -> u8;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFURLGetTypeID() -> usize;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFStringCreateWithCString(allocator: Ref, text: *const i8, encoding: u32) -> Ref;
    fn CFStringGetCString(text: Ref, buffer: *mut i8, size: isize, encoding: u32) -> u8;
    fn CFURLGetString(url: Ref) -> Ref;
    static kCFBooleanTrue: Ref;
}
pub(super) struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
fn string(value: &str) -> Result<Owned, String> {
    let c = CString::new(value).map_err(|_| failure())?;
    let p = unsafe { CFStringCreateWithCString(ptr::null(), c.as_ptr(), 0x0800_0100) };
    if p.is_null() {
        Err(failure())
    } else {
        Ok(Owned(p))
    }
}
fn failure() -> String {
    "Safari Accessibility did not confirm the checked window and page. Keep Safari visible, then read the page again.".into()
}
impl Owned {
    fn attr(&self, key: &str) -> Result<Self, String> {
        let key = string(key)?;
        let mut result = ptr::null();
        if unsafe { AXUIElementCopyAttributeValue(self.0, key.0, &raw mut result) } != 0
            || result.is_null()
        {
            return Err(failure());
        }
        Ok(Self(result))
    }
    fn text(&self) -> Option<String> {
        let mut value = self.0;
        unsafe {
            if CFGetTypeID(value) == CFURLGetTypeID() {
                value = CFURLGetString(value);
            }
            if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
                return None;
            }
            let mut bytes = vec![0i8; 8193];
            if CFStringGetCString(value, bytes.as_mut_ptr(), 8193, 0x0800_0100) == 0 {
                return None;
            }
            Some(
                std::ffi::CStr::from_ptr(bytes.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }
    fn list(&self) -> Result<Vec<Self>, String> {
        unsafe {
            if CFGetTypeID(self.0) != CFArrayGetTypeID() {
                return Err(failure());
            }
            let n = CFArrayGetCount(self.0);
            if !(0..=2048).contains(&n) {
                return Err(failure());
            }
            (0..n)
                .map(|i| {
                    let p = CFArrayGetValueAtIndex(self.0, i);
                    if p.is_null() || CFGetTypeID(p) != AXUIElementGetTypeID() {
                        Err(failure())
                    } else {
                        Ok(Self(CFRetain(p)))
                    }
                })
                .collect()
        }
    }
    fn pair(&self, key: &str, kind: i32) -> Result<Pair, String> {
        let v = self.attr(key)?;
        let mut p = Pair::default();
        if unsafe {
            CFGetTypeID(v.0) != AXValueGetTypeID()
                || AXValueGetValue(v.0, kind, (&raw mut p).cast()) == 0
        } {
            return Err(failure());
        }
        Ok(p)
    }
    pub(super) fn rect(&self) -> Result<Rect, String> {
        let p = self.pair("AXPosition", 1)?;
        let s = self.pair("AXSize", 2)?;
        Ok(Rect {
            x: p.x,
            y: p.y,
            width: s.x,
            height: s.y,
        })
    }
}
/// Ask macOS whether the app is trusted, showing the system Accessibility prompt when it is not.
/// The prompt only offers System Settings; it never grants or removes permission by itself.
/// Called only when Safari pairing completes; agent actions use the read-only check in [`Window::discover`].
/// Unlike the window code it holds no AX element, only its own CF values, so any thread may call it.
pub(super) fn request_accessibility() -> bool {
    use core_foundation::{
        base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
    };
    // SAFETY: read-only TCC query, no prompt or permission change.
    if unsafe { AXIsProcessTrusted() } != 0 {
        return true;
    }
    // SAFETY: the key is a framework-owned constant string; wrap_under_get_rule retains it for the dictionary.
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt.cast()) };
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    // SAFETY: the options dictionary stays alive through this synchronous call.
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast()) != 0 }
}
pub(super) struct Window {
    app: Owned,
    element: Owned,
    bounds: Rect,
}
impl Window {
    pub(super) fn discover(bounds: Rect) -> Result<Self, String> {
        // SAFETY: read-only TCC query, no prompt or permission change.
        super::super::lifecycle::require_accessibility(|| unsafe { AXIsProcessTrusted() } != 0)?;
        let apps = objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &objc2_foundation::NSString::from_str("com.apple.Safari"),
        );
        if apps.len() != 1 {
            return Err("Safari's running application is ambiguous or missing. Open one Safari application, then read the page again.".into());
        }
        let pid = apps.objectAtIndex(0).processIdentifier();
        if pid <= 0 {
            return Err(failure());
        }
        // SAFETY: pid is the OS-provided Safari process; returned owned references are released on this worker.
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        if app.0.is_null() {
            return Err(failure());
        }
        unsafe {
            AXUIElementSetMessagingTimeout(app.0, 0.2);
        }
        let mut windows = app.attr("AXWindows")?.list()?;
        let bounds_list = windows
            .iter()
            .map(Owned::rect)
            .collect::<Result<Vec<_>, _>>()?;
        let index = window_index(&bounds_list, bounds)?;
        let window = windows.remove(index);
        Ok(Self {
            app,
            element: window,
            bounds,
        })
    }
    pub(super) fn activate(&self, live: &dyn Fn() -> Result<(), String>) -> Result<(), String> {
        let raise = string("AXRaise")?;
        let front = string("AXFrontmost")?;
        live()?;
        if unsafe { AXUIElementPerformAction(self.element.0, raise.0) } != 0 {
            return Err(failure());
        }
        live()?;
        if unsafe { AXUIElementSetAttributeValue(self.app.0, front.0, kCFBooleanTrue) } != 0 {
            return Err(failure());
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while self.front().is_err() {
            live()?;
            if std::time::Instant::now() >= until {
                return Err(failure());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Ok(())
    }
    pub(super) fn front(&self) -> Result<(), String> {
        let focused = self.app.attr("AXFocusedWindow")?;
        let front = self.app.attr("AXFrontmost")?;
        if unsafe {
            CFEqual(focused.0, self.element.0) == 0 || CFEqual(front.0, kCFBooleanTrue) == 0
        } || !same(self.element.rect()?, self.bounds)
        {
            return Err("Safari's checked window moved or lost focus. Read the page again before native input.".into());
        }
        Ok(())
    }
    pub(super) fn bounds(&self) -> Rect {
        self.bounds
    }
    pub(super) fn viewport(&self, url: &str) -> Result<Rect, String> {
        self.front()?;
        let mut queue = vec![(self.element.attr("AXChildren")?.list()?, None, 0)];
        let mut visited = 0;
        let mut found = Vec::new();
        while let Some((nodes, parent, depth)) = queue.pop() {
            for node in nodes {
                visited += 1;
                if visited > 2048 || depth > 16 {
                    return Err(failure());
                }
                let role = node
                    .attr("AXRole")
                    .ok()
                    .and_then(|v| v.text())
                    .unwrap_or_default();
                let scroll = if role == "AXScrollArea" {
                    Some(node.rect()?)
                } else {
                    parent
                };
                if role == "AXWebArea" {
                    if node.attr("AXURL").ok().and_then(|v| v.text()).as_deref() == Some(url) {
                        found.push(scroll.ok_or_else(failure)?);
                    }
                    continue;
                }
                if let Ok(children) = node.attr("AXChildren") {
                    queue.push((children.list()?, scroll, depth + 1));
                }
            }
        }
        if found.len() != 1 {
            return Err("Safari did not expose one matching visible web area through Accessibility. Keep the checked page active and read it again.".into());
        }
        Ok(found[0])
    }
}

// SAFETY: TIS returns a retained input-source object; its property is borrowed only while that object lives.
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> Ref;
    fn TISGetInputSourceProperty(source: Ref, key: Ref) -> Ref;
    static kTISPropertyInputSourceID: Ref;
}
// libdispatch (libSystem): `dispatch_get_main_queue()` is the address of `_dispatch_main_q`.
unsafe extern "C" {
    static _dispatch_main_q: u8;
    fn dispatch_async_f(queue: *const u8, context: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> i32;
}
/// The current keyboard layout's input source id; `Err` when macOS returned no source.
/// Main thread only: TIS/TSM asserts its queue (`dispatch_assert_queue`) on macOS 27 and aborts the whole app.
fn layout_id() -> Result<Option<String>, ()> {
    let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
    if source.is_null() {
        return Err(());
    }
    let source = Owned(source);
    let value = unsafe { TISGetInputSourceProperty(source.0, kTISPropertyInputSourceID) };
    Ok(if value.is_null() {
        None
    } else {
        Owned(unsafe { CFRetain(value) }).text()
    })
}
type LayoutReply = std::sync::mpsc::Sender<Result<Option<String>, ()>>;
extern "C" fn layout_on_main(context: *mut c_void) {
    // SAFETY: `context` is the Box leaked by `main_layout_id` for exactly this one call.
    let reply = unsafe { Box::from_raw(context.cast::<LayoutReply>()) };
    let _ = reply.send(layout_id());
}
/// Reads the layout on the main queue. The Safari input worker waits with a deadline, so a busy main thread
/// produces a retryable error, never a deadlock (asynchronous hop: the worker never blocks the main queue).
fn main_layout_id() -> Result<Option<String>, String> {
    if unsafe { pthread_main_np() } == 1 {
        return layout_id().map_err(|()| failure());
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<Option<String>, ()>>();
    let context = Box::into_raw(Box::new(tx)).cast::<c_void>();
    // SAFETY: the main queue is a process-lifetime static; `layout_on_main` frees `context` once.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, context, layout_on_main) };
    match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(id) => id.map_err(|()| failure()),
        Err(_) => {
            Err("macOS did not report the keyboard layout in time. Retry the shortcut.".into())
        }
    }
}
pub(super) fn require_us_layout() -> Result<(), String> {
    let id = main_layout_id()?;
    if !matches!(
        id.as_deref(),
        Some("com.apple.keylayout.US" | "com.apple.keylayout.ABC")
    ) {
        return Err("The current macOS keyboard layout cannot be mapped safely for this shortcut. Switch the input source to ABC or U.S., then retry the shortcut.".into());
    }
    Ok(())
}
#[cfg(test)]
mod layout_tests {
    // A worker thread must never call TIS itself (macOS 27 aborts the app). The test harness never
    // drains the main queue, so the hop has to come back as the retryable timeout, not as a crash.
    #[test]
    fn worker_thread_hops_to_the_main_queue() {
        let result = std::thread::spawn(super::require_us_layout)
            .join()
            .expect("worker thread survived");
        assert_eq!(
            result.unwrap_err(),
            "macOS did not report the keyboard layout in time. Retry the shortcut."
        );
    }
}
