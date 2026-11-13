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
    let mut chrome = if config.is_macos() {
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
        let base =
            linux_config(home, config, std::env::var_os("XDG_CONFIG_HOME")).join("google-chrome");
        vec![Target {
            name: "Google Chrome",
            hosts: Some(base.join("NativeMessagingHosts")),
            user_data: base,
            registry: None,
        }]
    };
    if cfg!(feature = "your-edge") && super::super::edge_policy::refusal(config).is_none() {
        let user_data = edge_data_for(home, config);
        chrome.push(Target {
            name: "Microsoft Edge",
            hosts: (!cfg!(windows)).then(|| user_data.join("NativeMessagingHosts")),
            user_data,
            registry: cfg!(windows).then_some(config.edge_registry_root.as_str()),
        });
    }
    if cfg!(feature = "your-firefox") {
        let (user_data, hosts) = if config.is_macos() {
            (
                home.join("Library/Application Support/Firefox"),
                home.join("Library/Application Support/Mozilla/NativeMessagingHosts"),
            )
        } else if cfg!(windows) {
            (home.join("AppData/Roaming/Mozilla/Firefox"), PathBuf::new())
        } else {
            (
                home.join(".mozilla/firefox"),
                home.join(".mozilla/native-messaging-hosts"),
            )
        };
        chrome.push(Target {
            name: "Mozilla Firefox",
            user_data,
            hosts: (!cfg!(windows)).then_some(hosts),
            registry: cfg!(windows).then_some(config.firefox_registry_root.as_str()),
        });
    }
    chrome
}

/// The folder Chrome and Edge on Linux keep their user data in, and read per-user native messaging manifests
/// from: `xdg` (`$XDG_CONFIG_HOME`) when it is an absolute path, else `~/.config`. A scratch home (`user_home`)
/// never takes the process environment's folder.
fn linux_config(
    home: &Path,
    config: &ChromeControlConfig,
    xdg: Option<std::ffi::OsString>,
) -> PathBuf {
    xdg.filter(|_| config.user_home.is_none())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
}

pub(super) fn edge_data_for(home: &Path, config: &ChromeControlConfig) -> PathBuf {
    if config.is_macos() {
        home.join("Library/Application Support/Microsoft Edge")
    } else if cfg!(windows) {
        home.join("AppData")
            .join("Local")
            .join("Microsoft")
            .join("Edge")
            .join("User Data")
    } else {
        linux_config(home, config, std::env::var_os("XDG_CONFIG_HOME")).join("microsoft-edge")
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

/// App bundles establish presence. Profile folders and stale registrations never do.
pub(super) fn installed(config: &ChromeControlConfig, browser: &str) -> bool {
    if !config.is_macos() {
        return true;
    }
    if browser == "Mozilla Firefox" {
        return super::home_of(config).is_ok_and(|home| {
            [
                PathBuf::from("/Applications/Firefox.app/Contents/MacOS/firefox"),
                home.join("Applications/Firefox.app/Contents/MacOS/firefox"),
            ]
            .iter()
            .any(|path| path.is_file())
        });
    }
    let browser = match browser {
        "Google Chrome" => vw_platform::browser::Browser::Chrome,
        "Microsoft Edge" => vw_platform::browser::Browser::Edge,
        _ => return false,
    };
    super::home_of(config)
        .is_ok_and(|home| vw_platform::browser::macos_executable(browser, &home).is_some())
}

pub(super) fn require_installed(config: &ChromeControlConfig, browser: &str) -> Result<(), String> {
    if installed(config, browser) {
        Ok(())
    } else {
        Err(format!("{browser} is not installed"))
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;

    /// Child fixture: prints where Chrome's and Edge's manifests go for the real home (no scratch home), under
    /// the environment the parent gave it.
    #[test]
    #[ignore = "child fixture: run only by the XDG_CONFIG_HOME test"]
    fn print_linux_host_folders() {
        let config = ChromeControlConfig::default();
        for target in targets(Path::new("/home/person"), &config) {
            println!(
                "TARGET {}|{}|{}",
                target.name,
                target.user_data.display(),
                target.hosts.unwrap_or_default().display()
            );
        }
    }

    fn folders(xdg: Option<&str>) -> Vec<String> {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "chrome_control::setup::places::tests::print_linux_host_folders",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]);
        command.env_remove("XDG_CONFIG_HOME");
        if let Some(xdg) = xdg {
            command.env("XDG_CONFIG_HOME", xdg);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once("TARGET ").map(|(_, row)| row.to_owned()))
            .collect()
    }

    /// Chrome and Edge on Linux keep their user data, and read per-user native messaging manifests, under
    /// `$XDG_CONFIG_HOME` when it is set to an absolute path, else under `~/.config`.
    #[test]
    fn linux_chrome_and_edge_follow_xdg_config_home() {
        let set = folders(Some("/home/person/custom-config"));
        assert!(
            set.contains(
                &"Google Chrome|/home/person/custom-config/google-chrome|/home/person/custom-config/google-chrome/NativeMessagingHosts"
                    .to_owned()
            ),
            "{set:?}"
        );
        if cfg!(feature = "your-edge") {
            assert!(
                set.contains(
                    &"Microsoft Edge|/home/person/custom-config/microsoft-edge|/home/person/custom-config/microsoft-edge/NativeMessagingHosts"
                        .to_owned()
                ),
                "{set:?}"
            );
        }
        for unset in [None, Some(""), Some("relative/config")] {
            let folders = folders(unset);
            assert!(
                folders.contains(
                    &"Google Chrome|/home/person/.config/google-chrome|/home/person/.config/google-chrome/NativeMessagingHosts"
                        .to_owned()
                ),
                "{unset:?}: {folders:?}"
            );
        }
    }

    /// A scratch home never takes the process environment's config folder.
    #[test]
    fn a_scratch_home_ignores_xdg_config_home() {
        let config = ChromeControlConfig {
            user_home: Some(PathBuf::from("/scratch/home")),
            ..ChromeControlConfig::default()
        };
        assert_eq!(
            linux_config(
                Path::new("/scratch/home"),
                &config,
                Some("/elsewhere".into())
            ),
            PathBuf::from("/scratch/home/.config")
        );
    }
}
