//! Native Chrome/Edge paths and Linux wrapper inspection. Used by browser detection and launch, never by policy.
use super::{Browser, InstallationIssue};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(super) fn executable(path: &Path) -> bool {
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
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(name))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn sandboxed(path: &Path) -> bool {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    if resolved
        .components()
        .any(|c| c.as_os_str() == "snap" || c.as_os_str() == "flatpak")
    {
        return true;
    }
    // Distribution wrappers may sit in /usr/bin while invoking a container package.
    let mut bytes = Vec::new();
    if path.metadata().is_ok_and(|meta| meta.is_file())
        && let Ok(file) = std::fs::File::open(path)
    {
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

fn candidates(browser: Browser, home: &Path) -> Vec<PathBuf> {
    let edge = browser == Browser::Edge;
    if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications").join(browser.app_path()),
            home.join("Applications").join(browser.app_path()),
        ]
    } else if cfg!(windows) {
        let mut paths = vec![home.join("AppData/Local").join(browser.windows_path())];
        for name in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Some(root) = std::env::var_os(name) {
                paths.push(PathBuf::from(root).join(browser.windows_path()));
            }
        }
        paths
    } else if edge {
        [on_path("microsoft-edge"), on_path("microsoft-edge-stable")].concat()
    } else {
        [on_path("google-chrome"), on_path("google-chrome-stable")].concat()
    }
}

pub(super) fn discover(browser: Browser, home: &Path) -> Result<PathBuf, InstallationIssue> {
    let edge = browser == Browser::Edge;
    let candidates = candidates(browser, home);
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
            home.join(if edge {
                ".local/share/flatpak/exports/bin/com.microsoft.Edge"
            } else {
                ".local/share/flatpak/exports/bin/com.google.Chrome"
            }),
            PathBuf::from(if edge {
                "/var/lib/flatpak/exports/bin/com.microsoft.Edge"
            } else {
                "/var/lib/flatpak/exports/bin/com.google.Chrome"
            }),
            PathBuf::from(if edge {
                "/snap/bin/microsoft-edge"
            } else {
                "/snap/bin/chromium"
            }),
        ]
        .iter()
        .any(|p| executable(p));
        if found_sandbox {
            return Err(InstallationIssue::Sandboxed);
        }
        if edge {
            return Err(InstallationIssue::Missing);
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
                    InstallationIssue::Sandboxed
                } else {
                    InstallationIssue::DifferentBrowser
                });
            }
        }
    }
    if edge {
        return Err(InstallationIssue::Missing);
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
        InstallationIssue::DifferentBrowser
    } else {
        InstallationIssue::Missing
    })
}
