//! Native browser facts for app status and launch. Discovery and bounded process probes run on a background worker;
//! synchronous snapshots never wait for a subprocess. Callers keep pairing and authorization policy themselves.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
#[path = "browser_discovery.rs"]
mod discovery;
#[path = "browser_launch.rs"]
mod launch;
#[path = "browser_process.rs"]
mod process;
#[cfg(any(windows, test))]
#[path = "browser_windows.rs"]
mod windows;
pub use launch::{launch_command, launch_command_with_profile};

/// A supported browser identity; Chrome and Edge facts are always kept separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    /// Google Chrome.
    Chrome,
    /// Microsoft Edge.
    Edge,
}
impl Browser {
    fn app_path(self) -> &'static str {
        match self {
            Self::Chrome => "Google Chrome.app/Contents/MacOS/Google Chrome",
            Self::Edge => "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        }
    }
    fn windows_path(self) -> &'static str {
        match self {
            Self::Chrome => "Google/Chrome/Application/chrome.exe",
            Self::Edge => "Microsoft/Edge/Application/msedge.exe",
        }
    }
}
/// Why a native browser executable was not found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallationIssue {
    /// No supported installation.
    Missing,
    /// Only a container wrapper was found.
    Sandboxed,
    /// Only a different Chromium browser was found.
    DifferentBrowser,
}
/// Result of the bounded process/window query; unknown never proves that a browser has closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// A matching process (a visible current-session window on Windows) exists.
    Running,
    /// The query completed and found none.
    Stopped,
    /// The query exceeded its deadline and was killed and reaped.
    TimedOut,
    /// The query could not run or failed.
    Unknown,
}
/// Browser facts, independent of extension state.
#[derive(Debug, Clone)]
pub struct Facts {
    /// A verified executable path or a typed installation issue.
    pub executable: Result<PathBuf, InstallationIssue>,
    /// Bounded OS process observation.
    pub process: ProcessState,
    /// POSIX process ids from a successful query; absent when unknown or unsupported.
    pub process_ids: Option<Vec<u32>>,
}
/// Find a native executable. Launchers use this on their blocking worker, immediately before spawn.
///
/// # Errors
/// Returns the installation issue when no native executable is available.
pub fn discover(browser: Browser, home: &Path) -> Result<PathBuf, InstallationIssue> {
    discovery::discover(browser, home)
}
/// Finds a macOS browser in the system or user's Applications folder, never in its data directory.
/// A stale `LaunchServices` registration cannot establish presence.
#[must_use]
pub fn macos_executable(browser: Browser, home: &Path) -> Option<PathBuf> {
    [PathBuf::from("/Applications"), home.join("Applications")]
        .into_iter()
        .map(|root| root.join(browser.app_path()))
        .find(|path| discovery::executable(path))
}
#[derive(Default)]
struct State {
    last: Option<(Facts, Instant)>,
    running: bool,
}
#[derive(Default)]
struct Slot {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
}
/// Per-app cache: at most one worker per browser, with facts refreshed after two seconds.
#[derive(Clone, Default)]
pub struct Detection([Arc<Slot>; 2]);
impl Detection {
    /// Read the last facts, starting a background refresh if needed. Never waits for OS probing.
    #[must_use]
    pub fn snapshot(&self, browser: Browser, home: &Path) -> Option<Facts> {
        let slot = &self.0[usize::from(browser == Browser::Edge)];
        let mut state = slot.state.lock().unwrap_or_else(PoisonError::into_inner);
        let last = state.last.as_ref().map(|(facts, _)| facts.clone());
        if !state.running
            && state
                .last
                .as_ref()
                .is_none_or(|(_, at)| at.elapsed() >= Duration::from_secs(2))
        {
            state.running = true;
            let slot = slot.clone();
            let home = home.to_owned();
            if std::thread::Builder::new()
                .name("browser-detection".into())
                .spawn(move || {
                    let executable = discover(browser, &home);
                    let (process, process_ids) = process::inspect(browser);
                    let mut state = slot.state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.last = Some((
                        Facts {
                            executable,
                            process,
                            process_ids,
                        },
                        Instant::now(),
                    ));
                    state.running = false;
                    drop(state);
                    slot.changed.notify_waiters();
                })
                .is_err()
            {
                state.running = false;
            }
        }
        last
    }
    /// Await a needed refresh without occupying an async worker; fresh cached observations return immediately.
    pub async fn ready(&self, browser: Browser, home: &Path) -> Option<Facts> {
        let slot = &self.0[usize::from(browser == Browser::Edge)];
        let changed = slot.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let _ = self.snapshot(browser, home);
        {
            let state = slot.state.lock().unwrap_or_else(PoisonError::into_inner);
            if !state.running {
                return state.last.as_ref().map(|(facts, _)| facts.clone());
            }
        }
        changed.await;
        slot.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last
            .as_ref()
            .map(|(facts, _)| facts.clone())
    }
}
/// Opens Safari in front, requested by the calling app while it is the active one (the person just clicked it).
/// macOS 14+ uses cooperative activation, where the active app hands activation to another app
/// (<https://developer.apple.com/documentation/appkit/nsapplication/yieldactivation(toapplicationwithbundleidentifier:)>);
/// an `NSWorkspace` open with `activates` from the active app brings an already running Safari forward (checked
/// natively on macOS 27). When the open has to launch Safari, that activation is sometimes lost and Safari's first
/// window stays behind this app, so once the launch has finished this app hands activation over again
/// (`activateFromApplication:options:`, only while this app is still the active one: a person who moved on keeps
/// the app they chose). The embedded Safari setup tool cannot be relied on for this: on macOS 27 it exits before its
/// own open when its extension-state query fails. `false` when Safari is not found. `NSWorkspace` and
/// `NSRunningApplication` are thread-safe; the open is async. `handed_over` runs on a worker thread, only when such a
/// hand-over was attempted, with whether the system allowed it.
#[cfg(target_os = "macos")]
pub fn bring_safari_forward(handed_over: impl FnOnce(bool) + Send + 'static) -> bool {
    use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
    let workspace = NSWorkspace::sharedWorkspace();
    let Some(safari) = workspace.URLForApplicationWithBundleIdentifier(safari_id()) else {
        return false;
    };
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    configuration.setActivates(true);
    let handed_over = Mutex::new(Some(handed_over));
    let opened = block2::RcBlock::new(
        move |_: *mut NSRunningApplication, _: *mut objc2_foundation::NSError| {
            let Some(handed_over) = handed_over
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            else {
                return;
            };
            let _ = std::thread::Builder::new()
                .name("safari-forward".into())
                .spawn(move || {
                    let running =
                        NSRunningApplication::runningApplicationsWithBundleIdentifier(safari_id());
                    if let Some(allowed) = running
                        .firstObject()
                        .and_then(|app| hand_over_activation(&app))
                    {
                        handed_over(allowed);
                    }
                });
        },
    );
    workspace.openApplicationAtURL_configuration_completionHandler(
        &safari,
        &configuration,
        Some(&opened),
    );
    true
}

#[cfg(target_os = "macos")]
fn safari_id() -> &'static objc2_foundation::NSString {
    objc2_foundation::ns_string!("com.apple.Safari")
}

/// Whether Safari is running now; a launch in progress counts.
#[cfg(target_os = "macos")]
#[must_use]
pub fn safari_running() -> bool {
    !objc2_app_kit::NSRunningApplication::runningApplicationsWithBundleIdentifier(safari_id())
        .is_empty()
}

/// Waits (bounded) until `app` has finished launching, then activates it as if from this app when the launch left it
/// behind and this app is still the active one. `None` when no hand-over was needed or allowed to be attempted.
#[cfg(target_os = "macos")]
fn hand_over_activation(app: &objc2_app_kit::NSRunningApplication) -> Option<bool> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.isFinishedLaunching() && !app.isTerminated() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let current = NSRunningApplication::currentApplication();
    if app.isTerminated() || app.isActive() || !current.isActive() {
        return None;
    }
    Some(app.activateFromApplication_options(
        &current,
        NSApplicationActivationOptions::ActivateAllWindows,
    ))
}
#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
