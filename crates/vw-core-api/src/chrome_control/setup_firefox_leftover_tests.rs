//! Firefox Connect after the app's data was removed: the Firefox host manifest an earlier setup of this installation
//! left behind (Linux `~/.mozilla/native-messaging-hosts`, macOS `~/Library/Application Support/Mozilla/
//! NativeMessagingHosts`) still names this installation's own helper path and is rewritten; one naming another path,
//! or unreadable, is refused and left as it is. The Windows registry value shape is tested in `setup_firefox.rs`.
//!
//! PREDICT (before the fix): the "rewrite" case fails with "Firefox is already set up by another Vibe Work":
//! first-time Firefox setup refused ANY existing manifest file before looking at the path it names. The refusal
//! cases already pass and must keep passing.
use super::*;

fn config_for(s: &Scratch, macos: bool) -> ChromeControlConfig {
    use std::os::unix::fs::PermissionsExt;
    let mut config = s.config();
    config.macos = Some(macos);
    if macos {
        let exe = s
            .root
            .join("home/Applications/Firefox.app/Contents/MacOS/firefox");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"browser").unwrap();
        std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    config
}

fn firefox_file(s: &Scratch, config: &ChromeControlConfig) -> PathBuf {
    targets(&s.root.join("home"), config)
        .into_iter()
        .find(|t| t.name == "Mozilla Firefox")
        .unwrap()
        .hosts
        .unwrap()
        .join(format!("{HOST_NAME}.json"))
}

/// A Firefox manifest with this host name naming `helper`; other details drifted.
fn leftover(path: &Path, helper: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "an older helper description",
        "path": helper,
        "type": "stdio",
        "allowed_extensions": ["older@vibework.app"],
    });
    std::fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

#[test]
fn firefox_connect_rewrites_a_leftover_manifest_naming_this_installations_helper() {
    for macos in [false, true] {
        let s = Scratch::new(if macos {
            "ff-leftover-mac"
        } else {
            "ff-leftover-linux"
        });
        let config = config_for(&s, macos);
        let helper = s.state().join("bin").join(helper_name());
        let file = firefox_file(&s, &config);
        leftover(&file, &helper);
        assert!(read_receipt(&config, &s.state()).is_none());
        install_for(&config, &s.state(), &s.vibe_home(), "Mozilla Firefox")
            .unwrap_or_else(|e| panic!("macos {macos}: {e}"));
        assert_eq!(
            std::fs::read(&file).unwrap(),
            manifest_for(&helper, &config, "Mozilla Firefox"),
            "macos {macos}"
        );
        let receipt = read_receipt(&config, &s.state()).unwrap();
        assert!(
            receipt.files.contains(&file),
            "the rewritten manifest is this setup's own now"
        );
        remove(&config, &s.state()).unwrap();
        assert!(!file.exists(), "macos {macos}");
    }
}

#[test]
fn firefox_connect_still_refuses_a_foreign_or_unreadable_manifest() {
    for macos in [false, true] {
        for garbled in [false, true] {
            let s = Scratch::new(&format!("ff-foreign-{macos}-{garbled}"));
            let config = config_for(&s, macos);
            let file = firefox_file(&s, &config);
            if garbled {
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(&file, b"not json").unwrap();
            } else {
                leftover(&file, &s.root.join("home/another/bin").join(helper_name()));
            }
            let before = std::fs::read(&file).unwrap();
            let error =
                install_for(&config, &s.state(), &s.vibe_home(), "Mozilla Firefox").unwrap_err();
            assert_eq!(
                error, "Firefox is already set up by another Vibe Work",
                "macos {macos}"
            );
            assert_eq!(std::fs::read(&file).unwrap(), before, "macos {macos}");
            assert!(!s.state().join("bin").join(helper_name()).exists());
            assert!(read_receipt(&config, &s.state()).is_none());
        }
    }
}
