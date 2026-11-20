//! The main window's page process ending under the app (the engine crashed, the system ended it, or it stopped
//! answering): the page is reloaded once and the person is told; a second end soon after asks before reloading, so a
//! page that keeps crashing never loops; when the whole engine ended (Windows) the person is told to restart. The app
//! process, the core and every chat keep running throughout: only the window's page is replaced, and it reads the
//! chats back as at any reload.
//! A preview's page process ending (macOS: Tauri's builder hook, which replaces `WebKit`'s own silent reload for every
//! webview) follows the same rule per preview: the first end reloads it quietly; a second end of the same preview
//! within two minutes does not reload it but asks the person, so a page that keeps crashing its process cannot drive
//! a reload loop. Windows and Linux previews are never reloaded automatically (their engines show the ended page).
//! Depends on: `vw_webview::PageCrash` (the platform hooks), the native dialogs. Called by: setup (Linux, Windows) and
//! the macOS Tauri builder hook (lib).
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use vw_webview::PageCrash;

/// A second end within this time asks before reloading.
const AGAIN_WITHIN: Duration = Duration::from_secs(120);

/// The notice after the reload (heading, then the detail); its one button.
const RELOADED: &str = "The Vibe Work window stopped working and was reloaded.\n\nYour chats and the agent's work \
                        continue. Text you had not sent yet may be gone.";
/// The question when it ended again soon after; the reload button, then the declining one.
const AGAIN: &str = "The Vibe Work window stopped working again.\n\nReload it now? If this keeps happening, quit and \
                     start Vibe Work again.";
/// The notice when the whole engine ended (no reload is possible).
const ENGINE: &str = "The Vibe Work window stopped working and cannot be reloaded.\n\nQuit and start Vibe Work again. \
                      Your chats are kept.";
const OK: &str = "OK";
const RELOAD: &str = "Reload";
const NOT_NOW: &str = "Not now";

/// The question when a preview's page ended again soon after; the reload button, then the declining one.
const PREVIEW_AGAIN: &str = "A page in Vibe Work stopped working again.\n\nReload it now? If it keeps happening, \
                             close that page.";

/// When the main window's page last ended.
static LAST: Mutex<Option<Instant>> = Mutex::new(None);
/// When each preview's page last ended, by webview label (ends older than [`AGAIN_WITHIN`] are forgotten).
static PREVIEW_LAST: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());

/// The previous end of the preview `label` (forgetting ends older than [`AGAIN_WITHIN`]), recording `now`.
fn preview_last(ends: &mut Vec<(String, Instant)>, label: &str, now: Instant) -> Option<Instant> {
    ends.retain(|(_, at)| now.saturating_duration_since(*at) < AGAIN_WITHIN);
    let last = ends.iter().find(|(l, _)| l == label).map(|(_, at)| *at);
    ends.retain(|(l, _)| l != label);
    ends.push((label.to_owned(), now));
    last
}

/// Handles one end of a preview's page (macOS; called on the UI thread; nothing here waits).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn preview_crashed(webview: &tauri::Webview) {
    let now = Instant::now();
    let label = webview.label().to_owned();
    let last = PREVIEW_LAST
        .lock()
        .map_or(None, |mut ends| preview_last(&mut ends, &label, now));
    let recovery = recovery(PageCrash::Ended, last, now);
    tracing::warn!(?recovery, label, "a preview's page stopped working");
    match recovery {
        Recovery::AskToReload => {
            let (app, webview) = (webview.app_handle().clone(), webview.clone());
            tauri::async_runtime::spawn(async move {
                let labels = [RELOAD.to_owned(), NOT_NOW.to_owned()];
                if crate::dialogs::ask_person(&app, PREVIEW_AGAIN.to_owned(), &labels)
                    .await
                    .as_deref()
                    == Some(RELOAD)
                    && let Err(error) = webview.reload()
                {
                    tracing::warn!(%error, "a preview could not be reloaded");
                }
            });
        }
        Recovery::ReloadAndTell | Recovery::TellToRestart => {
            if let Err(error) = webview.reload() {
                tracing::warn!(%error, label = webview.label(), "a preview could not be reloaded");
            }
        }
    }
}

/// What the app does about one end of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recovery {
    /// Reload at once, then say that it happened.
    ReloadAndTell,
    /// Ask first (it ended again soon after the last time).
    AskToReload,
    /// Say that only a restart helps.
    TellToRestart,
}

/// The recovery for `crash` at `now`, the previous end having been at `last`.
pub(crate) fn recovery(crash: PageCrash, last: Option<Instant>, now: Instant) -> Recovery {
    if crash == PageCrash::EngineEnded {
        Recovery::TellToRestart
    } else if last.is_some_and(|last| now.saturating_duration_since(last) < AGAIN_WITHIN) {
        Recovery::AskToReload
    } else {
        Recovery::ReloadAndTell
    }
}

/// Handles one end of the main window's page (called on the UI thread; nothing here waits).
pub(crate) fn crashed(app: &AppHandle, crash: PageCrash) {
    let now = Instant::now();
    let last = LAST.lock().map_or(None, |mut last| last.replace(now));
    let recovery = recovery(crash, last, now);
    tracing::warn!(?crash, ?recovery, "the main window's page stopped working");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match recovery {
            Recovery::ReloadAndTell => {
                reload(&app);
                crate::dialogs::ask_person(&app, RELOADED.to_owned(), &[OK.to_owned()]).await;
            }
            Recovery::AskToReload => {
                let labels = [RELOAD.to_owned(), NOT_NOW.to_owned()];
                if crate::dialogs::ask_person(&app, AGAIN.to_owned(), &labels)
                    .await
                    .as_deref()
                    == Some(RELOAD)
                {
                    reload(&app);
                }
            }
            Recovery::TellToRestart => {
                crate::dialogs::ask_person(&app, ENGINE.to_owned(), &[OK.to_owned()]).await;
            }
        }
    });
}

fn reload(app: &AppHandle) {
    let Some(webview) = app.get_webview(crate::MAIN_WINDOW) else {
        tracing::warn!("the main window could not be reloaded: no main webview");
        return;
    };
    if let Err(error) = webview.reload() {
        tracing::warn!(%error, "the main window could not be reloaded");
    }
}

#[cfg(test)]
mod tests {
    use super::{AGAIN_WITHIN, PageCrash, Recovery, preview_last, recovery};
    use std::time::{Duration, Instant};

    #[test]
    fn the_first_end_reloads_a_second_soon_after_asks_and_a_late_one_reloads_again() {
        let now = Instant::now() + AGAIN_WITHIN * 2;
        assert_eq!(
            recovery(PageCrash::Ended, None, now),
            Recovery::ReloadAndTell
        );
        let soon = now.checked_sub(Duration::from_secs(5));
        assert_eq!(recovery(PageCrash::Ended, soon, now), Recovery::AskToReload);
        assert_eq!(
            recovery(PageCrash::Unresponsive, soon, now),
            Recovery::AskToReload
        );
        let late = now.checked_sub(AGAIN_WITHIN + Duration::from_secs(1));
        assert_eq!(
            recovery(PageCrash::Ended, late, now),
            Recovery::ReloadAndTell
        );
        assert_eq!(
            recovery(PageCrash::EngineEnded, None, now),
            Recovery::TellToRestart
        );
    }

    // A preview that ends again soon after a reload is not reloaded again: the person is asked instead, so
    // repeated ends never become a reload loop.
    #[test]
    fn a_preview_that_ends_again_soon_is_not_reloaded_but_asked_about() {
        let start = Instant::now() + AGAIN_WITHIN * 4;
        let mut ends = Vec::new();
        let mut reloads = 0;
        let mut asked = 0;
        for second in [0, 20, 40] {
            let now = start + Duration::from_secs(second);
            match recovery(
                PageCrash::Ended,
                preview_last(&mut ends, "browser-7", now),
                now,
            ) {
                Recovery::ReloadAndTell => reloads += 1,
                Recovery::AskToReload => asked += 1,
                Recovery::TellToRestart => unreachable!(),
            }
        }
        assert_eq!(
            (reloads, asked),
            (1, 2),
            "three ends in a minute: one reload, then questions"
        );
        // Another preview keeps its own count; a late end reloads quietly again.
        let now = start + Duration::from_secs(41);
        assert_eq!(
            recovery(
                PageCrash::Ended,
                preview_last(&mut ends, "browser-8", now),
                now
            ),
            Recovery::ReloadAndTell
        );
        let late = start + Duration::from_secs(40) + AGAIN_WITHIN + Duration::from_secs(1);
        assert_eq!(
            recovery(
                PageCrash::Ended,
                preview_last(&mut ends, "browser-7", late),
                late
            ),
            Recovery::ReloadAndTell
        );
        assert!(ends.len() <= 2, "old ends are forgotten: {ends:?}");
    }

    #[test]
    fn the_preview_question_has_a_heading_and_a_detail() {
        let (heading, detail) = super::PREVIEW_AGAIN.split_once("\n\n").unwrap();
        assert!(heading.ends_with("stopped working again."), "{heading}");
        assert!(detail.starts_with("Reload it now?"), "{detail}");
    }

    #[test]
    fn each_text_has_a_heading_and_a_detail() {
        for text in [super::RELOADED, super::AGAIN, super::ENGINE] {
            let (heading, detail) = text.split_once("\n\n").unwrap();
            assert!(
                heading.starts_with("The Vibe Work window stopped working"),
                "{text}"
            );
            assert!(detail.len() > 20, "{text}");
        }
    }
}
