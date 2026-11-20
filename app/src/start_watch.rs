//! The start watch: a window that never comes up at start is never left silent. Armed at setup, it ends when the main
//! page first connects to the core (`core_subscribe`). If the page has not connected after [`PAGE_LIMIT`], the main
//! (UI) thread is asked to answer:
//! - it answers: the page is the problem, and it is handled as a page that stopped answering (`page_crash`: reloaded
//!   and the person told; a second time the person is asked);
//! - it does not answer within [`MAIN_LIMIT`]: the UI thread itself is stuck, so nothing can be shown in this process.
//!   The app stops its host (bounded), starts itself again and ends; the next start tells the person once. At most
//!   once per [`AGAIN_WITHIN`], so a start that keeps getting stuck never loops (later ones are only logged).
//!
//! Only the start is watched, on purpose: later, a native dialog or menu the person has open can hold the UI thread's
//! queued tasks for as long as it stays open, and that must never restart the app.
//! Depends on: `page_crash`, `vw_host` (instance restart mark, exit), the native dialogs. Called by: setup, lib
//! (`core_subscribe` through `page_connected`).
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

/// How long the main page may take to connect after setup before the UI thread is asked.
pub(crate) const PAGE_LIMIT: Duration = Duration::from_secs(45);
/// How long the UI thread may take to answer that question.
pub(crate) const MAIN_LIMIT: Duration = Duration::from_secs(15);
/// A second stuck start within this time is not restarted again.
pub(crate) const AGAIN_WITHIN: Duration = Duration::from_secs(10 * 60);
/// Pages handled as "stopped answering" before the watch gives up (the second one asks the person first).
const PAGE_ROUNDS: u32 = 2;
/// The marker of the last restart after a stuck start (data folder): "<unix ms>" and, once the person was told, " told".
const MARKER: &str = "start-restart";

/// The notice at the start after such a restart (heading, then the detail); its one button.
const RESTARTED: &str = "Vibe Work stopped answering while it started and was started again.\n\nYour chats and \
                         settings are kept.";
const OK: &str = "OK";

/// What a silent start is, once the UI thread was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The UI thread answered: only the page is silent.
    PageSilent,
    /// The UI thread did not answer: the app's window cannot work in this process.
    UiThreadStuck,
}

/// The verdict for a page still silent after [`PAGE_LIMIT`], the UI thread having answered or not.
pub(crate) fn verdict(ui_thread_answered: bool) -> Verdict {
    if ui_thread_answered {
        Verdict::PageSilent
    } else {
        Verdict::UiThreadStuck
    }
}

/// Whether a stuck start may restart the app now (`now_ms`), the last such restart having been at `last_ms`.
pub(crate) fn may_restart(last_ms: Option<u64>, now_ms: u64) -> bool {
    let again = u64::try_from(AGAIN_WITHIN.as_millis()).unwrap_or(u64::MAX);
    last_ms.is_none_or(|last| now_ms.saturating_sub(last) >= again)
}

/// The marker's restart time and whether the person was told.
fn read_marker(data: &Path) -> Option<(u64, bool)> {
    let text = std::fs::read_to_string(data.join(MARKER)).ok()?;
    let mut words = text.split_whitespace();
    let at = words.next()?.parse().ok()?;
    Some((at, words.next() == Some("told")))
}

fn write_marker(data: &Path, at: u64, told: bool) {
    let text = if told {
        format!("{at} told")
    } else {
        at.to_string()
    };
    if let Err(error) = vw_platform::replace_private(&data.join(MARKER), text.as_bytes()) {
        tracing::warn!(%error, "start watch: the restart marker could not be written");
    }
}

/// The watch's state: whether the main page has connected.
#[derive(Default)]
pub(crate) struct StartWatch {
    connected: AtomicBool,
}

/// The main page connected to the core: the start is over.
pub(crate) fn page_connected(app: &AppHandle) {
    if crate::startup::DEV_BUILD && std::env::var_os("VW_E2E_START_WATCH_SILENT_PAGE").is_some() {
        return;
    }
    if let Some(watch) = app.try_state::<StartWatch>() {
        watch.connected.store(true, Ordering::Release);
    }
}

/// Arms the watch (setup, before the main window exists).
pub(crate) fn start(app: &AppHandle) {
    app.manage(StartWatch::default());
    let data = crate::startup::data_dir(app).ok();
    stick_for_test(app, data.as_deref());
    let watched = app.clone();
    let spawned = std::thread::Builder::new()
        .name("start-watch".to_owned())
        .spawn(move || watch(&watched, data.as_deref()));
    if let Err(error) = spawned {
        tracing::warn!(%error, "start watch: not started");
    }
}

fn watch(app: &AppHandle, data: Option<&Path>) {
    for _ in 0..PAGE_ROUNDS {
        if wait_for_page(app, PAGE_LIMIT) {
            return;
        }
        let ui_thread_answered = ask_ui_thread(app, MAIN_LIMIT);
        if wait_for_page(app, Duration::ZERO) {
            return;
        }
        match verdict(ui_thread_answered) {
            Verdict::PageSilent => {
                tracing::warn!(
                    limit_s = PAGE_LIMIT.as_secs(),
                    "start watch: the main page has not connected; reloading it"
                );
                let page = app.clone();
                let _ = app.run_on_main_thread(move || {
                    crate::page_crash::crashed(&page, vw_webview::PageCrash::Unresponsive);
                });
            }
            Verdict::UiThreadStuck => {
                restart(app, data);
                return;
            }
        }
    }
    tracing::warn!("start watch: the main page still has not connected; the watch ends");
}

/// True once the page connected; waits up to `limit` for it.
fn wait_for_page(app: &AppHandle, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if app.state::<StartWatch>().connected.load(Ordering::Acquire) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Whether a task queued on the UI thread runs within `limit`.
fn ask_ui_thread(app: &AppHandle, limit: Duration) -> bool {
    let (answer, answered) = mpsc::channel();
    if app
        .run_on_main_thread(move || {
            let _ = answer.send(());
        })
        .is_err()
    {
        return false;
    }
    answered.recv_timeout(limit).is_ok()
}

/// The UI thread is stuck at start: stop the host, start the app again and end this process (it cannot show
/// anything). Not again within [`AGAIN_WITHIN`] of the last such restart.
fn restart(app: &AppHandle, data: Option<&Path>) {
    let now = vw_platform::wall_clock::unix_ms();
    let last = data.and_then(read_marker).map(|(at, _)| at);
    if !may_restart(last, now) {
        tracing::error!(
            "start watch: the app's UI thread is stuck at start again soon after a restart; not restarted again \
             (quit Vibe Work and start it again)"
        );
        return;
    }
    tracing::error!(
        limit_s = (PAGE_LIMIT + MAIN_LIMIT).as_secs(),
        "start watch: the app's UI thread is stuck at start; starting Vibe Work again"
    );
    if let Some(data) = data {
        write_marker(data, now, false);
        if let Err(error) = vw_host::instance::mark_restart(data) {
            tracing::warn!(%error, "start watch: no restart mark; the new start may only find this one");
        }
    }
    if let Some(host) = app.state::<vw_host::HostHandle>().try_get() {
        host.shutdown_at_exit(
            tauri::async_runtime::handle().inner(),
            vw_host::exit::EXIT_LIMIT,
        );
    }
    relaunch(app);
    // Its UI thread is stuck: nothing that waits for it (exit handlers) may run.
    vw_platform::end_process::now(0);
}

/// Starts this app again: through `LaunchServices` from a macOS bundle (`open -n`, as Tauri's own restart), else the
/// current program with this run's arguments.
fn relaunch(app: &AppHandle) {
    let env = app.env();
    let Ok(program) = tauri::process::current_binary(&env) else {
        tracing::error!(
            "start watch: this program's path is unknown; start Vibe Work again by hand"
        );
        return;
    };
    let args = env.args_os.iter().skip(1);
    let bundle = program
        .parent()
        .filter(|dir| dir.ends_with("Contents/MacOS"))
        .and_then(Path::parent)
        .and_then(Path::parent);
    let started = match bundle {
        Some(bundle) if cfg!(target_os = "macos") => {
            vw_platform::process_command::hidden("/usr/bin/open")
                .arg("-n")
                .arg(bundle)
                .arg("--args")
                .args(args)
                .spawn()
        }
        _ => vw_platform::process_command::hidden(&program)
            .args(args)
            .spawn(),
    };
    if let Err(error) = started {
        tracing::error!(%error, "start watch: Vibe Work could not be started again");
    }
}

/// At the start after a restart from a stuck start: tells the person once (after the main window exists).
pub(crate) fn notice(app: &AppHandle) {
    let Ok(data) = crate::startup::data_dir(app) else {
        return;
    };
    let Some((at, false)) = read_marker(&data) else {
        return;
    };
    write_marker(&data, at, true);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::dialogs::ask_person(&app, RESTARTED.to_owned(), &[OK.to_owned()]).await;
    });
}

/// Development builds only: `VW_E2E_START_WATCH_STUCK_UI` holds the UI thread at start (once: not after the watch's
/// own restart), so the restart path can be tested in the real app.
fn stick_for_test(app: &AppHandle, data: Option<&Path>) {
    if !crate::startup::DEV_BUILD
        || std::env::var_os("VW_E2E_START_WATCH_STUCK_UI").is_none()
        || data.and_then(read_marker).is_some()
    {
        return;
    }
    tracing::warn!("start watch test: the UI thread is held at start");
    // Queued from another thread: called on the UI thread itself, `run_on_main_thread` would run it here, inline,
    // before setup has finished.
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = app.run_on_main_thread(|| std::thread::sleep(Duration::from_secs(600)));
    });
}

#[cfg(test)]
mod tests {
    use super::{AGAIN_WITHIN, Verdict, may_restart, read_marker, verdict, write_marker};

    /// PREDICT: a start whose page stays silent is never left silent: with the UI thread answering it is a page
    /// problem (reload and tell), without it the app restarts itself; a second stuck start within ten minutes of
    /// the last restart is not restarted again, so a start that keeps getting stuck never loops.
    #[test]
    fn a_silent_start_reloads_the_page_or_restarts_the_app_once_per_ten_minutes() {
        assert_eq!(verdict(true), Verdict::PageSilent);
        assert_eq!(verdict(false), Verdict::UiThreadStuck);
        let again = u64::try_from(AGAIN_WITHIN.as_millis()).unwrap();
        let now = 10 * again;
        assert!(may_restart(None, now));
        assert!(!may_restart(Some(now - 1000), now));
        assert!(!may_restart(Some(now - again + 1), now));
        assert!(may_restart(Some(now - again), now));
        // A clock set back counts as "just now".
        assert!(!may_restart(Some(now + 5000), now));
    }

    /// PREDICT: the marker keeps the restart time and whether the person was told, so the notice shows once.
    #[test]
    fn the_restart_marker_tells_the_person_once() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_marker(dir.path()), None);
        write_marker(dir.path(), 1234, false);
        assert_eq!(read_marker(dir.path()), Some((1234, false)));
        write_marker(dir.path(), 1234, true);
        assert_eq!(read_marker(dir.path()), Some((1234, true)));
    }

    #[test]
    fn the_notice_has_a_heading_and_a_detail() {
        let (heading, detail) = super::RESTARTED.split_once("\n\n").unwrap();
        assert!(heading.starts_with("Vibe Work stopped answering"));
        assert!(detail.contains("chats"));
    }
}
