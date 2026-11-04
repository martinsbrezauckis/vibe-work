//! Native tray and close wiring. Decisions/persistence belong to vw-host; Linux host discovery to
//! vw-platform. Called from setup and window events; every Open path uses `reveal_main_window`.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager};
use vw_host::tray::{TrayPreferences, TrayView, WindowAction};
use vw_platform::Notifier;
use vw_vocabulary::texts;

pub(crate) const OPEN: &str = "open";
pub(crate) const QUIT: &str = "quit";

pub(crate) struct TrayState {
    preferences: Mutex<TrayPreferences>,
    icon: AtomicBool,
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
    window.show()?;
    #[cfg(target_os = "macos")]
    app.show()?;
    window.set_focus()?;
    crate::report_native_window_visibility(&window);
    Ok(())
}

pub(crate) fn menu(app: &AppHandle, id: &str) -> Result<(), String> {
    match id {
        OPEN => reveal_main_window(app).map_err(|error| error.to_string()),
        QUIT => {
            app.exit(0);
            Ok(())
        }
        _ => Err("unknown tray menu item".to_owned()),
    }
}

pub(crate) fn setup(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let path = crate::startup::data_dir(app)?.join("settings.json");
    app.manage(TrayState {
        preferences: Mutex::new(TrayPreferences::load(&path)),
        icon: AtomicBool::new(false),
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
    app.state::<TrayState>().icon.store(true, Ordering::Release);
    #[cfg(target_os = "linux")]
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            vw_platform::tray::watch_host(move |host| {
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

fn build(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;
    let open = MenuItem::with_id(app, OPEN, texts::OPEN_VIBE_WORK, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, texts::QUIT_VIBE_WORK, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;
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
    let host = vw_platform::tray::host_available().await;
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
        let notifier = vw_platform::DesktopNotifier::new(
            app.config().identifier.clone(),
            !crate::startup::DEV_BUILD,
        );
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
