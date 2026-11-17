//! What Connect opens for Firefox sharing: the pinned bundled add-on (copied into Snap Firefox's readable folder
//! when needed), Firefox's Add-ons page, or a Mozilla listing. Depends on setup and is called by `firefox_setup`.
//! Invariant: every file address comes from bytes matching the release pin; other addresses are refused.
#[cfg(target_os = "linux")]
use super::setup;
use super::setup::ChromeControlConfig;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

/// Firefox's own add-on manager; development builds without the signed file keep it.
pub(super) const ADDONS_PAGE: &str = "about:addons";
const LISTING: &str = "https://addons.mozilla.org/";
const DAMAGED: &str = "The signed Firefox add-on in Vibe Work's folder is missing or changed. Reinstall Vibe Work, then try Connect again.";
const INVALID: &str =
    "The Firefox add-on listing is invalid. Use the signed Vibe Work add-on in Firefox Add-ons.";

/// The bundled signed add-on when this build ships it; an error when its bytes are not the pinned ones.
fn bundled(config: &ChromeControlConfig) -> Result<Option<(&Path, Vec<u8>)>, &'static str> {
    use sha2::{Digest, Sha256};
    let Some(path) = &config.firefox_addon else {
        return Ok(None);
    };
    let pin = config.firefox_addon_sha256.as_deref().ok_or(DAMAGED)?;
    let bytes = std::fs::read(path).map_err(|_| DAMAGED)?;
    if format!("{:x}", Sha256::digest(&bytes)) != pin {
        return Err(DAMAGED);
    }
    Ok(Some((path, bytes)))
}

fn file_url(path: &Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
}

#[cfg(target_os = "linux")]
fn snap_firefox(binary: &Path) -> bool {
    use std::io::Read;
    let mut path = binary.to_owned();
    for _ in 0..8 {
        if path.starts_with("/snap")
            || matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("snap" | "snapd")
            )
        {
            return true;
        }
        match std::fs::read_link(&path) {
            Ok(next) => path = path.parent().map_or(next.clone(), |dir| dir.join(&next)),
            Err(_) => break,
        }
    }
    let mut head = Vec::new();
    std::fs::File::open(&path)
        .and_then(|file| file.take(4096).read_to_end(&mut head))
        .is_ok()
        && head.starts_with(b"#!")
        && head
            .windows(b"/snap/bin/firefox".len())
            .any(|part| part == b"/snap/bin/firefox")
}

#[cfg(target_os = "linux")]
fn snap_copy(config: &ChromeControlConfig, bytes: &[u8]) -> Result<PathBuf, &'static str> {
    let pin = config.firefox_addon_sha256.as_deref().ok_or(DAMAGED)?;
    let home = setup::home_of(config).map_err(|_| DAMAGED)?;
    let path = home
        .join("snap/firefox/common/vibe-work")
        .join(pin)
        .join("vibe-work.xpi");
    match setup::files::matches(config, &path, bytes) {
        Ok(true) => return Ok(path),
        Ok(false) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err(DAMAGED),
    }
    setup::files::write(config, &path, None, bytes).map_err(|_| DAMAGED)?;
    if setup::files::matches(config, &path, bytes).map_err(|_| DAMAGED)? {
        Ok(path)
    } else {
        Err(DAMAGED)
    }
}

/// Whether Firefox may be handed `target`: the exact bundled file, the Add-ons page, or a Mozilla listing.
pub(super) fn allowed(config: &ChromeControlConfig, target: &str) -> bool {
    target == ADDONS_PAGE
        || target.starts_with(LISTING)
        || config
            .firefox_addon
            .as_deref()
            .and_then(file_url)
            .is_some_and(|exact| exact == target)
}

/// What Connect opens. An installed add-on opens its management page; otherwise the pinned signed file is offered
/// through a path the selected Firefox can read, which makes Firefox show its own install prompt.
pub(super) fn target_for(
    config: &ChromeControlConfig,
    installed: bool,
    executable: Option<&Path>,
) -> Result<String, String> {
    #[cfg(not(target_os = "linux"))]
    let _ = executable;
    let fallback = || {
        config
            .firefox_store_url
            .clone()
            .unwrap_or_else(|| ADDONS_PAGE.to_owned())
    };
    if !installed && let Some((path, bytes)) = bundled(config)? {
        #[cfg(target_os = "linux")]
        let path = if executable.is_some_and(snap_firefox) {
            snap_copy(config, &bytes)?
        } else {
            path.to_owned()
        };
        #[cfg(not(target_os = "linux"))]
        let path = {
            let _ = bytes;
            path.to_owned()
        };
        return file_url(&path).ok_or_else(|| DAMAGED.to_owned());
    }
    let target = fallback();
    if allowed(config, &target) {
        Ok(target)
    } else {
        Err(INVALID.to_owned())
    }
}

#[cfg(test)]
fn target(config: &ChromeControlConfig, installed: bool) -> Result<String, String> {
    target_for(config, installed, None)
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::target_for;
    use super::{ADDONS_PAGE, allowed, target};
    use crate::chrome_control::setup::ChromeControlConfig;
    use sha2::{Digest, Sha256};

    const SIGNED: &[u8] = b"signed add-on fixture bytes";

    fn shipped(dir: &std::path::Path) -> ChromeControlConfig {
        let mut config = ChromeControlConfig {
            firefox_addon_sha256: Some(format!("{:x}", Sha256::digest(SIGNED))),
            ..ChromeControlConfig::default()
        };
        config.bundled(dir);
        let file = config
            .firefox_addon
            .clone()
            .expect("a pinned build ships the file");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, SIGNED).unwrap();
        config
    }

    fn file_url(config: &ChromeControlConfig) -> String {
        url::Url::from_file_path(config.firefox_addon.as_ref().unwrap())
            .unwrap()
            .to_string()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snap_firefox_gets_the_pinned_add_on_inside_its_readable_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = shipped(dir.path());
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        config.user_home = Some(home.clone());
        let wrapper = dir.path().join("firefox");
        std::fs::write(&wrapper, b"#!/bin/sh\nexec /snap/bin/firefox \"$@\"\n").unwrap();
        let target = target_for(&config, false, Some(&wrapper)).unwrap();
        let staged = url::Url::parse(&target).unwrap().to_file_path().unwrap();
        assert_eq!(staged.file_name().unwrap(), "vibe-work.xpi");
        assert!(
            staged.starts_with(home.join("snap/firefox/common")),
            "{target}"
        );
        assert_eq!(std::fs::read(staged).unwrap(), SIGNED);
        assert_eq!(
            target_for(&config, true, Some(&wrapper)).unwrap(),
            ADDONS_PAGE
        );
        let native = dir.path().join("firefox-native");
        std::fs::write(&native, b"\x7fELF").unwrap();
        assert_eq!(
            target_for(&config, false, Some(&native)).unwrap(),
            file_url(&config)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn snap_staging_refuses_changed_or_linked_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = shipped(dir.path());
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        config.user_home = Some(home);
        let wrapper = dir.path().join("firefox");
        std::fs::write(&wrapper, b"#!/bin/sh\nexec /snap/bin/firefox \"$@\"\n").unwrap();
        let target = target_for(&config, false, Some(&wrapper)).unwrap();
        let staged = url::Url::parse(&target).unwrap().to_file_path().unwrap();
        std::fs::write(&staged, b"changed").unwrap();
        assert!(target_for(&config, false, Some(&wrapper)).is_err());
        std::fs::remove_file(&staged).unwrap();
        std::os::unix::fs::symlink(config.firefox_addon.as_ref().unwrap(), &staged).unwrap();
        assert!(target_for(&config, false, Some(&wrapper)).is_err());
    }

    #[test]
    fn a_development_build_without_the_signed_file_keeps_firefox_add_ons() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = ChromeControlConfig {
            firefox_addon_sha256: None,
            ..ChromeControlConfig::default()
        };
        config.bundled(dir.path());
        assert_eq!(config.firefox_addon, None);
        assert_eq!(target(&config, false).unwrap(), ADDONS_PAGE);
        assert_eq!(target(&config, true).unwrap(), ADDONS_PAGE);
    }

    #[test]
    fn a_release_opens_its_bundled_signed_file_until_the_add_on_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        let config = shipped(dir.path());
        assert_eq!(
            config.firefox_addon.as_deref(),
            Some(dir.path().join("firefox-addon/vibe-work.xpi").as_path())
        );
        let url = target(&config, false).unwrap();
        assert!(url.starts_with("file:///"), "{url}");
        assert_eq!(url, file_url(&config));
        // Installed (connected or turned off): its management page, not a second install prompt.
        assert_eq!(target(&config, true).unwrap(), ADDONS_PAGE);
    }

    #[test]
    fn a_changed_or_missing_bundled_file_is_refused_not_replaced_by_the_add_ons_page() {
        let dir = tempfile::tempdir().unwrap();
        let config = shipped(dir.path());
        let file = config.firefox_addon.clone().unwrap();
        std::fs::write(&file, b"changed").unwrap();
        let refused = target(&config, false).unwrap_err();
        assert!(refused.contains("missing or changed"), "{refused}");
        std::fs::remove_file(&file).unwrap();
        assert!(target(&config, false).is_err());
    }

    #[test]
    fn only_the_exact_bundled_file_the_add_ons_page_or_a_mozilla_listing_may_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let config = shipped(dir.path());
        let exact = file_url(&config);
        assert!(allowed(&config, &exact));
        assert!(allowed(&config, ADDONS_PAGE));
        assert!(allowed(
            &config,
            "https://addons.mozilla.org/firefox/addon/vibe-work/"
        ));
        let sibling = url::Url::from_file_path(dir.path().join("firefox-addon/other.xpi")).unwrap();
        for refused in [
            sibling.as_str(),
            &format!("{exact}?x=1"),
            "file:///tmp/vibe-work.xpi",
            "https://addons.mozilla.org.example/x",
            "http://addons.mozilla.org/",
            "about:config",
        ] {
            assert!(!allowed(&config, refused), "{refused}");
        }
        // A build without the bundled file allows no file address at all.
        let plain = ChromeControlConfig {
            firefox_addon_sha256: None,
            ..ChromeControlConfig::default()
        };
        assert!(!allowed(&plain, &exact));
        // A configured listing outside Mozilla's site is refused before Firefox starts.
        let listed = ChromeControlConfig {
            firefox_store_url: Some("https://example.com/vibe-work.xpi".into()),
            firefox_addon_sha256: None,
            ..ChromeControlConfig::default()
        };
        assert!(target(&listed, false).is_err());
    }
}
