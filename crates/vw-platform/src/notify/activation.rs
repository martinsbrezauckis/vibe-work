//! Installed Windows toast COM activation, including clicks in Notification Center after the banner.
//! Uses a per-user COM local server and `AppUserModelId` registration; called by the notifier at startup.
// windows::implement emits these forms in the generated COM vtables.
#![allow(clippy::ref_as_ptr, clippy::inline_always)]
use std::{ffi::c_void, sync::OnceLock};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoInitializeEx, CoRegisterClassObject,
    IClassFactory, IClassFactory_Impl, REGCLS_MULTIPLEUSE,
};
use windows::Win32::UI::Notifications::{
    INotificationActivationCallback, INotificationActivationCallback_Impl,
    NOTIFICATION_USER_INPUT_DATA,
};
use windows::core::{BOOL, GUID, IUnknown, Interface, PCWSTR, Ref, Result, implement};
use windows_registry::CURRENT_USER;

// Owned solely by Vibe Work; the uninstaller removes these registration keys.
const CLASS: GUID = GUID::from_u128(0xe9d28628_47a6_45a7_80c2_2bdc64107a50);
const CLASS_TEXT: &str = "{E9D28628-47A6-45A7-80C2-2BDC64107A50}";

pub(super) fn set_icon(app_id: &str, path: &std::path::Path) -> Result<()> {
    // IconUri must name an image file. An executable's icon resource and a shortcut's
    // AppUserModelID do not populate the notification header for an unpackaged app.
    let key = CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{app_id}"))?;
    key.set_string("IconUri", path.to_string_lossy())
}

#[implement(INotificationActivationCallback)]
struct Activation;
impl INotificationActivationCallback_Impl for Activation_Impl {
    fn Activate(
        &self,
        app: &PCWSTR,
        args: &PCWSTR,
        _: *const NOTIFICATION_USER_INPUT_DATA,
        _: u32,
    ) -> Result<()> {
        // SAFETY: COM supplies valid NUL-terminated strings for the duration of this callback.
        let (app, args) = unsafe { (app.to_string()?, args.to_string()?) };
        super::windows::activate(&app, &args);
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: non-null COM output slot is valid for this call.
        unsafe {
            *object = std::ptr::null_mut();
        }
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let callback: INotificationActivationCallback = Activation.into();
        // SAFETY: COM supplies the requested IID and an output slot; QueryInterface owns the returned reference.
        unsafe { callback.query(iid, object).ok() }
    }
    fn LockServer(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
}

pub(super) fn register(app_id: &str) -> std::result::Result<(), String> {
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            let app_id = app_id.to_owned();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("vw-toast-activation".into())
                .spawn(move || {
                    let result = register_thread(&app_id).map_err(|e| e.to_string());
                    let live = result.is_ok();
                    let _ = tx.send(result);
                    if live {
                        // MTA COM dispatches callbacks on its own threads. The registration lives until process exit.
                        loop {
                            std::thread::park();
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            rx.recv().map_err(|e| e.to_string())?
        })
        .clone()
}

fn register_thread(app_id: &str) -> Result<()> {
    let executable = std::env::current_exe()?;
    CURRENT_USER
        .create(format!(
            r"Software\Classes\CLSID\{CLASS_TEXT}\LocalServer32"
        ))?
        .set_string(
            "",
            format!("\"{}\" --notification-activation", executable.display()),
        )?;
    let key = CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{app_id}"))?;
    key.set_string("CustomActivator", CLASS_TEXT)?;
    key.set_string("DisplayName", "Vibe Work")?;
    let factory: IClassFactory = Factory.into();
    // SAFETY: this new thread has no apartment yet; COM retains the factory reference until process exit.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        CoRegisterClassObject(&CLASS, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)?;
    }
    Ok(())
}
