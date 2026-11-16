//! Connect after the app's data was removed: a browser's host manifest that still names this installation's own
//! helper path is rewritten, one naming another path is refused. Linux and macOS host-folder shapes on real scratch
//! files; the Windows registry value has its own tests in vw-platform's extension registry tests.
//!
//! PREDICT (before the fix): every case that expects Connect to succeed fails with "Browser helper is set up by
//! another Vibe Work", because setup without a receipt refused any existing manifest, even one naming the very path
//! it is about to write. The refusal cases already pass and must keep passing.
use super::*;

fn bundle(s: &Scratch, browser: &str) {
    use std::os::unix::fs::PermissionsExt;
    let exe = s
        .root
        .join("home/Applications")
        .join(format!("{browser}.app/Contents/MacOS/{browser}"));
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"browser").unwrap();
    std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn config_for(s: &Scratch, macos: bool) -> ChromeControlConfig {
    let mut config = s.config();
    config.macos = Some(macos);
    if macos {
        bundle(s, "Google Chrome");
        bundle(s, "Microsoft Edge");
    }
    config
}

fn host_file(s: &Scratch, config: &ChromeControlConfig, browser: &str) -> PathBuf {
    targets(&s.root.join("home"), config)
        .into_iter()
        .find(|t| t.name == browser)
        .unwrap()
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"))
}

/// A manifest an earlier setup of this installation left behind: same host and helper path, other details drifted.
fn leftover(path: &Path, helper: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "an older helper description",
        "path": helper,
        "type": "stdio",
        "allowed_origins": ["chrome-extension://olderextensionidolderextensionid/"],
    });
    std::fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

fn browsers() -> Vec<&'static str> {
    let mut list = vec!["Google Chrome"];
    if cfg!(feature = "your-edge") {
        list.push("Microsoft Edge");
    }
    list
}

#[test]
fn connect_rewrites_a_leftover_manifest_naming_this_installations_helper() {
    for macos in [false, true] {
        let s = Scratch::new(if macos {
            "leftover-mac"
        } else {
            "leftover-linux"
        });
        let config = config_for(&s, macos);
        let helper = s.state().join("bin").join(helper_name());
        for browser in browsers() {
            leftover(&host_file(&s, &config, browser), &helper);
        }
        assert!(read_receipt(&config, &s.state()).is_none());
        for browser in browsers() {
            install_for(&config, &s.state(), &s.vibe_home(), browser)
                .unwrap_or_else(|e| panic!("macos {macos}, {browser}: {e}"));
            let file = host_file(&s, &config, browser);
            assert_eq!(
                std::fs::read(&file).unwrap(),
                manifest_for(&helper, &config, browser),
                "macos {macos}, {browser}"
            );
            assert!(
                read_receipt(&config, &s.state())
                    .unwrap()
                    .files
                    .contains(&file),
                "the rewritten manifest is this setup's own now"
            );
        }
        remove(&config, &s.state()).unwrap();
        for browser in browsers() {
            assert!(!host_file(&s, &config, browser).exists(), "{browser}");
        }
    }
}

#[test]
fn connect_still_refuses_a_manifest_naming_another_helper_path() {
    for macos in [false, true] {
        let s = Scratch::new(if macos {
            "foreign-mac"
        } else {
            "foreign-linux"
        });
        let config = config_for(&s, macos);
        let other = s.root.join("home/another/bin").join(helper_name());
        let file = host_file(&s, &config, "Google Chrome");
        leftover(&file, &other);
        let before = std::fs::read(&file).unwrap();
        let error = install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").unwrap_err();
        assert_eq!(error, "Browser helper is set up by another Vibe Work");
        assert_eq!(std::fs::read(&file).unwrap(), before, "macos {macos}");
        assert!(!s.state().join("bin").join(helper_name()).exists());
        assert!(read_receipt(&config, &s.state()).is_none());
    }
}

#[test]
fn connect_refuses_an_unreadable_manifest_without_a_receipt() {
    let s = Scratch::new("leftover-garbled");
    let config = config_for(&s, false);
    let file = host_file(&s, &config, "Google Chrome");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"not json").unwrap();
    assert!(install_for(&config, &s.state(), &s.vibe_home(), "Google Chrome").is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"not json");
}
