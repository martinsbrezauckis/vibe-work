//! Per-user browser registration paths from vendor documentation; used only by setup.
use super::ChromeControlConfig;
use std::path::{Path, PathBuf};

/// One Chrome this user may run: its name, its user data folder, its per-user host folder (`None` on Windows,
/// where the registry names the manifest).
pub(super) struct Target<'a> {
    pub(super) name: &'static str,
    pub(super) user_data: PathBuf,
    pub(super) hosts: Option<PathBuf>,
    pub(super) registry: Option<&'a str>,
}

pub(super) fn targets<'a>(home: &Path, config: &'a ChromeControlConfig) -> Vec<Target<'a>> {
    let mut chrome = if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support/Google/Chrome");
        vec![Target {
            name: "Google Chrome",
            hosts: Some(base.join("NativeMessagingHosts")),
            user_data: base,
            registry: None,
        }]
    } else if cfg!(windows) {
        vec![Target {
            name: "Google Chrome",
            user_data: home
                .join("AppData")
                .join("Local")
                .join("Google")
                .join("Chrome")
                .join("User Data"),
            hosts: None,
            registry: Some(&config.windows_registry_root),
        }]
    } else {
        let base = home.join(".config/google-chrome");
        vec![Target {
            name: "Google Chrome",
            hosts: Some(base.join("NativeMessagingHosts")),
            user_data: base,
            registry: None,
        }]
    };
    if cfg!(feature = "your-edge") && super::super::edge_policy::refusal(config).is_none() {
        let user_data = edge_data(home);
        chrome.push(Target {
            name: "Microsoft Edge",
            hosts: (!cfg!(windows)).then(|| user_data.join("NativeMessagingHosts")),
            user_data,
            registry: cfg!(windows).then_some(config.edge_registry_root.as_str()),
        });
    }
    chrome
}

pub(super) fn edge_data(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Microsoft Edge")
    } else if cfg!(windows) {
        home.join("AppData")
            .join("Local")
            .join("Microsoft")
            .join("Edge")
            .join("User Data")
    } else {
        home.join(".config/microsoft-edge")
    }
}

/// Recognizes legacy folders lacking a receipt identity, including vendor release channels on every OS.
/// Match whole path components so an unrelated folder containing a browser name is never attributed to it.
pub(super) fn browser_of(path: &Path) -> Option<&'static str> {
    let path = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let path = path.trim_end_matches('/');
    for (browser, vendor, channels, unix) in [
        (
            "Google Chrome",
            "google",
            &[
                "chrome",
                "chrome beta",
                "chrome dev",
                "chrome canary",
                "chrome sxs",
            ][..],
            "google-chrome",
        ),
        (
            "Microsoft Edge",
            "microsoft",
            &["edge", "edge beta", "edge dev", "edge canary", "edge sxs"][..],
            "microsoft-edge",
        ),
    ] {
        let leaf = path.rsplit('/').next().unwrap_or_default();
        if ["", "-stable", "-beta", "-dev", "-unstable", "-canary"]
            .iter()
            .any(|suffix| leaf == format!("{unix}{suffix}"))
            || channels.iter().any(|channel| {
                path.ends_with(&format!("/{vendor}/{channel}/user data"))
                    || (vendor == "google" && path.ends_with(&format!("/google/{channel}")))
                    || (vendor == "microsoft" && leaf == format!("microsoft {channel}"))
            })
        {
            return Some(browser);
        }
    }
    None
}
