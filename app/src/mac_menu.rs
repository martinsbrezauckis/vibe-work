//! macOS app menu: Tauri's default menu with its Quit item replaced by Vibe Work's own. The predefined Quit sends `AppKit`'s
//! `terminate:`, which the windowing layer (`tao` 0.37) ends the process on without an exit request, so Cmd+Q skipped the
//! running-work question. Vibe Work's item keeps the label and Cmd+Q and asks through `quit_guard` like the tray's Quit.
//! Depends on: `quit_guard`, the vocabulary's Quit label. Called by: lib (Builder menu and menu events). macOS only.

use tauri::menu::{Menu, MenuEvent, MenuItem, MenuItemKind};
use tauri::{AppHandle, Wry};

/// Menu id of Vibe Work's own Quit in the app menu (distinct from the tray's ids).
const QUIT_ID: &str = "app-quit";

/// The default menu, with the app submenu's trailing predefined Quit replaced by Vibe Work's Quit (Cmd+Q).
pub(crate) fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next()
        && let Some(last) = app_menu.items()?.into_iter().last()
        && matches!(last, MenuItemKind::Predefined(_))
    {
        app_menu.remove(&last)?;
        let quit = MenuItem::with_id(
            app,
            QUIT_ID,
            vw_vocabulary::texts::QUIT_VIBE_WORK,
            true,
            Some("CmdOrCtrl+Q"),
        )?;
        app_menu.append(&quit)?;
    }
    Ok(menu)
}

/// Menu events for the app menu; the tray handles its own ids.
pub(crate) fn on_event(app: &AppHandle, event: &MenuEvent) {
    if event.id().as_ref() == QUIT_ID {
        crate::quit_guard::request(app, crate::quit_guard::End::Quit);
    }
}
