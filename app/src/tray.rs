//! Native tray and close wiring. Decisions/persistence belong to vw-host; Linux host discovery to
//! vw-platform. Called from setup and window events; every Open path uses `reveal_main_window`.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager};
use vw_host::tray::{TrayPreferences, TrayView, WindowAction};
use vw_platform::Notifier;
use vw_vocabulary::texts;

pub(crate) const OPEN: &str = "open";
pub(crate) const RESTART: &str = "restart";
pub(crate) const QUIT: &str = "quit";
/// The tray's Restart item (only the tray says it; the shared vocabulary's sentences already say "Restart Vibe Work").
const RESTART_VIBE_WORK: &str = "Restart Vibe Work";

/// The tray menu's last items, in order (after Open and Feedback): Restart beside Quit. Both ask first while agent work
/// runs (`quit_guard`); Restart stops the app exactly as Quit does, then starts it again.
pub(crate) const END_ITEMS: [(&str, &str); 2] =
    [(RESTART, RESTART_VIBE_WORK), (QUIT, texts::QUIT_VIBE_WORK)];

pub(crate) struct TrayState {
    preferences: Mutex<TrayPreferences>,
    icon: AtomicBool,
    /// The tray host as the startup probe and the Linux host watch last saw it; Settings reads it without a bus probe.
    host: vw_platform::tray::HostCache,
    notifications: AtomicBool,
    pending_notice: AtomicBool,
    closing: Mutex<Option<tauri::ipc::Channel<()>>>,
}

/// Reveals even a window holding child webviews. macOS retains its regular Dock activation policy.
pub(crate) fn reveal_main_window<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = crate::main_window(app) else {
        return Ok(());
    };
    window.unminimize()?;
    show(app, &window)?;
    #[cfg(target_os = "macos")]
    app.show()?;
    window.set_focus()?;
    crate::report_native_window_visibility(&window);
    Ok(())
}

/// Shows the main window and nothing inside it. On Linux, tao's `Window::show` is `gtk_widget_show_all`: it also
/// shows every child webview the preview engine hid with `gtk_widget_hide` (a Browser page of another chat or of a
/// closed panel), and GTK then draws that page over the app. So only the GTK toplevel is shown here; each child keeps
/// the visibility the preview engine gave it. Runs on the GTK main thread (at once when called there), queued before
/// the focus request that follows.
#[cfg(target_os = "linux")]
fn show<R: tauri::Runtime>(app: &AppHandle<R>, window: &tauri::Window<R>) -> tauri::Result<()> {
    use gtk::prelude::*;
    let window = window.clone();
    app.run_on_main_thread(move || match window.gtk_window() {
        Ok(native) => show_toplevel(native.upcast_ref()),
        Err(error) => tracing::warn!(%error, "the main window could not be shown"),
    })
}

#[cfg(not(target_os = "linux"))]
fn show<R: tauri::Runtime>(_app: &AppHandle<R>, window: &tauri::Window<R>) -> tauri::Result<()> {
    window.show()
}

/// Maps `window` without changing the visibility of its children (`gtk_widget_show`, never `show_all`).
#[cfg(target_os = "linux")]
fn show_toplevel(window: &gtk::Window) {
    use gtk::prelude::*;
    window.show();
}

#[cfg(all(test, target_os = "linux"))]
#[path = "tray_linux_tests.rs"]
mod linux_tests;

pub(crate) fn menu(app: &AppHandle, id: &str) -> Result<(), String> {
    match id {
        OPEN => reveal_main_window(app).map_err(|error| error.to_string()),
        "feedback" => {
            crate::feedback_entry::open(app);
            Ok(())
        }
        RESTART => {
            crate::quit_guard::request(app, crate::quit_guard::End::Restart);
            Ok(())
        }
        QUIT => {
            crate::quit_guard::request(app, crate::quit_guard::End::Quit);
            Ok(())
        }
        _ => Err("unknown tray menu item".to_owned()),
    }
}

/// Quits as Quit does (the host shuts down at exit), then starts the app again (after `quit_guard`'s question). The restart mark lets the new process
/// wait for this one's data-folder lock instead of calling the folder "already running".
pub(crate) fn restart(app: &AppHandle) {
    match crate::startup::data_dir(app) {
        Ok(data) => {
            if let Err(error) = vw_host::instance::mark_restart(&data) {
                tracing::warn!(%error, "restart mark not written");
            }
        }
        Err(error) => tracing::warn!(%error, "restart mark not written: no data folder"),
    }
    // Linux (the AppImage): as for an update's restart, the new process must not inherit the old image's mount
    // keep-alive descriptor.
    #[cfg(target_os = "linux")]
    {
        let changed = vw_platform::stop_descriptor_inheritance();
        tracing::info!(changed, "restart: inherited descriptors closed");
    }
    app.request_restart();
}

pub(crate) fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::startup::data_dir(app)?.join("settings.json");
    app.manage(TrayState {
        preferences: Mutex::new(TrayPreferences::load(&path)),
        icon: AtomicBool::new(false),
        host: vw_platform::tray::HostCache::default(),
        notifications: AtomicBool::new(false),
        pending_notice: AtomicBool::new(false),
        closing: Mutex::new(None),
    });
    if !tauri::async_runtime::block_on(vw_platform::tray::host_available()) {
        return Ok(());
    }
    // AppIndicator dlopens its library and panics if it is missing; that must leave a usable window.
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(app)));
    if !matches!(built, Ok(Ok(()))) {
        tracing::warn!("tray could not be created; closing the window will quit");
        return Ok(());
    }
    let state = app.state::<TrayState>();
    state.host.update(true);
    state.icon.store(true, Ordering::Release);
    #[cfg(target_os = "linux")]
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            vw_platform::tray::watch_host(move |host| {
                if app.state::<TrayState>().host.update(host)
                    && let Some(events) = app
                        .try_state::<vw_host::HostHandle>()
                        .and_then(|handle| handle.try_get())
                {
                    events.tray_host(host);
                }
                if let Some(window) = crate::main_window(&app)
                    && vw_host::tray::must_reveal(window.is_visible().unwrap_or(true), host)
                {
                    let _ = reveal_main_window(&app);
                }
            })
            .await;
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn build(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let normal = tauri::include_image!("icons/tray-linux.png");
    let marked =
        vw_host::attention::tray(normal.rgba(), normal.width(), normal.height(), false, true);
    let open = app.clone();
    let quit = app.clone();
    let mut actions: Vec<vw_platform::tray::indicator::MenuAction<'_>> = vec![(
        texts::OPEN_VIBE_WORK,
        Box::new(move || {
            let _ = reveal_main_window(&open);
        }),
    )];
    {
        let feedback = app.clone();
        actions.push((
            crate::feedback_entry::LABEL,
            Box::new(move || crate::feedback_entry::open(&feedback)),
        ));
    }
    let restart = app.clone();
    actions.push((
        END_ITEMS[0].1,
        Box::new(move || crate::quit_guard::request(&restart, crate::quit_guard::End::Restart)),
    ));
    actions.push((
        END_ITEMS[1].1,
        Box::new(move || crate::quit_guard::request(&quit, crate::quit_guard::End::Quit)),
    ));
    vw_platform::tray::indicator::create(
        &crate::startup::data_dir(app)?.join("tray"),
        include_bytes!("../icons/tray-linux.png"),
        (&marked, normal.width(), normal.height()),
        actions,
    )
    .map_err(Into::into)
}

#[cfg(not(target_os = "linux"))]
fn build(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;
    let open = MenuItem::with_id(app, OPEN, texts::OPEN_VIBE_WORK, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open])?;
    menu.append(&MenuItem::with_id(
        app,
        "feedback",
        crate::feedback_entry::LABEL,
        true,
        None::<&str>,
    )?)?;
    menu.append(&separator)?;
    for (id, label) in END_ITEMS {
        menu.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
    }
    #[cfg(windows)]
    let icon = tauri::include_image!("icons/tray-windows.ico");
    #[cfg(target_os = "macos")]
    let icon = tauri::include_image!("icons/tray-template@2x.png");
    #[cfg(not(any(windows, target_os = "macos")))]
    let icon = tauri::include_image!("icons/tray-linux.png");
    let builder = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Vibe Work")
        .menu(&menu)
        .icon_as_template(cfg!(target_os = "macos"))
        // Windows and macOS: a left click brings the window back at once (as the Dock and the taskbar do); the menu is on the
        // right click. Linux tray hosts mostly show the menu on any click, so it keeps the menu there.
        .show_menu_on_left_click(cfg!(not(any(windows, target_os = "macos"))))
        .on_menu_event(|app, event| {
            let _ = self::menu(app, event.id.as_ref());
        });
    #[cfg(any(windows, target_os = "macos"))]
    let builder = builder.on_tray_icon_event(|tray, event| {
        use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            }
        ) {
            let _ = reveal_main_window(tray.app_handle());
        }
    });
    builder.build(app)?;
    Ok(())
}

pub(crate) async fn view(app: &AppHandle, enabled: Option<bool>) -> Result<TrayView, String> {
    let state = app.state::<TrayState>();
    // Linux: the host watch's answer (it re-probes every second and on the watcher's signals), not a bus probe per
    // call. Other platforms answer at once.
    let host = if cfg!(target_os = "linux") {
        state.host.get()
    } else {
        vw_platform::tray::host_available().await
    };
    let mut preferences = state
        .preferences
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(enabled) = enabled {
        preferences
            .set_enabled(enabled)
            .map_err(|error| error.to_string())?;
    }
    Ok(TrayView {
        enabled: preferences.enabled(),
        available: host && state.icon.load(Ordering::Acquire),
    })
}

pub(crate) fn subscribe(app: &AppHandle, closing: tauri::ipc::Channel<()>, notifications: bool) {
    let state = app.state::<TrayState>();
    state.notifications.store(notifications, Ordering::Release);
    *state.closing.lock().unwrap_or_else(PoisonError::into_inner) = Some(closing);
    first_notice(app);
}

pub(crate) fn close(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    let app = window.app_handle();
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    let enabled = state
        .preferences
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .enabled();
    // Check on every close, including a host lost since startup. An absent icon never loads its library.
    let icon = state.icon.load(Ordering::Acquire);
    let host =
        enabled && icon && tauri::async_runtime::block_on(vw_platform::tray::host_available());
    if !vw_host::tray::hides(WindowAction::Close, enabled, icon, host) {
        // This close ends the app: while agent work runs the person is asked first.
        crate::quit_guard::hold_close(window, api);
        return;
    }
    api.prevent_close();
    if let Some(closing) = state
        .closing
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        let _ = closing.send(());
    }
    if let Err(error) = window.hide() {
        tracing::warn!(%error, "window could not hide");
        return;
    }
    crate::report_native_window_visibility(window);
    let first = state
        .preferences
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first_hide();
    if matches!(first, Ok(true)) {
        state.pending_notice.store(true, Ordering::Release);
    }
    // Hide is queued by the runtime. Query visibility from a worker after the event handler
    // returns, so an already-subscribed UI gets its notice after the queued hide takes effect.
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || first_notice(&app));
}

/// A close can precede the webview's subscription. Wait for its actual notification preference,
/// then consume the notice only while hidden; a fast reopen must never get a hidden-mode banner.
fn first_notice(app: &AppHandle) {
    let state = app.state::<TrayState>();
    if !state.pending_notice.load(Ordering::Acquire)
        || state
            .closing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
        || crate::main_window(app).is_none_or(|window| window.is_visible().unwrap_or(true))
    {
        return;
    }
    if !state.pending_notice.swap(false, Ordering::AcqRel)
        || !state.notifications.load(Ordering::Acquire)
    {
        return;
    }
    let app = app.clone();
    // OS calls and notification actions never block the UI thread.
    std::thread::spawn(move || {
        let notifier = crate::notifications::notifier(&app, !crate::startup::DEV_BUILD);
        #[cfg(target_os = "macos")]
        let body = texts::TRAY_NOTICE_MAC;
        #[cfg(not(target_os = "macos"))]
        let body = texts::TRAY_NOTICE;
        let click = app.clone();
        let _ = notifier.show(
            vw_platform::NotificationRequest {
                title: texts::TRAY_NOTICE_TITLE.to_owned(),
                body: body.to_owned(),
            },
            Box::new(move || {
                let _ = reveal_main_window(&click);
            }),
        );
    });
}

#[cfg(test)]
mod tests {
    /// Restart sits beside Quit at the end of the tray menu, worded as the owner chose, on every system.
    #[test]
    fn the_tray_offers_restart_beside_quit() {
        assert_eq!(
            super::END_ITEMS,
            [
                ("restart", "Restart Vibe Work"),
                ("quit", vw_vocabulary::texts::QUIT_VIBE_WORK)
            ]
        );
    }
}
