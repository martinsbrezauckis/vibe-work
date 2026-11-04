//! libproxy 0.5.5+ stable C API. Loads the installed library, whose backends read env/GNOME/KDE/PAC.
//! No process execution or JS evaluation here. Missing library/backend errors fail closed; use Settings manually.
use std::ffi::{CStr, CString, c_char, c_void};

static LIBRARY: std::sync::OnceLock<Result<libloading::Library, &'static str>> =
    std::sync::OnceLock::new();
static CALL: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn resolve(url: &str) -> Result<Vec<String>, &'static str> {
    let url = CString::new(url).map_err(|_| "Invalid proxy lookup URL.")?;
    let _call = CALL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: symbols and signatures are the stable libproxy C API. Library stays loaded until the returned
    // array and factory are freed with their own allocator. Every native string is copied before free.
    unsafe {
        // Native GLib types can outlive an individual factory; keep their library loaded for the process.
        let library = LIBRARY
            .get_or_init(|| {
                libloading::Library::new("libproxy.so.1")
                    .map_err(|_| "Install libproxy or set a manual proxy.")
            })
            .as_ref()
            .map_err(|e| *e)?;
        // libproxy 0.4 has different diagnostic behavior; require the 0.5.5 API marker, then protect logging
        // before any factory code can print PAC URLs or proxy credentials (even with PX_DEBUG enabled).
        let _: libloading::Symbol<*const c_void> = library
            .get(b"px_proxy_factory_get_type\0")
            .map_err(|_| "Install libproxy 0.5.5 or set a manual proxy.")?;
        let glib = libloading::Library::new("libglib-2.0.so.0")
            .map_err(|_| "Proxy logging could not be protected.")?;
        let quiet = super::linux_log::Quiet::new(&glib)?;
        let new: libloading::Symbol<unsafe extern "C" fn() -> *mut c_void> = library
            .get(b"px_proxy_factory_new\0")
            .map_err(|_| "libproxy API unavailable.")?;
        let free: libloading::Symbol<unsafe extern "C" fn(*mut c_void)> = library
            .get(b"px_proxy_factory_free\0")
            .map_err(|_| "libproxy API unavailable.")?;
        let get: libloading::Symbol<
            unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut *mut c_char,
        > = library
            .get(b"px_proxy_factory_get_proxies\0")
            .map_err(|_| "libproxy API unavailable.")?;
        let free_proxies: libloading::Symbol<unsafe extern "C" fn(*mut *mut c_char)> = library
            .get(b"px_proxy_factory_free_proxies\0")
            .map_err(|_| "libproxy API unavailable.")?;
        let factory = new();
        if factory.is_null() {
            return Err("libproxy initialization failed.");
        }
        let proxies = get(factory, url.as_ptr());
        let mut out = Vec::new();
        let result = if proxies.is_null() {
            Err("System proxy resolution failed.")
        } else {
            for i in 0..=32 {
                let value = *proxies.add(i);
                if value.is_null() {
                    break;
                }
                out.push(CStr::from_ptr(value).to_string_lossy().into_owned());
            }
            free_proxies(proxies);
            if out.is_empty() || out.len() > 32 {
                Err("System proxy result is invalid.")
            } else {
                Ok(out)
            }
        };
        free(factory);
        if quiet.failed() {
            Err("System proxy resolution failed.")
        } else {
            result
        }
    }
}
