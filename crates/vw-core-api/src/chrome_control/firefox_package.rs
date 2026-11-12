//! What Connect opens for the optional Firefox sharing add-on: the signed add-on file shipped in a release bundle,
//! Firefox's Add-ons page, or a Mozilla listing. Depends on the Chrome Control config; called by `firefox_setup`.
//! Invariant: Firefox is only ever handed the bundled file whose bytes match the release pin, `about:addons`, or an
//! `https://addons.mozilla.org/` address; anything else is refused before a process starts.
use super::setup::ChromeControlConfig;

/// Firefox's own add-on manager; development builds without the signed file keep it.
pub(super) const ADDONS_PAGE: &str = "about:addons";
const LISTING: &str = "https://addons.mozilla.org/";
const DAMAGED: &str = "The signed Firefox add-on in Vibe Work's folder is missing or changed. Reinstall Vibe Work, then try Connect again.";
const INVALID: &str =
    "The Firefox add-on listing is invalid. Use the signed Vibe Work add-on in Firefox Add-ons.";

/// The bundled signed add-on's `file:` address when this build ships it; an error when its bytes are not the pinned ones.
fn bundled(config: &ChromeControlConfig) -> Result<Option<String>, &'static str> {
    use sha2::{Digest, Sha256};
    let Some(path) = &config.firefox_addon else {
        return Ok(None);
    };
    let pin = config.firefox_addon_sha256.as_deref().ok_or(DAMAGED)?;
    let bytes = std::fs::read(path).map_err(|_| DAMAGED)?;
    if format!("{:x}", Sha256::digest(&bytes)) != pin {
        return Err(DAMAGED);
    }
    file_url(path).map(Some).ok_or(DAMAGED)
}

fn file_url(path: &std::path::Path) -> Option<String> {
    url::Url::from_file_path(path).ok().map(String::from)
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

/// What Connect opens. `installed`: the add-on is already in Firefox at a current version, so its management page is
/// the next step; otherwise the bundled signed file, which makes Firefox show its own install prompt.
pub(super) fn target(config: &ChromeControlConfig, installed: bool) -> Result<String, String> {
    let fallback = || {
        config
            .firefox_store_url
            .clone()
            .unwrap_or_else(|| ADDONS_PAGE.to_owned())
    };
    let target = if installed {
        fallback()
    } else {
        bundled(config)?.unwrap_or_else(fallback)
    };
    if allowed(config, &target) {
        Ok(target)
    } else {
        Err(INVALID.to_owned())
    }
}

#[cfg(test)]
mod tests {
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
