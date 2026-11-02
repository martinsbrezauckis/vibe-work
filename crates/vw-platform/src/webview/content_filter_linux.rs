//! Safe `WebKitGTK` content-filter installation for the preview engine, on GTK's main thread.
//! Bridges the filter-store API missing from webkit2gtk-rs; compilation completes before replacement.
use glib::translate::{FromGlibPtrFull, ToGlibPtr, from_glib_full};
use std::path::Path;
use webkit2gtk::{UserContentManager, UserContentManagerExt as _, WebView, WebViewExt as _, ffi};

type Done = Box<dyn FnOnce(Result<(), String>)>;
struct Pending {
    manager: UserContentManager,
    done: Done,
    install_if: Box<dyn Fn() -> bool>,
}

unsafe extern "C" fn compiled(
    store: *mut glib::gobject_ffi::GObject,
    result: *mut webkit2gtk::gio::ffi::GAsyncResult,
    data: glib::ffi::gpointer,
) {
    // SAFETY: save owns this Box until this one completion callback; GIO supplies its store and result.
    let pending = unsafe { Box::from_raw(data.cast::<Pending>()) };
    let mut error = std::ptr::null_mut();
    let filter = unsafe {
        ffi::webkit_user_content_filter_store_save_finish(store.cast(), result, &raw mut error)
    };
    if filter.is_null() {
        let message = if error.is_null() {
            "content filter compilation failed".into()
        } else {
            // SAFETY: finish transfers its GError to us on failure.
            unsafe { glib::Error::from_glib_full(error) }.to_string()
        };
        (pending.done)(Err(message));
        return;
    }
    if !(pending.install_if)() {
        // SAFETY: finish transferred a reference, even when this operation was superseded.
        unsafe {
            ffi::webkit_user_content_filter_unref(filter);
        }
        (pending.done)(Err("ad filter update was superseded".into()));
        return;
    }
    pending.manager.remove_filter_by_id("vibe-work-shields");
    // SAFETY: filter is owned from finish; add_filter takes a reference and does not take ownership.
    unsafe {
        ffi::webkit_user_content_manager_add_filter(pending.manager.to_glib_none().0, filter);
        ffi::webkit_user_content_filter_unref(filter);
    }
    (pending.done)(Ok(()));
}

/// Replaces only the browser's ad filter; never removes navigation/network-policy rules.
/// Called on the UI thread. `None` removes ad filtering. Completion fires once, including compile errors.
pub fn set_content_filter(
    view: &WebView,
    directory: &Path,
    rules: Option<&str>,
    install_if: Box<dyn Fn() -> bool>,
    done: Done,
) {
    if !install_if() {
        done(Err("ad filter update was superseded".into()));
        return;
    }
    let Some(manager) = view.user_content_manager() else {
        done(Err("page has no content manager".into()));
        return;
    };
    let Some(rules) = rules.filter(|rules| *rules != "[]") else {
        manager.remove_filter_by_id("vibe-work-shields");
        done(Ok(()));
        return;
    };
    let source = glib::Bytes::from_owned(rules.as_bytes().to_vec());
    // SAFETY: the store constructor copies its path; glib::Object owns its returned reference.
    let store: glib::Object = unsafe {
        from_glib_full(ffi::webkit_user_content_filter_store_new(directory.to_glib_none().0).cast())
    };
    let pending = Box::new(Pending {
        manager,
        done,
        install_if,
    });
    let store_ptr: *mut glib::gobject_ffi::GObject = store.to_glib_none().0;
    // SAFETY: GIO retains store and source until completion; only compiled reclaims pending.
    unsafe {
        ffi::webkit_user_content_filter_store_save(
            store_ptr.cast(),
            c"vibe-work-shields".as_ptr(),
            source.to_glib_none().0,
            std::ptr::null_mut(),
            Some(compiled),
            Box::into_raw(pending).cast(),
        );
    }
}
