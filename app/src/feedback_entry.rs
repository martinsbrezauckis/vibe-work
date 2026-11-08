//! Native feedback entry wiring. Menus reveal the window and offer the core's existing form; never attach or send.
//! Called by setup and tray. The optional build feature gates every native entry.
use tauri::{AppHandle, Manager};
use vw_core_api::{Command, CommandResult, FeedbackDraft, FeedbackKind, FeedbackRequest};
use vw_host::HostHandle;

pub(crate) const LABEL: &str = "Send feedback…";

pub(crate) fn open(app: &AppHandle) {
    let _ = crate::tray::reveal_main_window(app);
    let host = app.state::<HostHandle>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let host = host.get().await;
        let read = host
            .execute(Command::Feedback {
                request: FeedbackRequest::Read,
            })
            .await;
        // Read also publishes the current form. Never replace an already edited draft.
        if matches!(read, Ok(CommandResult::Feedback { view }) if view.draft.is_none()) {
            let _ = host
                .execute(Command::Feedback {
                    request: FeedbackRequest::Open {
                        draft: FeedbackDraft {
                            kind: FeedbackKind::Bug,
                            title: String::new(),
                            description: String::new(),
                            remarks: String::new(),
                        },
                    },
                })
                .await;
        }
    });
}

#[cfg(target_os = "macos")]
pub(crate) fn setup(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{HELP_SUBMENU_ID, MenuItem};
    // Tauri installs its standard macOS menus before setup. Keep all their native behavior.
    if let Some(menu) = app.menu()
        && let Some(help) = menu
            .get(HELP_SUBMENU_ID)
            .and_then(|item| item.as_submenu().cloned())
    {
        help.append(&MenuItem::with_id(
            app,
            "feedback-help",
            LABEL,
            true,
            None::<&str>,
        )?)?;
    }
    app.on_menu_event(|app, event| {
        if event.id.as_ref() == "feedback-help" {
            open(app);
        }
    });
    Ok(())
}
