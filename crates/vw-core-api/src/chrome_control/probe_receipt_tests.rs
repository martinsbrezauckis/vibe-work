//! Setup-receipt profile folders: linked or junctioned browser trees, version folders and preference files are
//! refused, while a direct registered profile and aliases above it stay usable.
use super::tests::{ID, Scratch, prefs, probe};
use super::*;
use serde_json::json;

fn receipt_profiles(s: &Scratch, user_data: &Path) -> Vec<PathBuf> {
    let state = s.0.join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("setup.json"),
        json!({"files":[], "browsers":["Google Chrome"], "userData":[user_data]}).to_string(),
    )
    .unwrap();
    let roots = super::super::setup::user_data(
        &super::super::setup::ChromeControlConfig::default(),
        Some(&state),
        "Google Chrome",
    );
    assert_eq!(roots, [user_data]);
    assert_eq!(
        super::super::setup::user_data(
            &super::super::setup::ChromeControlConfig::default(),
            Some(&state),
            "Microsoft Edge"
        ),
        Vec::<PathBuf>::new()
    );
    roots.iter().flat_map(|root| profiles(root, ID)).collect()
}

#[cfg(any(unix, windows))]
fn link_directory(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        // cmd treats forward slashes in fixture paths as switches.
        let native = |path: &Path| {
            std::ffi::OsString::from_wide(
                &path
                    .as_os_str()
                    .encode_wide()
                    .map(|c| {
                        if c == u16::from(b'/') {
                            u16::from(b'\\')
                        } else {
                            c
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let output = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(native(link))
            .arg(native(target))
            .output()
            .unwrap();
        assert!(output.status.success(), "junction: {output:?}");
    }
}

#[cfg(any(unix, windows))]
fn receipt_refuses_linked_browser_tree(component: &str) {
    let s = Scratch::new(&format!("linked-{component}"));
    let outside = s.profile("outside/Profile 7", &[("Preferences", prefs(&json!([1])))]);
    let data = s.0.join("registered");
    let profile = data.join("Profile 7");
    if component == "user-data" {
        link_directory(outside.parent().unwrap(), &data);
    } else {
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("Preferences"), prefs(&json!([1]))).unwrap();
        link_directory(&outside.join("Extensions"), &profile.join("Extensions"));
    }
    let found = receipt_profiles(&s, &data);
    let version = found.iter().find_map(|p| installed_version(p, ID));
    let state = probe(found.clone());
    assert!(
        found.is_empty() && version.is_none() && state == Probe::Unknown,
        "{component}: profiles={found:?}, version={version:?}, state={state:?}"
    );
    // Version reads must also recheck a path retained by a previous discovery.
    assert_eq!(installed_version(&profile, ID), None);
    assert_eq!(probe(vec![profile]), Probe::Unknown);
}

#[cfg(unix)]
#[test]
fn receipt_refuses_user_data_symlink() {
    receipt_refuses_linked_browser_tree("user-data");
}

#[cfg(unix)]
#[test]
fn receipt_refuses_extensions_symlink() {
    receipt_refuses_linked_browser_tree("extensions");
}

#[cfg(windows)]
#[test]
fn receipt_refuses_user_data_junction() {
    receipt_refuses_linked_browser_tree("user-data");
}

#[cfg(windows)]
#[test]
fn receipt_refuses_extensions_junction() {
    receipt_refuses_linked_browser_tree("extensions");
}

#[test]
fn receipt_direct_profile_keeps_version_state_and_browser_identity() {
    let s = Scratch::new("direct-receipt");
    let profile = s.profile(
        "registered/Profile 7",
        &[("Preferences", prefs(&json!([1])))],
    );
    let found = receipt_profiles(&s, profile.parent().unwrap());
    assert_eq!(found, std::slice::from_ref(&profile));
    assert_eq!(installed_version(&profile, ID).as_deref(), Some("0.1.3"));
    assert_eq!(probe(found), Probe::Off);
}

#[cfg(any(unix, windows))]
#[test]
fn discovered_paths_and_cached_off_recheck_each_directory() {
    for component in ["user-data", "profile", "extensions", "id"] {
        let s = Scratch::new(&format!("recheck-{component}"));
        let profile = s.profile(
            "registered/Profile 7",
            &[("Preferences", prefs(&json!([1])))],
        );
        let data = profile.parent().unwrap();
        let found = receipt_profiles(&s, data);
        let prober = Prober::default();
        let now = Instant::now();
        assert_eq!(prober.probe(found.clone(), ID, now), Probe::Off);
        let path = match component {
            "user-data" => data.to_owned(),
            "profile" => profile.clone(),
            "extensions" => profile.join("Extensions"),
            _ => profile.join("Extensions").join(ID),
        };
        let moved = s.0.join("outside");
        std::fs::rename(&path, &moved).unwrap();
        link_directory(&moved, &path);
        assert_eq!(installed_version(&profile, ID), None, "{component}");
        assert_eq!(prober.probe(found, ID, now), Probe::Unknown, "{component}");
        assert!(receipt_profiles(&s, data).is_empty(), "{component}");
    }
}

#[cfg(any(unix, windows))]
#[test]
fn version_directory_links_are_not_versions() {
    let s = Scratch::new("linked-version");
    let profile = s.profile("registered/Profile 7", &[]);
    let version = profile.join("Extensions").join(ID).join("0.1.3_0");
    let outside = s.0.join("outside");
    std::fs::rename(&version, &outside).unwrap();
    link_directory(&outside, &version);
    assert_eq!(installed_version(&profile, ID), None);
}

#[cfg(unix)]
#[test]
fn secure_preferences_and_manifest_links_are_not_read() {
    let s = Scratch::new("linked-files");
    let profile = s.profile("registered/Profile 7", &[]);
    let outside = s.0.join("outside.json");
    std::fs::write(&outside, prefs(&json!([1]))).unwrap();
    for name in ["Preferences", "Secure Preferences", "manifest.json"] {
        let path = profile.join(name);
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(read_regular(&path, MAX_BYTES).is_none(), "{name}");
    }
    assert_eq!(probe(vec![profile]), Probe::Unknown);
}

#[cfg(unix)]
#[test]
fn aliases_above_the_registered_folder_remain_usable() {
    let s = Scratch::new("parent-alias");
    s.profile(
        "real/registered/Profile 7",
        &[("Preferences", prefs(&json!([1])))],
    );
    link_directory(&s.0.join("real"), &s.0.join("alias"));
    let found = receipt_profiles(&s, &s.0.join("alias/registered"));
    assert_eq!(found.len(), 1);
    assert_eq!(installed_version(&found[0], ID).as_deref(), Some("0.1.3"));
    assert_eq!(probe(found), Probe::Off);
}
