//! Browser policy adapters and detached launch. OS discovery/process facts live in vw-platform; launchers reap children.
//! Called by setup, enrollment and status. Injected launchers isolate tests from installed browsers.
use super::setup::ChromeControlConfig;
#[cfg(all(test, target_os = "linux"))]
use std::path::PathBuf;
use vw_platform::browser::{Browser, Facts, InstallationIssue, ProcessState};

/// A browser installation that cannot use this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", ts(rename = "ChromeBrowserIssue"))]
pub enum BrowserIssue {
    /// No Google Chrome installation was found.
    Missing,
    /// A Snap or Flatpak browser cannot reach the per-user native host.
    Sandboxed,
    /// Only a different Chromium browser was found.
    DifferentBrowser,
}

impl BrowserIssue {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Missing => "Google Chrome is not installed",
            Self::Sandboxed => "Snap or Flatpak Chrome is unsupported",
            Self::DifferentBrowser => "Google Chrome required",
        }
    }
}

impl From<InstallationIssue> for BrowserIssue {
    fn from(issue: InstallationIssue) -> Self {
        match issue {
            InstallationIssue::Missing => Self::Missing,
            InstallationIssue::Sandboxed => Self::Sandboxed,
            InstallationIssue::DifferentBrowser => Self::DifferentBrowser,
        }
    }
}
fn injected(config: &ChromeControlConfig, browser: Browser) -> bool {
    match browser {
        Browser::Chrome => config.launcher.is_some(),
        Browser::Edge => config.edge_launcher.is_some(),
    }
}
pub(super) fn snapshot(config: &ChromeControlConfig, browser: Browser) -> Option<Facts> {
    config
        .browser_detection
        .snapshot(browser, &super::setup::home_of(config).unwrap_or_default())
}
pub(super) async fn refresh(config: &ChromeControlConfig) {
    if !config.available() {
        return;
    }
    let home = super::setup::home_of(config).unwrap_or_default();
    for browser in [Browser::Chrome, Browser::Edge] {
        if (browser == Browser::Chrome || cfg!(feature = "your-edge")) && !injected(config, browser)
        {
            config.browser_detection.ready(browser, &home).await;
        }
    }
}
pub(super) fn issue_for(config: &ChromeControlConfig, browser: Browser) -> Option<BrowserIssue> {
    if injected(config, browser) {
        None
    } else {
        snapshot(config, browser)?.executable.err().map(Into::into)
    }
}
pub(super) fn issue(config: &ChromeControlConfig) -> Option<BrowserIssue> {
    issue_for(config, Browser::Chrome)
}
/// Unknown process facts cannot claim a remembered browser has closed.
pub(super) fn running(config: &ChromeControlConfig) -> bool {
    running_for(config, Browser::Chrome)
}
pub(super) fn edge_running(config: &ChromeControlConfig) -> bool {
    running_for(config, Browser::Edge)
}
fn running_for(config: &ChromeControlConfig, browser: Browser) -> bool {
    snapshot(config, browser).is_none_or(|facts| facts.process != ProcessState::Stopped)
}
pub(super) fn open(config: &ChromeControlConfig, url: &str) -> Result<(), String> {
    if let Some(launcher) = &config.launcher {
        return launcher(url);
    }
    launch(config, Browser::Chrome, url, None)
}
pub(super) fn open_edge(config: &ChromeControlConfig, url: &str) -> Result<(), String> {
    if !cfg!(feature = "your-edge") {
        return Err(super::setup::absent_for(true).to_owned());
    }
    if let Some(launcher) = &config.edge_launcher {
        return launcher(url);
    }
    launch(config, Browser::Edge, url, None)
}
/// Reopen the browser without visiting a page; authenticated reconnection still chooses the task identity.
pub(super) fn open_paired(
    shared: &crate::core::Shared,
    browser: super::agent_browser::Browser,
) -> Result<(), String> {
    let config = &shared.chrome_control.config;
    let (native, launcher) = match browser {
        super::agent_browser::Browser::Chrome => (Browser::Chrome, &config.launcher),
        super::agent_browser::Browser::Edge => (Browser::Edge, &config.edge_launcher),
        _ => return Ok(()),
    };
    if let Some(launcher) = launcher {
        return launcher("");
    }
    launch(
        config,
        native,
        "",
        super::installation::paired_directory(shared, browser),
    )
}
fn launch(
    config: &ChromeControlConfig,
    browser: Browser,
    url: &str,
    profile: Option<String>,
) -> Result<(), String> {
    let home = super::setup::home_of(config)?;
    let url = url.to_owned();
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("browser-launch".into())
        .spawn(move || {
            let name = if browser == Browser::Edge {
                "Microsoft Edge"
            } else {
                "Google Chrome"
            };
            let child = vw_platform::browser::launch_command_with_profile(
                browser,
                &home,
                &url,
                profile.as_deref(),
            )
            .map_err(|issue| match issue {
                InstallationIssue::Missing => format!("{name} is not installed"),
                InstallationIssue::Sandboxed => {
                    format!("Snap or Flatpak {name} is unsupported")
                }
                InstallationIssue::DifferentBrowser => format!("{name} required"),
            })
            .and_then(|mut command| {
                command
                    .spawn()
                    .map_err(|error| start_failed(name, &error, cfg!(windows)))
            });
            // `open` is a short LaunchServices request. Its exit status tells us whether the bundle was found;
            // the browser itself belongs to LaunchServices and is never waited on or killed by the app.
            #[cfg(target_os = "macos")]
            let child = child.and_then(|mut child| {
                if child.wait().is_ok_and(|status| status.success()) {
                    Ok(child)
                } else {
                    Err(format!("{name} could not be opened"))
                }
            });
            let _ = sent.send(child.as_ref().map(|_| ()).map_err(Clone::clone));
            if let Ok(mut child) = child {
                let _ = child.wait();
            }
        })
        .map_err(|_| "The browser could not be opened".to_owned())?;
    received
        .recv()
        .map_err(|_| "The browser could not be opened".to_owned())?
}
/// Why the browser `name` did not start (`error` from starting its program), as the person and the agent read it; the
/// OS error is logged. On Windows the app starts a browser outside its own job (`CREATE_BREAKAWAY_FROM_JOB`) so closing
/// the app never closes the person's browser; a program that started the app inside a job that forbids that (a
/// scheduled task does) makes Windows refuse with access denied. The app does not tie the browser to that job: it
/// says so, and a browser the person opened connects without being started.
fn start_failed(name: &str, error: &std::io::Error, windows: bool) -> String {
    tracing::warn!(%error, browser = name, "the browser could not be started");
    if windows && error.kind() == std::io::ErrorKind::PermissionDenied {
        vw_vocabulary::texts::BROWSER_START_REFUSED.replace("{browser}", name)
    } else {
        format!("{name} could not be opened")
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "browser_reap_tests.rs"]
mod reap_tests;

#[cfg(test)]
mod start_failed_tests {
    /// PREDICT (before the change): every failed start said only "Microsoft Edge could not be opened" and nothing was
    /// logged; an app started by a scheduled task could not start Edge (Windows refuses the job breakaway with access
    /// denied inside a task and allows it outside) and the person could not learn why or what to do.
    #[test]
    fn a_refused_start_on_windows_says_why_and_what_to_do() {
        let denied = std::io::Error::from_raw_os_error(5);
        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, denied);
        assert_eq!(
            super::start_failed("Microsoft Edge", &denied, true),
            "Microsoft Edge could not be opened: the program that started Vibe Work does not let it start a browser of \
             its own. Open Microsoft Edge yourself, then try again."
        );
        // Elsewhere, and for any other failure, the short sentence stays.
        assert_eq!(
            super::start_failed("Microsoft Edge", &denied, false),
            "Microsoft Edge could not be opened"
        );
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            super::start_failed("Google Chrome", &missing, true),
            "Google Chrome could not be opened"
        );
    }
}
