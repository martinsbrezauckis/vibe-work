//! Native Windows registry checks of Connect Chrome's external-extension registration, and the disposable
//! `HKCU\Software\VibeWorkTests` root those checks (and the setup and Connect tests) work in.
//!
//! Invariants:
//! - Every test key lives under `HKCU\Software\VibeWorkTests`, never under Chrome's own keys.
//! - Each [`TestRoot`] owns one key of its own (`<name>-<process id>`, the name unique within the test process) and
//!   creates it before Connect's registry transaction. Concurrent tests otherwise race to create the shared parent
//!   inside separate transactions, which Windows can reject as a transactional conflict. The root is deleted when
//!   dropped, also when the test panics. The shared parent is never deleted: other
//!   tests, in this process or in another test process, may be working below it at the same moment, and deleting it
//!   would mark their keys for deletion under them. Test roots therefore need no turn-taking.
//!
//! Depends on: reg.exe (Windows), `extension_install` (Connect and Remove). Called by: the Windows tests in
//! `extension_registry` and `setup_tests`, the Connect tests in `enroll_tests`, and the test below.

/// The shared parent of every test key.
const PARENT: &str = r"HKCU\Software\VibeWorkTests";

/// One test's own registry key under [`PARENT`].
pub(super) struct TestRoot {
    key: String,
}

impl TestRoot {
    /// Names this test's key (`<parent>\<name>-<pid>`); a key left by an earlier crashed run of the same process id
    /// is deleted first. `name` must differ from every other live test root's name in this process.
    pub(super) fn new(name: &str) -> Self {
        let key = format!(r"{PARENT}\{name}-{}", std::process::id());
        let _ = reg(&["delete", &key, "/f"]);
        assert!(
            reg(&["add", &key, "/f"]),
            "cannot create the disposable Chrome registry root"
        );
        Self { key }
    }

    /// The test's own key.
    pub(super) fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = reg(&["delete", &self.key, "/f"]);
    }
}

/// reg.exe's exit status (always success off Windows, where there is no registry to clean).
fn reg(args: &[&str]) -> bool {
    if !cfg!(windows) {
        return true;
    }
    std::process::Command::new("reg")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn exists(key: &str) -> bool {
    reg(&["query", key])
}

/// Connect on Windows writes Chrome's per-user external-extension key with the update address; Remove deletes it
/// again, the whole key once it is empty, so a later Connect gets Chrome's one-click prompt again.
#[cfg(windows)]
#[test]
fn remove_deletes_the_empty_extension_key_that_connect_created() {
    let root = TestRoot::new("connect-remove");
    let folder = std::env::temp_dir().join(format!("vw-connect-remove-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("state")).unwrap();
    std::fs::create_dir_all(folder.join("home")).unwrap();
    let config = super::setup::ChromeControlConfig {
        user_home: Some(folder.join("home")),
        windows_registry_root: format!(r"{}\NativeMessagingHosts", root.key()),
        // Chrome is never started by this test.
        launcher: Some(std::sync::Arc::new(|_: &str| Ok(()))),
        ..super::setup::ChromeControlConfig::default()
    };
    let key = format!(r"{}\Extensions\{}", root.key(), config.extension_id);
    super::extension_install::begin(&config, &folder.join("state")).unwrap();
    assert!(exists(&key), "Connect did not write {key}");
    let removed = super::extension_install::remove(&config, &folder.join("state"));
    let left = exists(&key);
    let _ = std::fs::remove_dir_all(&folder);
    assert_eq!(removed, Ok(true));
    assert!(!left, "Remove left the empty key it created: {key}");
}
