//! Main-thread startup wiring: instance, tray, host and main webview creation.
//! Depends on Tauri and the app adapters; called by lib's setup hook, which reports errors before exiting.
//! The caller must consume failures: returning them to Tauri would panic inside its event loop.

use tauri::webview::NewWindowResponse;
use tauri::{Manager, WebviewWindowBuilder};
use vw_host::HostHandle;
use vw_host::hand_over::HandOver;

use crate::{
    dialogs, instance, live_profile, main_permission, report_native_window_visibility, startup,
    tray, updater,
};

pub(crate) fn start(
    app: &mut tauri::App,
    handle: &HostHandle,
    hand_over: Option<HandOver>,
    update_checks: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !instance::admit(app, hand_over)? {
        return Ok(());
    }
    tray::setup(app.handle())?;
    let live_root = live_profile::root()?;
    let confirmer = dialogs::native_confirmer(app.handle().clone());
    if update_checks && let Ok(data) = startup::data_dir(app.handle()) {
        updater::start(
            app.handle(),
            &app.state::<updater::UpdatesHandle>(),
            data,
            startup::DEV_BUILD,
        );
    }
    handle.start_in_background(
        &tauri::async_runtime::handle().inner().clone(),
        startup::host_config(app.handle(), confirmer),
    );
    // The window is created in code (its config has `create: false`) so that opening
    // further windows (`window.open`, `target=_blank`) can be refused. Its config also has
    // `dragDropEnabled: false`: Tauri's own file-drop handler took every OLE drop on Windows
    // (the page saw no drag event, so dropping an image on the composer did nothing
    // on Windows 11), and the app uses only the webview's HTML5 drop.
    let Some(config) = app.config().app.windows.first() else {
        return Err("tauri.conf.json defines no window".into());
    };
    // The main window grants the microphone (dictation) and nothing else; the
    // decision is vw_host::media's. Linux also needs WebKitGTK's media stream switched on before the first
    // document exists (vw_webview::enable_media_capture, called right after the build on this thread).
    let mut builder = WebviewWindowBuilder::from_config(app.handle(), config)?;
    // A development run with its own data folder keeps its web storage there too (Windows and Linux; WKWebView
    // keeps one store per app, which the macOS test runner separates per run).
    if live_root.is_none()
        && let Some(dev) = startup::dev_data_override()
    {
        builder = builder.data_directory(dev.join("webview"));
    }
    let window = live_profile::window(builder, live_root)?
        .on_new_window(|_url, _features| NewWindowResponse::Deny)
        .on_permission_request(|_webview, kind| main_permission(&kind.to_string()))
        .build()?;
    vw_webview::prepare_child_views(&window)?;
    vw_webview::enable_media_capture(&window)?;
    if let Some(native) = app.handle().get_window("main") {
        report_native_window_visibility(&native);
    }
    // Windows: a bare Alt tap put the window in menu mode, and every bridge call waited until the next key.
    // The app has no menu bar, so the tap is ignored (Alt+Space unchanged).
    #[cfg(windows)]
    if let Err(error) = window
        .hwnd()
        .map_err(|e| e.to_string())
        .and_then(|h| vw_platform::ignore_bare_alt(h.0 as isize))
    {
        tracing::warn!(%error, "the main window keeps the OS's Alt menu mode");
    }
    // Windows: a polite close (taskkill without /F, MainWindowHandle) of the minimized app hit tao's
    // event-target window and cut the event loop's route; it is now passed on to this window.
    #[cfg(windows)]
    if let Err(error) = window
        .hwnd()
        .map_err(|e| e.to_string())
        .and_then(|h| vw_platform::keep_event_target_on_close(h.0 as isize))
    {
        tracing::warn!(%error, "the event-target window keeps the OS's WM_CLOSE");
    }
    Ok(())
}
