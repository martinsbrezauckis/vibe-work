//! Silence libproxy's credential-bearing backend diagnostics while querying. Used only by the Linux adapter.
//! The handler is scoped to pxbackend, restored on drop, and serialized by the caller. No app/GTK log is changed.
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
static FAILED: AtomicBool = AtomicBool::new(false);
type Remove = unsafe extern "C" fn(*const c_char, u32);
type Callback = unsafe extern "C" fn(*const c_char, u32, *const c_char, *mut c_void);
type Install = unsafe extern "C" fn(*const c_char, u32, Option<Callback>, *mut c_void) -> u32;

pub(super) struct Quiet<'a> {
    failed: &'static AtomicBool,
    remove: libloading::Symbol<'a, Remove>,
    id: u32,
}
impl<'a> Quiet<'a> {
    pub(super) fn new(glib: &'a libloading::Library) -> Result<Self, &'static str> {
        // SAFETY: these are GLib's stable log API signatures. The library remains loaded through Drop; our
        // callback never dereferences foreign text, and the handler receives no borrowed user data.
        unsafe {
            let install: libloading::Symbol<Install> = glib
                .get(b"g_log_set_handler\0")
                .map_err(|_| "Proxy logging could not be protected.")?;
            let remove = glib
                .get(b"g_log_remove_handler\0")
                .map_err(|_| "Proxy logging could not be protected.")?;
            // All levels and fatal/recursion flags. libproxy 0.5 uses ordinary GLib logging in this domain.
            FAILED.store(false, Ordering::Release);
            let id = install(
                c"pxbackend".as_ptr(),
                0xff,
                Some(discard),
                std::ptr::null_mut(),
            );
            if id == 0 {
                return Err("Proxy logging could not be protected.");
            }
            Ok(Self {
                failed: &FAILED,
                remove,
                id,
            })
        }
    }
}
unsafe extern "C" fn discard(_: *const c_char, level: u32, _: *const c_char, _: *mut c_void) {
    // Some libproxy releases return DIRECT after a failed PAC download. Preserve native warning/error signals
    // without ever reading their credential-bearing text; the adapter converts them to a lookup failure.
    if level & 0x1c != 0 {
        FAILED.store(true, Ordering::Release);
    }
}
impl Quiet<'_> {
    pub(super) fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
}
impl Drop for Quiet<'_> {
    fn drop(&mut self) {
        // SAFETY: the handler belongs to this object and the library still lives.
        unsafe {
            (self.remove)(c"pxbackend".as_ptr(), self.id);
        }
    }
}
