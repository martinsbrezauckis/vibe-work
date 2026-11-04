//! The helper copy Chrome starts follows the packaged helper after an app update: replaced when it differs, left
//! alone when it matches, never written through a link, and set aside when a running copy holds its file.
//! Scratch folders only; the Windows-only checks use a copied system program as the running helper.
use super::*;
use setup::HOST_NAME;

struct Rig {
    root: PathBuf,
    registry: super::super::registry_native_tests::TestRoot,
}

const PACKAGED: &[u8] = b"helper of this version";
const OLD: &[u8] = b"helper of the previous version";

impl Rig {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("vw-cc-refresh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("package/bin")).unwrap();
        std::fs::write(
            root.join("package/bin").join(setup::helper_name()),
            PACKAGED,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("state/bin")).unwrap();
        std::fs::create_dir_all(root.join("hosts")).unwrap();
        Self {
            root: vw_platform::canonical_plain(&root).unwrap(),
            registry: super::super::registry_native_tests::TestRoot::new(name),
        }
    }
    fn config(&self) -> ChromeControlConfig {
        ChromeControlConfig {
            windows_registry_root: self.registry.key().into(),
            package: Some(self.root.join("package")),
            user_home: Some(self.root.join("home")),
            ..ChromeControlConfig::default()
        }
    }
    fn state(&self) -> PathBuf {
        self.root.join("state")
    }
    fn helper(&self) -> PathBuf {
        self.state().join("bin").join(setup::helper_name())
    }
    fn manifest(&self) -> PathBuf {
        if cfg!(windows) {
            self.state().join("host")
        } else {
            self.root.join(if cfg!(target_os = "macos") {
                "home/Library/Application Support/Google/Chrome/NativeMessagingHosts"
            } else {
                "home/.config/google-chrome/NativeMessagingHosts"
            })
        }
        .join(format!("{HOST_NAME}.json"))
    }
    /// Set up as an earlier version left it: its helper copy, its manifest and the receipt listing both.
    fn set_up(&self, installed: &[u8]) {
        std::fs::write(self.helper(), installed).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(self.helper(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        std::fs::create_dir_all(self.manifest().parent().unwrap()).unwrap();
        std::fs::write(
            self.manifest(),
            setup::host_manifest(&self.helper(), &self.config().extension_id),
        )
        .unwrap();
        self.receipt();
    }
    fn receipt(&self) {
        std::fs::write(
            self.state().join("setup.json"),
            serde_json::json!({
                "files": [self.helper(), self.manifest()],
                "browsers": ["Google Chrome"],
                "userData": [self.manifest().parent().unwrap().parent().unwrap()],
                "registryKeys": if cfg!(windows) { vec![format!(r"{}\{HOST_NAME}", self.registry.key())] } else { vec![] },
            })
            .to_string(),
        )
        .unwrap();
    }
    fn bin_names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.state().join("bin"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn an_old_copy_is_replaced_with_the_packaged_helper_keeping_its_permissions() {
    let rig = Rig::new("old");
    rig.set_up(OLD);
    assert_eq!(
        refresh(&rig.config(), &rig.state()),
        Ok(Refreshed::Replaced)
    );
    assert_eq!(std::fs::read(rig.helper()).unwrap(), PACKAGED);
    assert_eq!(rig.bin_names(), vec![setup::helper_name().to_owned()]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(rig.helper())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}

#[test]
fn a_missing_copy_is_put_back() {
    let rig = Rig::new("missing");
    rig.set_up(OLD);
    std::fs::remove_file(rig.helper()).unwrap();
    assert_eq!(
        refresh(&rig.config(), &rig.state()),
        Ok(Refreshed::Replaced)
    );
    assert_eq!(std::fs::read(rig.helper()).unwrap(), PACKAGED);
}

#[test]
fn an_identical_copy_is_not_written() {
    let rig = Rig::new("same");
    rig.set_up(PACKAGED);
    let before = std::fs::metadata(rig.helper()).unwrap().modified().unwrap();
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(rig.helper()).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(refresh(&rig.config(), &rig.state()), Ok(Refreshed::Current));
    assert_eq!(
        std::fs::metadata(rig.helper()).unwrap().modified().unwrap(),
        before
    );
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(rig.helper()).unwrap()),
        inode
    );
    assert_eq!(rig.bin_names(), vec![setup::helper_name().to_owned()]);
}

#[test]
fn nothing_happens_before_set_up() {
    let rig = Rig::new("not-set-up");
    std::fs::write(rig.helper(), OLD).unwrap();
    assert_eq!(
        refresh(&rig.config(), &rig.state()),
        Ok(Refreshed::NotSetUp)
    );
    assert_eq!(std::fs::read(rig.helper()).unwrap(), OLD);
}

#[test]
fn a_changed_package_is_refused_and_the_copy_stays() {
    let rig = Rig::new("pin");
    rig.set_up(OLD);
    let config = ChromeControlConfig {
        helper_sha256: Some("0".repeat(64)),
        ..rig.config()
    };
    assert!(refresh(&config, &rig.state()).is_err());
    assert_eq!(std::fs::read(rig.helper()).unwrap(), OLD);
    assert_eq!(rig.bin_names(), vec![setup::helper_name().to_owned()]);
}

#[test]
fn an_outdated_manifest_is_rewritten_as_set_up_writes_it() {
    let rig = Rig::new("manifest");
    rig.set_up(PACKAGED);
    let mut old: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rig.manifest()).unwrap()).unwrap();
    old["description"] = serde_json::json!("Old helper description");
    std::fs::write(rig.manifest(), old.to_string()).unwrap();
    assert_eq!(refresh(&rig.config(), &rig.state()), Ok(Refreshed::Current));
    assert_eq!(
        std::fs::read(rig.manifest()).unwrap(),
        setup::host_manifest(&rig.helper(), &rig.config().extension_id)
    );
}

/// A locked old copy (as a running helper on Windows) is renamed aside, the new one takes its name, and the next
/// refresh deletes the old one.
#[test]
fn a_locked_copy_is_set_aside_and_deleted_later() {
    fn locked(from: &Path, to: &Path) -> std::io::Result<()> {
        if to.exists() {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        } else {
            std::fs::rename(from, to)
        }
    }
    let rig = Rig::new("locked");
    rig.set_up(OLD);
    assert_eq!(
        refresh_with(&rig.config(), &rig.state(), &mut locked),
        Ok(Refreshed::Replaced)
    );
    assert_eq!(std::fs::read(rig.helper()).unwrap(), PACKAGED);
    let names = rig.bin_names();
    let aside: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with(ASIDE_PREFIX))
        .collect();
    assert_eq!(aside.len(), 1, "{names:?}");
    assert_eq!(
        std::fs::read(rig.state().join("bin").join(aside[0])).unwrap(),
        OLD
    );
    assert_eq!(refresh(&rig.config(), &rig.state()), Ok(Refreshed::Current));
    assert_eq!(rig.bin_names(), vec![setup::helper_name().to_owned()]);
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_the_copy_is_refused_and_its_target_is_untouched() {
    let rig = Rig::new("link-copy");
    rig.set_up(OLD);
    let outside = rig.root.join("outside");
    std::fs::write(&outside, OLD).unwrap();
    std::fs::remove_file(rig.helper()).unwrap();
    std::os::unix::fs::symlink(&outside, rig.helper()).unwrap();
    assert!(refresh(&rig.config(), &rig.state()).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), OLD);
    assert!(rig.helper().is_symlink());
    assert_eq!(rig.bin_names(), vec![setup::helper_name().to_owned()]);
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_the_folder_is_refused_and_nothing_is_written_outside() {
    let rig = Rig::new("link-folder");
    rig.set_up(OLD);
    let outside = rig.root.join("outside");
    std::fs::rename(rig.state().join("bin"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, rig.state().join("bin")).unwrap();
    assert!(refresh(&rig.config(), &rig.state()).is_err());
    let names: Vec<_> = std::fs::read_dir(&outside)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from(setup::helper_name())]);
    assert_eq!(
        std::fs::read(outside.join(setup::helper_name())).unwrap(),
        OLD
    );
}

#[cfg(windows)]
fn mklink_junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(windows)]
#[test]
fn a_junction_in_place_of_the_folder_is_refused_and_nothing_is_written_outside() {
    let rig = Rig::new("junction-folder");
    rig.set_up(OLD);
    let outside = rig.root.join("outside");
    std::fs::rename(rig.state().join("bin"), &outside).unwrap();
    mklink_junction(&rig.state().join("bin"), &outside);
    assert!(refresh(&rig.config(), &rig.state()).is_err());
    let names: Vec<_> = std::fs::read_dir(&outside)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from(setup::helper_name())]);
    assert_eq!(
        std::fs::read(outside.join(setup::helper_name())).unwrap(),
        OLD
    );
    let _ = std::fs::remove_dir(rig.state().join("bin"));
}

/// A real running copy: Windows refuses to overwrite a running program's file, so the old copy goes aside and the
/// new one takes its name while the old one keeps running.
#[cfg(windows)]
#[test]
fn a_running_copy_on_windows_is_replaced_by_setting_it_aside() {
    let rig = Rig::new("running");
    let system =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let ping = std::fs::read(system.join(r"System32\PING.EXE")).unwrap();
    rig.set_up(&ping);
    let mut running = std::process::Command::new(rig.helper())
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let outcome = refresh(&rig.config(), &rig.state());
    let names = rig.bin_names();
    let _ = running.kill();
    let _ = running.wait();
    assert_eq!(outcome, Ok(Refreshed::Replaced), "{names:?}");
    assert_eq!(std::fs::read(rig.helper()).unwrap(), PACKAGED);
    assert!(
        names.iter().any(|n| n.starts_with(ASIDE_PREFIX)),
        "{names:?}"
    );
    // Once the old program has ended, the next refresh deletes its set-aside file.
    assert_eq!(refresh(&rig.config(), &rig.state()), Ok(Refreshed::Current));
    assert_eq!(rig.bin_names(), vec![setup::helper_name().to_owned()]);
}

/// A copy another program holds without sharing delete cannot even be renamed: the refresh fails, the copy stays
/// as it was and no new file is left beside it.
#[cfg(windows)]
#[test]
fn a_copy_that_cannot_be_moved_stays_and_the_refresh_reports_it() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    let rig = Rig::new("held");
    rig.set_up(OLD);
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(rig.helper())
        .unwrap();
    let outcome = refresh(&rig.config(), &rig.state());
    let names = rig.bin_names();
    drop(held);
    assert!(outcome.is_err(), "{outcome:?}");
    assert_eq!(std::fs::read(rig.helper()).unwrap(), OLD);
    assert_eq!(names, vec![setup::helper_name().to_owned()]);
}

#[test]
fn failed_install_and_restore_are_retried_and_report_a_missing_host() {
    use std::cell::Cell;
    use std::rc::Rc;
    for restore_fails_twice in [false, true] {
        let rig = Rig::new(if restore_fails_twice {
            "restore-twice"
        } else {
            "restore-once"
        });
        rig.set_up(OLD);
        let receipt = std::fs::read(rig.state().join("setup.json")).unwrap();
        let calls = Rc::new(Cell::new(0));
        let count = calls.clone();
        let mut failing = move |from: &Path, to: &Path| {
            let call = count.get() + 1;
            count.set(call);
            if [1, 3, 4].contains(&call) || restore_fails_twice && call == 5 {
                Err(std::io::Error::other(format!("rename {call} failed")))
            } else {
                std::fs::rename(from, to)
            }
        };
        let problem = refresh_with(&rig.config(), &rig.state(), &mut failing).unwrap_err();
        assert_eq!(calls.get(), 5, "restore must be retried once: {problem}");
        if restore_fails_twice {
            assert!(!rig.helper().exists());
            assert!(problem.contains("host path is absent"), "{problem}");
            assert!(
                problem.contains("rename 4 failed") && problem.contains("rename 5 failed"),
                "{problem}"
            );
        } else {
            assert_eq!(std::fs::read(rig.helper()).unwrap(), OLD);
        }
        assert_eq!(
            std::fs::read(rig.state().join("setup.json")).unwrap(),
            receipt
        );
        assert_eq!(
            refresh(&rig.config(), &rig.state()),
            Ok(Refreshed::Replaced)
        );
        assert_eq!(std::fs::read(rig.helper()).unwrap(), PACKAGED);
    }
}

#[test]
fn sweeping_refresh_files_preserves_foreign_prefix_matches() {
    let rig = Rig::new("sweep-names");
    rig.set_up(PACKAGED);
    let extension = if cfg!(windows) { ".exe" } else { "" };
    let foreign = [
        "chrome-control.old-not-ours".to_owned(),
        ".chrome-control.new-not-ours".to_owned(),
        format!("{ASIDE_PREFIX}0123456789abcde{extension}"),
        format!("{ASIDE_PREFIX}0123456789abcdef0{extension}"),
        format!("{ASIDE_PREFIX}0123456789abcdeg{extension}"),
        format!("{ASIDE_PREFIX}1123456789ABCDEf{extension}"),
        format!("{ASIDE_PREFIX}0123456789abcdef{extension}.bak"),
        format!("{TEMP_PREFIX}0123456789abcdef.exe"),
    ];
    let generated = [
        format!("{ASIDE_PREFIX}0123456789abcdef{extension}"),
        format!("{TEMP_PREFIX}0123456789abcdef"),
    ];
    // Foreign fixtures must stay distinct even on case-insensitive filesystems.
    for name in &foreign {
        assert!(
            generated
                .iter()
                .all(|owned| !name.eq_ignore_ascii_case(owned)),
            "foreign fixture aliases a generated file: {name}"
        );
    }
    for name in foreign.iter().chain(&generated) {
        std::fs::write(rig.state().join("bin").join(name), b"keep foreign bytes").unwrap();
    }
    assert_eq!(refresh(&rig.config(), &rig.state()), Ok(Refreshed::Current));
    for name in foreign {
        assert_eq!(
            std::fs::read(rig.state().join("bin").join(&name))
                .ok()
                .as_deref(),
            Some(b"keep foreign bytes".as_slice()),
            "foreign file removed: {name}"
        );
    }
    for name in generated {
        assert!(!rig.state().join("bin").join(name).exists());
    }
}
