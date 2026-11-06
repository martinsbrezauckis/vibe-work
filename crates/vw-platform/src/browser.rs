//! Native browser facts for app status and launch. Discovery and bounded process probes run on a background worker;
//! synchronous snapshots never wait for a subprocess. Callers keep pairing and authorization policy themselves.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
#[path = "browser_discovery.rs"]
mod discovery;
#[path = "browser_process.rs"]
mod process;

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
#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
