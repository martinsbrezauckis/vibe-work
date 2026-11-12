//! Read-only Firefox add-on metadata. Reads only the fixed extension identity; never history, bookmarks or sign-ins.
use super::setup::ChromeControlConfig;
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Status {
    Unknown,
    Missing,
    Disabled,
    Installed,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub status: Status,
    pub version: Option<String>,
}
fn unknown() -> Snapshot {
    Snapshot {
        status: Status::Unknown,
        version: None,
    }
}
fn parse(value: &Value) -> Snapshot {
    let Some(addons) = value["addons"].as_array() else {
        return unknown();
    };
    let Some(addon) = addons
        .iter()
        .find(|a| a["id"] == super::setup::FIREFOX_EXTENSION_ID)
    else {
        return Snapshot {
            status: Status::Missing,
            version: None,
        };
    };
    let status = if addon["userDisabled"] == true
        || addon["appDisabled"] == true
        || addon["softDisabled"] == true
    {
        Status::Disabled
    } else if addon["userDisabled"] == false && addon["appDisabled"] == false {
        Status::Installed
    } else {
        Status::Unknown
    };
    Snapshot {
        status,
        version: addon["version"]
            .as_str()
            .filter(|v| v.len() <= 64)
            .map(str::to_owned),
    }
}
fn inventory(path: &std::path::Path) -> Snapshot {
    use std::io::Read;
    let Ok(path) = vw_platform::canonical_plain(path) else {
        return unknown();
    };
    let Ok(file) = std::fs::File::open(path) else {
        return unknown();
    };
    if !file
        .metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= 4 * 1024 * 1024)
    {
        return unknown();
    }
    let mut bytes = Vec::new();
    if file
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return unknown();
    }
    serde_json::from_slice(&bytes).map_or_else(|_| unknown(), |v| parse(&v))
}
pub(super) fn read(config: &ChromeControlConfig) -> Snapshot {
    let Ok(home) = super::setup::home_of(config) else {
        return unknown();
    };
    let root = if config.is_macos() {
        home.join("Library/Application Support/Firefox/Profiles")
    } else if cfg!(windows) {
        home.join("AppData/Roaming/Mozilla/Firefox/Profiles")
    } else {
        home.join(".mozilla/firefox")
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return unknown();
    };
    let mut rows = Vec::new();
    for entry in entries.take(32) {
        let Ok(entry) = entry else {
            return unknown();
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.path();
        // Firefox's data root also contains caches and crash reports. A profile has an
        // inventory or profile metadata; unrelated directories cannot turn known absence into Unknown.
        if !path.join("extensions.json").is_file()
            && !path.join("prefs.js").is_file()
            && !path.join("times.json").is_file()
        {
            continue;
        }
        rows.push(inventory(&path.join("extensions.json")));
    }
    if let Some(index) = rows.iter().position(|row| row.status == Status::Installed) {
        return rows.swap_remove(index);
    }
    if let Some(index) = rows.iter().position(|row| row.status == Status::Disabled) {
        return rows.swap_remove(index);
    }
    if !rows.is_empty() && rows.iter().all(|row| row.status == Status::Missing) {
        return Snapshot {
            status: Status::Missing,
            version: None,
        };
    }
    unknown()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn missing_is_known_only_from_a_readable_addon_inventory() {
        assert_eq!(parse(&json!({"addons":[]})).status, Status::Missing);
        assert_eq!(parse(&json!({})).status, Status::Unknown);
    }
    #[test]
    fn disabled_addon_keeps_its_version_and_never_claims_pairing() {
        let v = parse(
            &json!({"addons":[{"id":super::super::setup::FIREFOX_EXTENSION_ID,"version":"0.1.7","userDisabled":true,"appDisabled":false}]}),
        );
        assert_eq!(v.status, Status::Disabled);
        assert_eq!(v.version.as_deref(), Some("0.1.7"));
    }
    #[test]
    fn installed_version_is_the_exact_firefox_addon_identity() {
        let v = parse(
            &json!({"addons":[{"id":"another-addon","version":"9.9.9"},{"id":super::super::setup::FIREFOX_EXTENSION_ID,"version":"0.1.6","userDisabled":false,"appDisabled":false}]}),
        );
        assert_eq!(v.status, Status::Installed);
        assert_eq!(v.version.as_deref(), Some("0.1.6"));
    }
    #[tokio::test]
    async fn unrelated_directories_do_not_hide_a_confirmed_missing_addon() {
        let fixture = crate::feature_test_core::Fixture::new().await;
        let home = fixture.root.join("probe-home");
        let root = home.join("Library/Application Support/Firefox/Profiles");
        std::fs::create_dir_all(root.join("profile.default")).unwrap();
        std::fs::create_dir_all(root.join("cache")).unwrap();
        std::fs::write(
            root.join("profile.default/extensions.json"),
            br#"{"addons":[]}"#,
        )
        .unwrap();
        let config = ChromeControlConfig {
            user_home: Some(home),
            macos: Some(true),
            ..ChromeControlConfig::default()
        };
        assert_eq!(read(&config).status, Status::Missing);
        fixture.finish().await;
    }
}
