//! Finds Chrome and Edge and opens their trusted pages with the complete address as one argument, without a shell.
//! Depends on local browser paths and injected launchers; called by setup and enrollment. Chrome setup/status
//! share one probe; unrelated Chromium browsers and Linux container packages never receive its registration.
use std::io::Read;
use std::path::{Path, PathBuf};

use super::setup::ChromeControlConfig;

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
            Self::Missing => "Google Chrome not found",
            Self::Sandboxed => "Snap or Flatpak Chrome is unsupported",
            Self::DifferentBrowser => "Google Chrome required",
        }
    }
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

fn on_path(name: &str) -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).map(|p| p.join(name)).collect())
        .unwrap_or_default()
}

fn sandboxed(path: &Path) -> bool {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    if resolved
        .components()
        .any(|c| c.as_os_str() == "snap" || c.as_os_str() == "flatpak")
    {
        return true;
    }
    // Distribution wrappers may sit in /usr/bin while invoking a container package.
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(8192).read_to_end(&mut bytes);
    }
    bytes.starts_with(b"#!")
        && String::from_utf8_lossy(&bytes).lines().any(|line| {
            !line.trim_start().starts_with('#')
                && (line.contains("/snap/")
                    || line.contains("flatpak run")
                    || line.contains("snap run"))
        })
}

pub(super) fn find(config: &ChromeControlConfig) -> Result<PathBuf, BrowserIssue> {
    let home = super::setup::home_of(config).unwrap_or_default();
    let candidates = if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            home.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        ]
    } else if cfg!(windows) {
        let mut paths = vec![home.join("AppData/Local/Google/Chrome/Application/chrome.exe")];
        for name in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Some(root) = std::env::var_os(name) {
                paths.push(PathBuf::from(root).join("Google/Chrome/Application/chrome.exe"));
            }
        }
        paths
    } else {
        [on_path("google-chrome"), on_path("google-chrome-stable")].concat()
    };
    let mut found_sandbox = false;
    for path in candidates.into_iter().filter(|p| executable(p)) {
        if cfg!(target_os = "linux") && sandboxed(&path) {
            found_sandbox = true;
        } else {
            return Ok(path);
        }
    }
    if cfg!(target_os = "linux") {
        found_sandbox |= [
            home.join(".local/share/flatpak/exports/bin/com.google.Chrome"),
            PathBuf::from("/var/lib/flatpak/exports/bin/com.google.Chrome"),
            PathBuf::from("/snap/bin/chromium"),
        ]
        .iter()
        .any(|p| executable(p));
        if found_sandbox {
            return Err(BrowserIssue::Sandboxed);
        }
        for name in [
            "chromium",
            "chromium-browser",
            "brave-browser",
            "microsoft-edge",
            "vivaldi",
        ] {
            if let Some(path) = on_path(name).into_iter().find(|p| executable(p)) {
                return Err(if sandboxed(&path) {
                    BrowserIssue::Sandboxed
                } else {
                    BrowserIssue::DifferentBrowser
                });
            }
        }
    }
    let other = if cfg!(target_os = "macos") {
        ["Chromium", "Brave Browser", "Microsoft Edge"]
            .into_iter()
            .flat_map(|name| {
                [PathBuf::from("/Applications"), home.join("Applications")]
                    .map(|root| root.join(format!("{name}.app/Contents/MacOS/{name}")))
            })
            .collect::<Vec<_>>()
    } else if cfg!(windows) {
        ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"]
            .into_iter()
            .filter_map(std::env::var_os)
            .flat_map(|root| {
                [
                    "Chromium/Application/chrome.exe",
                    "BraveSoftware/Brave-Browser/Application/brave.exe",
                    "Microsoft/Edge/Application/msedge.exe",
                ]
                .map(|path| PathBuf::from(&root).join(path))
            })
            .collect()
    } else {
        Vec::new()
    };
    Err(if other.iter().any(|path| executable(path)) {
        BrowserIssue::DifferentBrowser
    } else {
        BrowserIssue::Missing
    })
}

pub(super) fn issue(config: &ChromeControlConfig) -> Option<BrowserIssue> {
    // The injected launcher is also the browser boundary for isolated tests.
    if config.launcher.is_some() {
        None
    } else {
        find(config).err()
    }
}

/// Chrome's arguments for opening `url`: the address only. No profile flag: Vibe Work cannot verify which profile
/// directory holds the extension's profile, so Chrome opens it in the profile it would use anyway.
pub(super) fn launch_args(url: &str) -> [std::ffi::OsString; 1] {
    [url.into()]
}

/// Whether Chrome can present external-install consent without opening another window.
pub(super) fn running() -> bool {
    running_for(false)
}

/// Edge's own window/process probe: Chrome running does not mean Edge is open.
pub(super) fn edge_running() -> bool {
    running_for(true)
}

fn running_for(edge: bool) -> bool {
    let process = if edge { "msedge" } else { "chrome" };
    if cfg!(windows) {
        // Chrome may keep background processes after its last window closes. A process-only check
        // would leave the enable prompt with no window to appear in. Query this desktop session only.
        let mut probe = std::process::Command::new("powershell.exe");
        let script = format!(
            "$s = (Get-Process -Id $PID).SessionId; if (@(Get-Process {process} -ErrorAction SilentlyContinue | Where-Object {{ $_.SessionId -eq $s -and $_.MainWindowHandle -ne 0 }}).Count -gt 0) {{ exit 0 }}; exit 1"
        );
        probe.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            probe.creation_flags(0x0800_0000);
        }
        probe
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/pgrep")
            .args([
                "-x",
                if edge {
                    "Microsoft Edge"
                } else {
                    "Google Chrome"
                },
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    } else {
        std::process::Command::new("pgrep")
            .args(["-x", process])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

pub(super) fn open(config: &ChromeControlConfig, url: &str) -> Result<(), String> {
    if let Some(launcher) = &config.launcher {
        return launcher(url);
    }
    let browser = find(config).map_err(|issue| issue.label().to_owned())?;
    launch(browser, url, "Google Chrome could not be opened")
}

/// Opens Edge's own pairing page, passing the complete address as one argument without a command shell.
pub(super) fn open_edge(config: &ChromeControlConfig, url: &str) -> Result<(), String> {
    if !cfg!(feature = "your-edge") {
        return Err(super::setup::absent_for(true).to_owned());
    }
    if let Some(launcher) = &config.edge_launcher {
        return launcher(url);
    }
    let home = super::setup::home_of(config).unwrap_or_default();
    let candidates = if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
            home.join("Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        ]
    } else if cfg!(windows) {
        let mut paths = vec![home.join("AppData/Local/Microsoft/Edge/Application/msedge.exe")];
        for name in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Some(root) = std::env::var_os(name) {
                paths.push(PathBuf::from(root).join("Microsoft/Edge/Application/msedge.exe"));
            }
        }
        paths
    } else {
        [on_path("microsoft-edge"), on_path("microsoft-edge-stable")].concat()
    };
    let browser = candidates
        .into_iter()
        .find(|path| executable(path))
        .ok_or("Microsoft Edge not found")?;
    launch(browser, url, "Microsoft Edge could not be opened")
}

fn launch(browser: PathBuf, url: &str, failure: &'static str) -> Result<(), String> {
    let args = launch_args(url);
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("browser-launch".into())
        .spawn(move || {
            let child = std::process::Command::new(browser)
                .args(args.into_iter().filter(|arg| !arg.is_empty()))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            let _ = sent.send(child.is_ok());
            if let Ok(mut child) = child {
                let _ = child.wait();
            }
        })
        .map_err(|_| failure.to_owned())?;
    if received.recv() == Ok(true) {
        Ok(())
    } else {
        Err(failure.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_wrappers_are_not_native_chrome() {
        let root = std::env::temp_dir().join(format!("chrome-probe-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let wrapper = root.join("google-chrome");
        for script in [
            "#!/bin/sh\nexec /snap/bin/chromium \"$@\"",
            "#!/bin/sh\nexec flatpak run com.google.Chrome \"$@\"",
        ] {
            std::fs::write(&wrapper, script).unwrap();
            assert!(sandboxed(&wrapper));
        }
        std::fs::write(
            &wrapper,
            "#!/bin/sh\n# not a flatpak run\nexec /opt/google/chrome/chrome \"$@\"",
        )
        .unwrap();
        assert!(!sandboxed(&wrapper));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "browser_reap_tests.rs"]
mod reap_tests;
