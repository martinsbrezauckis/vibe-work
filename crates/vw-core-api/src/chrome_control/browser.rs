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
/// Reopen the paired browser without visiting a page; a unique installed profile identifies the directory.
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
                    .map_err(|_| format!("{name} could not be opened"))
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
#[cfg(all(test, target_os = "linux"))]
#[path = "browser_reap_tests.rs"]
mod reap_tests;
