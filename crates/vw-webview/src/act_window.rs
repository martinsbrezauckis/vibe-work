//! An agent act's window on one mount: from the start of its input (an evaluation that counts as the person's
//! gesture, or a native click) until the user activation that input gave the page has run out. A page may use that
//! activation for seconds after the click. During the window the engine keeps two things a person's click would
//! allow but agent acts may not do:
//! - a file picker never shows (the agent attaches files with its upload action);
//! - full screen is refused: Linux refuses the window's full screen when `WebKitGTK` asks (`enter-fullscreen`) and
//!   makes the document leave it again; macOS 13+ and Windows see the view enter it (the view's own state, for
//!   every frame) and leave it at once (they have no way to refuse beforehand); macOS 11-12 switch it off for the
//!   window. Linux and macOS 13+ change no setting during an act: a settings change right before a native click
//!   moves the page under the checked point (Linux), and a document made while full screen is off keeps no
//!   full-screen API (macOS).
//!
//! Windows also lets the page count as focused during the window (focus emulation; the view never takes the
//! person's keyboard focus): `WebView2` refuses a clipboard write in a page without focus, and the agent's click,
//! unlike a person's, does not focus the view. A Copy button then works for the agent as for a person.
//!
//! Each refusal is reported (`PreviewHooks::act_refused`) for the act's receipt. The person's own clicks keep the
//! normal behaviour: a press of the person's own pointer in the view (the same native signal that stops the agent's
//! checked clicks, which never sees the engine's own input and which a page cannot produce) ends the window. It fails
//! closed: only a press made after the agent's input was dispatched counts (never a release, never a press already
//! down), and while that input is still in flight the press is only recorded; the window then ends the moment the
//! input returns instead of [`AFTER`] later. Right before the input is dispatched the restrictions are checked and
//! put on again if anything lifted them since the act started.
//!
//! Depends on vw-preview hooks, vw-platform (macOS full screen), Wry's file-panel gate (macOS). Called by the engine
//! around `evaluate_frames_as_input`, at mount (the pickers' watch) and at close.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tauri::{Webview, Wry};
use vw_preview::{ActRefusal, EngineError, PreviewHooks};

/// How long after an act's answer the page's reaction still counts as the act's: longer than the user activation
/// an input gives a page (5 s in Chromium and `WebKit`), so a request made with that activation is the act's.
pub(crate) const AFTER: Duration = Duration::from_secs(6);

struct Window {
    until: Instant,
    /// Counts the acts on this mount: an act's end applies only while no later act has started.
    generation: u64,
    hooks: Option<Arc<dyn PreviewHooks>>,
    /// The person pressed in the view after the agent's input was dispatched: the window ends when the input
    /// returns (at once, when it already has), and the finishing act does not reopen it.
    person: bool,
    /// The act's guard is alive: its input is being started or dispatched.
    in_flight: bool,
    /// When the act's input was dispatched; a person's press counts only after it.
    dispatched: Option<Instant>,
    /// The restrictions were lifted on the view (main thread, `end_now`) since this act's `start`.
    lifted: bool,
    /// The view, to lift the restrictions at once when the person takes over (macOS, Windows).
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    webview: Option<Webview<Wry>>,
}

/// Per mount label: the current or last act window.
static ACTING: LazyLock<Mutex<HashMap<String, Window>>> = LazyLock::new(Mutex::default);

fn windows() -> std::sync::MutexGuard<'static, HashMap<String, Window>> {
    ACTING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Marks mount `label` as running an agent act until the guard drops (then [`AFTER`] more).
pub(crate) struct Acting {
    label: String,
    generation: u64,
}

pub(crate) fn acting(
    label: &str,
    hooks: Option<Arc<dyn PreviewHooks>>,
    webview: Option<Webview<Wry>>,
) -> Acting {
    let mut windows = windows();
    let generation = windows.get(label).map_or(0, |w| w.generation) + 1;
    windows.insert(
        label.to_owned(),
        Window {
            until: Instant::now() + Duration::from_secs(3600),
            generation,
            hooks,
            person: false,
            in_flight: true,
            dispatched: None,
            lifted: false,
            webview,
        },
    );
    Acting {
        label: label.to_owned(),
        generation,
    }
}

impl Drop for Acting {
    fn drop(&mut self) {
        // A later act on this mount keeps its own window open.
        if let Some(window) = windows().get_mut(&self.label)
            && window.generation == self.generation
        {
            window.in_flight = false;
            // A press of the person's during the input ends the window now: their click works as soon as the agent's
            // input is done.
            window.until = Instant::now() + if window.person { Duration::ZERO } else { AFTER };
        }
    }
}

/// Whether the restrictions of act `generation` on `label` may end now: its window has run out and no later act
/// has started. Checked where the end is applied (the main thread), so a newer act is never left unprotected.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn may_end(label: &str, generation: u64) -> bool {
    windows()
        .get(label)
        .is_some_and(|w| w.generation == generation && !w.lifted && Instant::now() >= w.until)
}

/// Runs one agent input on mount `label` inside an act window: restrictions on before it, the window kept for the
/// activation it gives the page, the restrictions off afterwards (unless a later act runs then).
pub(crate) async fn during<T>(
    webview: &Webview<Wry>,
    label: &str,
    hooks: Arc<dyn PreviewHooks>,
    input: impl std::future::Future<Output = Result<T, EngineError>>,
) -> Result<T, EngineError> {
    let acting = acting(label, Some(hooks), Some(webview.clone()));
    let generation = acting.generation;
    let admitted = async {
        start(webview, label, generation).await?;
        // Whatever lifted the restrictions since `start`, confirm them again before input.
        if with_window(label, generation, |w| w.lifted) == Some(true) {
            start(webview, label, generation).await?;
        }
        Ok::<(), EngineError>(())
    }
    .await;
    if let Err(error) = admitted {
        drop(acting);
        with_window(label, generation, |w| w.until = Instant::now());
        end_later(webview, label, generation);
        return Err(EngineError::Platform(format!(
            "Not run: browser input restrictions could not be confirmed ({error}). Reopen the browser view before trying again."
        )));
    }
    with_window(label, generation, |w| w.dispatched = Some(Instant::now()));
    let answer = input.await;
    drop(acting);
    end_later(webview, label, generation);
    answer
}

/// Runs `f` on act `generation`'s window of mount `label`, while no later act has started there.
fn with_window<T>(label: &str, generation: u64, f: impl FnOnce(&mut Window) -> T) -> Option<T> {
    windows()
        .get_mut(label)
        .filter(|w| w.generation == generation)
        .map(f)
}

/// The person's own pointer went down (`pressed`) or up in mount `label`'s view. A press made after the agent's
/// input was dispatched ends the act window, so what the person's click asks for (a file picker, full screen) works
/// as for any person: at once, or, while the agent's input is still in flight, the moment it returns (the window
/// never opens under an input in flight). Releases and presses before the dispatch change nothing. Called from the
/// pointer watch, on the view's thread (Windows: the hook thread).
pub(crate) fn person_input(label: &str, pressed: bool) {
    if !pressed {
        return;
    }
    let ended = {
        let mut windows = windows();
        let Some(window) = windows.get_mut(label) else {
            return;
        };
        let now = Instant::now();
        if now >= window.until || window.dispatched.is_none_or(|at| now < at) {
            return;
        }
        window.person = true;
        if window.in_flight {
            return;
        }
        window.until = now;
        (window.webview.clone(), window.generation)
    };
    #[cfg(not(target_os = "linux"))]
    if let (Some(webview), generation) = ended {
        let label = label.to_owned();
        tauri::async_runtime::spawn(async move { end_now(&webview, &label, generation).await });
    }
    #[cfg(target_os = "linux")]
    let _ = ended;
}

/// Whether something mount `label`'s page asks for now belongs to an agent act.
pub(crate) fn agent_acting(label: &str) -> bool {
    windows().get(label).is_some_and(|w| {
        #[cfg(windows)]
        if w.webview.is_some() && !w.lifted {
            return true;
        }
        Instant::now() < w.until
    })
}

/// Reports a refusal for mount `label` (an acting one).
fn refused(label: &str, kind: ActRefusal) {
    let hooks = windows().get(label).and_then(|w| w.hooks.clone());
    if let Some(hooks) = hooks {
        hooks.act_refused(label, kind);
    }
}

/// Drops a closed mount's entry.
pub(crate) fn forget(label: &str) {
    windows().remove(label);
    #[cfg(target_os = "macos")]
    forget_view(label);
}

/// Opens the window's restrictions of act `generation` before its input, for every kind of input: macOS full
/// screen off; Windows the file picker interception and focus emulation on (its full screen is left at once,
/// `full_screen_entered`).
/// Linux decides at the engine's own requests (`install`), so nothing changes on the view there.
#[cfg_attr(target_os = "linux", allow(clippy::unused_async))]
async fn start(webview: &Webview<Wry>, label: &str, generation: u64) -> Result<(), EngineError> {
    #[cfg(any(target_os = "macos", windows))]
    {
        let native_label = label.to_owned();
        let result = crate::platform::with_native(
            webview,
            "starting an agent act",
            Duration::from_secs(5),
            move |native, reply| {
                if with_window(&native_label, generation, |_| ()).is_none() {
                    let _ = reply.send(Err("the browser act was replaced".into()));
                    return;
                }
                #[cfg(target_os = "macos")]
                {
                    if vw_platform::webview::full_screen_mac::needs_switch(native.inner()) {
                        vw_platform::webview::full_screen_mac::set_element_full_screen(
                            native.inner(),
                            false,
                        );
                    }
                    with_window(&native_label, generation, |w| w.lifted = false);
                    let _ = reply.send(Ok(()));
                }
                #[cfg(windows)]
                crate::frames_win::start_act(
                    &native,
                    generation,
                    Box::new(move |result| {
                        if result.is_ok() {
                            with_window(&native_label, generation, |w| w.lifted = false);
                        }
                        let _ = reply.send(result);
                    }),
                );
            },
        )
        .await;
        #[cfg(windows)]
        if result.is_err() {
            abort_native(webview, label, generation);
        }
        result?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = webview;
        with_window(label, generation, |w| w.lifted = false);
    }
    Ok(())
}

#[cfg(windows)]
fn abort_native(webview: &Webview<Wry>, label: &str, generation: u64) {
    let label = label.to_owned();
    if let Err(error) = webview.with_webview(move |native| {
        if with_window(&label, generation, |_| ()).is_some() {
            crate::frames_win::abort_act(&native, generation);
        }
    }) {
        tracing::error!("browser control teardown could not reach the view: {error}");
    }
}

/// After act `generation`: once its window has closed and no later act has started, macOS full screen is on again
/// (Windows: the picker interception and focus emulation end). Checked again on the main thread, right where it
/// applies.
#[cfg_attr(target_os = "linux", allow(clippy::needless_pass_by_value))]
fn end_later(webview: &Webview<Wry>, label: &str, generation: u64) {
    #[cfg(target_os = "linux")]
    let _ = (webview, label, generation);
    #[cfg(not(target_os = "linux"))]
    end_later_native(webview, label, generation);
}

#[cfg(not(target_os = "linux"))]
fn end_later_native(webview: &Webview<Wry>, label: &str, generation: u64) {
    let (webview, label) = (webview.clone(), label.to_owned());
    // Usually AFTER; at once when the person pressed during the input.
    let wait = with_window(&label, generation, |w| {
        w.until.saturating_duration_since(Instant::now())
    })
    .unwrap_or(AFTER);
    tokio::spawn(async move {
        tokio::time::sleep(wait + Duration::from_millis(100)).await;
        if may_end(&label, generation) {
            end_now(&webview, &label, generation).await;
        }
    });
}

/// Lifts act `generation`'s restrictions on the main thread, checked again there (a later act keeps them).
#[cfg(not(target_os = "linux"))]
async fn end_now(webview: &Webview<Wry>, label: &str, generation: u64) {
    let native_label = label.to_owned();
    let result = crate::platform::with_native(
        webview,
        "ending an agent act",
        Duration::from_secs(5),
        move |native, reply| {
            if !may_end(&native_label, generation) {
                let _ = reply.send(Ok(()));
                return;
            }
            #[cfg(target_os = "macos")]
            {
                if vw_platform::webview::full_screen_mac::needs_switch(native.inner()) {
                    vw_platform::webview::full_screen_mac::set_element_full_screen(
                        native.inner(),
                        true,
                    );
                }
                with_window(&native_label, generation, |w| w.lifted = true);
                let _ = reply.send(Ok(()));
            }
            #[cfg(windows)]
            crate::frames_win::end_act(
                &native,
                generation,
                Box::new(move |result| {
                    if result.is_ok() {
                        with_window(&native_label, generation, |w| w.lifted = true);
                    }
                    let _ = reply.send(result);
                }),
            );
        },
    )
    .await;
    if let Err(error) = result {
        tracing::error!("browser act controls did not end: {error}");
        #[cfg(windows)]
        abort_native(webview, label, generation);
    }
}

/// Linux: `WebKitGTK` asks `run-file-chooser` before it shows its dialog and `enter-fullscreen` before the window
/// goes full screen. During an agent act the picker request is cancelled (the page's field stays empty); the window
/// stays as it is and the document is made to leave full screen again. Both are reported. Outside an act `WebKit`
/// does both for the person as usual.
#[cfg(target_os = "linux")]
pub(crate) fn install(view: &webkit2gtk::WebView, label: String) {
    use webkit2gtk::{FileChooserRequestExt as _, WebViewExt as _};
    let pickers = label.clone();
    view.connect_run_file_chooser(move |_, request| {
        if !agent_acting(&pickers) {
            return false;
        }
        request.cancel();
        refused(&pickers, ActRefusal::FilePicker);
        true
    });
    view.connect_enter_fullscreen(move |view| {
        if !agent_acting(&label) {
            crate::linux_layout::page_full_screen(view, true);
            return false;
        }
        refused(&label, ActRefusal::FullScreen);
        // The document entered full screen inside the view; it leaves again once WebKit has finished entering.
        let view = view.clone();
        glib::timeout_add_local_once(Duration::from_millis(100), move || {
            view.evaluate_javascript(
                "document.fullscreenElement ? (document.exitFullscreen(), true) : false",
                Some("vibe-work-agent"),
                None,
                None::<&webkit2gtk::gio::Cancellable>,
                |_| {},
            );
        });
        true
    });
    view.connect_leave_fullscreen(|view| {
        crate::linux_layout::page_full_screen(view, false);
        false
    });
}

/// macOS: mounts by their `WKWebView`'s address, for the open-panel gate in Wry (`wry::file_panel`).
#[cfg(target_os = "macos")]
static VIEWS: LazyLock<Mutex<HashMap<usize, String>>> = LazyLock::new(Mutex::default);

/// macOS: the open panel Wry would show for this web view is refused and reported during an agent act, and (macOS
/// 13+) full screen that any of its documents enters during an act is reported and left at once.
#[cfg(target_os = "macos")]
pub(crate) fn install(address: usize, label: String) {
    static GATE: std::sync::Once = std::sync::Once::new();
    let watched = label.clone();
    vw_platform::webview::full_screen_mac::watch(
        address as *mut std::ffi::c_void,
        Box::new(move |state| full_screen_changed(&watched, address, state)),
    );
    GATE.call_once(|| {
        wry::file_panel::set_file_panel_gate(Box::new(|address| {
            let label = VIEWS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&address)
                .cloned();
            let Some(label) = label.filter(|label| agent_acting(label)) else {
                return false;
            };
            refused(&label, ActRefusal::FilePicker);
            true
        }));
    });
    VIEWS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(address, label);
}

#[cfg(target_os = "macos")]
thread_local! {
    /// macOS 13+: mounts whose current full screen is being left, since when (main thread).
    static LEAVING: std::cell::RefCell<HashMap<String, Instant>> = std::cell::RefCell::default();
}

/// macOS 13+: the view's own full-screen state changed (main thread). Seen by the engine for every document and
/// frame, whatever script ran there. During an agent act the entry is reported once and left at once; the time the
/// page held full screen and whether any app window stayed in macOS full screen are logged.
#[cfg(target_os = "macos")]
fn full_screen_changed(
    label: &str,
    address: usize,
    state: vw_platform::webview::full_screen_mac::FullScreenState,
) {
    use vw_platform::webview::full_screen_mac::{self as fs, FullScreenState};
    if let Some(since) = LEAVING.with(|l| l.borrow().get(label).copied()) {
        // The measurement of an act's full screen: every state until it is left.
        tracing::info!(
            "agent act full screen: {state:?} at {} ms; app windows in full screen: {}",
            since.elapsed().as_millis(),
            fs::windows_in_full_screen()
        );
    }
    match state {
        FullScreenState::Entering | FullScreenState::In => {
            let leaving = LEAVING.with(|l| l.borrow().contains_key(label));
            if !leaving && !agent_acting(label) {
                return;
            }
            if !leaving {
                LEAVING.with(|l| l.borrow_mut().insert(label.to_owned(), Instant::now()));
                refused(label, ActRefusal::FullScreen);
                tracing::info!(
                    "agent act full screen: {state:?}, leaving it; app windows in full screen before: {}",
                    fs::windows_in_full_screen()
                );
            }
            // Left once it is in full screen, the way a page leaves it (the top document's exit, from the agent's
            // world, which the page cannot change). Ending it while it is still entering left the web view
            // without its window on macOS 27 (the preview was gone until it reopened).
            if state == FullScreenState::In {
                vw_platform::webview::agent_world_mac::evaluate_main(
                    address as *mut std::ffi::c_void,
                    "document.fullscreenElement ? (document.exitFullscreen(), true) : false",
                    false,
                    Box::new(|_| {}),
                );
            }
        }
        FullScreenState::Not => {
            if let Some(since) = LEAVING.with(|l| l.borrow_mut().remove(label)) {
                tracing::info!(
                    "agent act full screen left after {} ms; app windows still in full screen: {}",
                    since.elapsed().as_millis(),
                    fs::windows_in_full_screen()
                );
            }
        }
        FullScreenState::Exiting => {}
    }
}

#[cfg(target_os = "macos")]
fn forget_view(label: &str) {
    VIEWS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|_, mount| mount != label);
}

/// Windows: full screen a page entered during an agent act is reported and to be left again at once (`WebView2`
/// cannot refuse it before it starts).
#[cfg(windows)]
pub(crate) fn full_screen_entered(label: &str) -> bool {
    if !agent_acting(label) {
        return false;
    }
    refused(label, ActRefusal::FullScreen);
    true
}

/// Windows: reports a picker the engine intercepted (only during agent acts).
#[cfg(windows)]
pub(crate) fn picker_refused(label: &str) {
    refused(label, ActRefusal::FilePicker);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_lasts_longer_than_the_user_activation_and_a_stale_end_never_ends_a_newer_act() {
        assert!(AFTER > Duration::from_secs(5));
        let first = acting("act-window-test", None, None);
        let stale = first.generation;
        drop(first);
        assert!(agent_acting("act-window-test"));
        let second = acting("act-window-test", None, None);
        // The first act's end, applied while the second runs, or after it: never.
        assert!(!may_end("act-window-test", stale));
        let current = second.generation;
        drop(second);
        assert!(!may_end("act-window-test", stale));
        assert!(
            !may_end("act-window-test", current),
            "still inside the activation"
        );
        windows().get_mut("act-window-test").unwrap().until = Instant::now();
        assert!(may_end("act-window-test", current));
        assert!(!may_end("act-window-test", stale));
        forget("act-window-test");
    }

    #[test]
    fn a_press_while_the_agents_input_is_in_flight_never_lifts_the_restrictions() {
        let label = "act-window-in-flight";
        let act = acting(label, None, None);
        // Before the dispatch (a press already down, or made while the act starts): never counts.
        person_input(label, true);
        dispatch(label, act.generation);
        person_input(label, false);
        // The press arrives while the agent's input is being dispatched.
        person_input(label, true);
        assert!(
            agent_acting(label),
            "the agent's input is still being dispatched"
        );
        drop(act);
        assert!(
            !agent_acting(label),
            "the window ends the moment the input returns, not AFTER later"
        );
        forget(label);
    }

    #[test]
    fn only_a_press_after_the_dispatch_counts() {
        let label = "act-window-press-only";
        let act = acting(label, None, None);
        person_input(label, true);
        dispatch(label, act.generation);
        drop(act);
        assert!(
            agent_acting(label),
            "a press from before the dispatch does not end it"
        );
        person_input(label, false);
        assert!(agent_acting(label), "a release does not end it");
        person_input(label, true);
        assert!(!agent_acting(label));
        forget(label);
    }

    #[test]
    fn a_lifted_window_is_started_again_before_the_dispatch() {
        let label = "act-window-lifted";
        let act = acting(label, None, None);
        with_window(label, act.generation, |w| w.lifted = true);
        // What `during` checks right before the input; `start` clears it for this act only.
        assert_eq!(with_window(label, act.generation, |w| w.lifted), Some(true));
        assert_eq!(with_window(label, act.generation - 1, |w| w.lifted), None);
        drop(act);
        forget(label);
    }

    fn dispatch(label: &str, generation: u64) {
        with_window(label, generation, |w| w.dispatched = Some(Instant::now()));
    }

    #[test]
    fn the_persons_press_after_the_input_ends_the_window_and_the_next_act_has_its_own() {
        let act = acting("act-window-person", None, None);
        dispatch("act-window-person", act.generation);
        drop(act);
        assert!(agent_acting("act-window-person"));
        person_input("act-window-person", true);
        assert!(
            !agent_acting("act-window-person"),
            "a press after the input ends the window at once"
        );
        let next = acting("act-window-person", None, None);
        assert!(
            agent_acting("act-window-person"),
            "the next agent act has its own window"
        );
        drop(next);
        forget("act-window-person");
    }
}
