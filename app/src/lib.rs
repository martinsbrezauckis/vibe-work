//! Vibe Work Tauri application: thin wiring only.
//!
//! Depends on: tauri, `vw-host` (all behaviour: core, Debug API, event forwarding, UI-control
//! broker, navigation policy). Called by: `main.rs`.
//! Invariants:
//! - The dialog plugin is registered for Rust's own use only: no capability grants it to the webview.
//! - No logic here: commands forward to [`vw_host::Host`], and the
//!   navigation hook forwards to [`vw_host::navigation::navigation_allowed`].
//! - The window opens before Vibe is ready; the host starts in the background and commands wait
//!   for it ([`vw_host::HostHandle`]).
//! - File paths reach the core only from the native picker; a drop on the composer sends the
//!   dropped files' bytes, never a path (Tauri's own drop handler is disabled).
//! - The main window is a trusted surface: it may only navigate within the app's own
//!   origin, and it may not open other windows. Its only web permission is the microphone, for dictation
//!   (`vw_host::media`); every other permission request is refused.
//!   Preview webviews live inside it as children with no capability (`capabilities/main.json` names
//!   the `main` webview only) and their own navigation policy.
//! - The updater plugin is registered only when the build carries an accepted update channel
//!   (`updater.rs`); user builds then check once after start and follow the update policy (always a
//!   notification; download in the background unless `settings.json` says `autoDownload: false`; install and
//!   restart only after the user's agreement, `vw_host::update_flow`). The plugin is for Rust's own use: no
//!   capability grants it to the webview, which reaches the flow only through the `update_*` commands.

// The e2e runs' in-page channel is for debug test builds only (vw-debug-api refuses it too).
#[cfg(all(feature = "e2e-script", not(debug_assertions)))]
compile_error!(
    "the e2e-script feature is for debug test builds only; it must never be in a release build"
);

mod commands;
mod dialogs;
mod live_profile;
mod startup;
mod updater;
mod window_api;

use tauri::webview::{NewWindowResponse, PermissionResponse};
use tauri::{Manager, RunEvent, WebviewWindowBuilder};
use vw_core_api::{Command, PreviewRequest};
use vw_host::HostHandle;
use vw_host::navigation::navigation_allowed;

fn main_permission(kind: &str) -> PermissionResponse {
    let granted = vw_host::media::main_window_grants(kind);
    tracing::info!(%kind, granted, "main window permission request");
    if granted {
        PermissionResponse::Allow
    } else {
        PermissionResponse::Deny
    }
}

/// Keeps the main webview on the app's own origin: a refused navigation never happens. Plugin navigation hooks run
/// for every webview (tauri `manager/webview.rs`), so the decision is per label: `main` gets the app-origin policy,
/// a preview webview is decided by its own handler (the preview's URL policy), any other label is
/// refused.
fn navigation_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|webview, url| match webview.label() {
            "main" => navigation_allowed(url, startup::DEV_BUILD),
            label => vw_preview::is_preview_label(label),
        })
        .build()
}

/// Files dropped on the window: the OS gives the paths to Rust; they go straight to the
/// core, which tells the UI (`documentsAdded`). The webview never sees a path.
/// NOT ACTIVE: the window config sets `dragDropEnabled: false` (Tauri's handler swallowed every OLE drop
/// on Windows), so Tauri never sends this event; drops reach the composer's HTML drop, which routes each file by
/// type (`ui/src/features/files/drop.ts`: pictures inline, everything else `documents addDropped`).
/// Kept for the day the Tauri drop is turned back on (then both paths must not add the same file twice).
fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() == "main"
        && matches!(
            event,
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_)
        )
    {
        report_native_window_visibility(window);
    }
    if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event
        && window.label() == "main"
        && !paths.is_empty()
    {
        let host = window.app_handle().state::<HostHandle>().inner().clone();
        let paths = paths.clone();
        tauri::async_runtime::spawn(async move {
            let host = host.get().await;
            if let Err(error) = host
                .add_documents(paths, vw_core_api::DocumentSource::Drop)
                .await
            {
                tracing::warn!(%error, "dropped files were not added");
            }
        });
    }
}

/// Sends native minimize/show changes to the core; the document's visibility report cannot see every OS transition.
fn report_native_window_visibility(window: &tauri::Window) {
    let (Ok(minimized), Ok(visible)) = (window.is_minimized(), window.is_visible()) else {
        return;
    };
    let host = window.app_handle().state::<HostHandle>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = host
            .get()
            .await
            .execute(Command::Preview {
                request: PreviewRequest::NativeWindowVisibility {
                    visible: visible && !minimized,
                },
            })
            .await;
    });
}

fn init_tracing() {
    // Logs go to stderr; the filter comes from RUST_LOG. Secrets are never logged by the crates
    // underneath (they redact at the source), so no filtering is needed here.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

/// Builds the Tauri application and runs it until the last window closes.
///
/// # Errors
///
/// Returns Tauri's error when the runtime cannot start (for example no display is available) or
/// the embedded context is invalid. The caller reports it and exits non-zero.
pub fn run() -> tauri::Result<()> {
    init_tracing();
    // The core will hold the Mistral key in memory: no core dumps, and on Linux
    // no ptrace or /proc reads by other same-user processes.
    let hardening = vw_platform::harden_process();
    tracing::info!(?hardening, "process hardening");
    #[cfg(feature = "e2e-script")]
    tracing::warn!(
        "test build: the Debug API's /e2e/script channel is compiled in; not for everyday use"
    );
    let handle = HostHandle::new();
    let setup_handle = handle.clone();
    let context = tauri::generate_context!();
    let updater = updater::plugin_if_configured(context.config());
    let update_checks = updater.is_some();
    let mut builder = tauri::Builder::default()
        .plugin(navigation_guard())
        .plugin(tauri_plugin_dialog::init());
    if let Some(plugin) = updater {
        builder = builder.plugin(plugin);
    }
    let app = builder
        .manage(handle.clone())
        .manage(updater::UpdatesHandle::default())
        .manage(updater::VibeSlot::default())
        .invoke_handler(tauri::generate_handler![
            commands::core_execute,
            commands::core_subscribe,
            commands::ui_control_next,
            commands::ui_control_reply,
            commands::debug_api_set,
            commands::cli_sharing,
            commands::debug_api_token_file,
            commands::host_subscribe,
            commands::pick_folder,
            commands::pick_documents,
            commands::pick_library_files,
            commands::export_page,
            commands::export_session,
            commands::reveal_folder,
            commands::open_link,
            commands::open_mail_link,
            commands::notify_session,
            commands::terminal_attach,
            commands::terminal_ack,
            commands::terminal_detach,
            commands::update_subscribe,
            commands::update_check,
            commands::update_download,
            commands::update_install,
            commands::update_set_auto_download,
            commands::update_open_release_page,
        ])
        .on_window_event(on_window_event)
        .setup(move |app| {
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
            setup_handle.start_in_background(
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
            let builder = WebviewWindowBuilder::from_config(app.handle(), config)?;
            let window = live_profile::window(builder, live_root)?
                .on_new_window(|_url, _features| NewWindowResponse::Deny)
                .on_permission_request(|_webview, kind| main_permission(&kind.to_string()))
                .build()?;
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
            Ok(())
        })
        .build(context)?;
    app.run(move |app_handle, event| {
        if let RunEvent::Exit = event
            && let Some(host) = app_handle.state::<HostHandle>().try_get()
        {
            tauri::async_runtime::block_on(host.shutdown());
        }
    });
    Ok(())
}
