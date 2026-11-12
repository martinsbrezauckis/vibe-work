//! The read-only Chrome preferences probe on disposable folders: shapes that must answer Unknown, one probe thread
//! at a time, and the positive cache's 5 s bound.
use super::*;
use serde_json::json;

const ID: &str = "gnlamocfdabihichplpepjpehlbafhng";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::var_os("VW_TEST_ROOT")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join(format!("chrome-probe-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Receipts reject linked ancestors, including macOS's /var -> /private/var.
        Self(vw_platform::canonical_plain(&root).unwrap())
    }

    /// A profile with the extension folder and `files` (name, bytes).
    fn profile(&self, name: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
        let profile = self.0.join(name);
        std::fs::create_dir_all(profile.join("Extensions").join(ID).join("0.1.3_0")).unwrap();
        for (file, bytes) in files {
            std::fs::write(profile.join(file), bytes).unwrap();
        }
        profile
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn prefs(reasons: &serde_json::Value) -> Vec<u8> {
    json!({"extensions":{"settings":{ID:{"disable_reasons":reasons}}}})
        .to_string()
        .into_bytes()
}

fn probe(profiles: Vec<PathBuf>) -> Probe {
    let prober = Prober::default();
    let _ = prober.probe(profiles, ID, Instant::now());
    // File-shape fixtures need the completed read. Caller deadlines are tested independently below.
    wait_for_worker(&prober);
    prober.state.lock().unwrap().last.unwrap().0
}

#[test]
fn unpacked_non_default_profiles_are_discovered_without_following_the_extension_path() {
    let s = Scratch::new("unpacked-profiles");
    let default = s.profile("Default", &[("Preferences", prefs(&json!(1)))]);
    for location in [4, 8] {
        let other = s.0.join("Profile 1");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("Preferences"),
            json!({"extensions":{"settings":{ID:{
                "location":location,"path":"/unavailable/external/extension","disable_reasons":[]
            }}}})
            .to_string(),
        )
        .unwrap();
        assert_eq!(profiles(&s.0, ID), vec![default.clone(), other.clone()]);
        assert_eq!(probe(vec![other]), Probe::NotOff);
    }
}

#[test]
fn a_cached_disabled_copy_does_not_describe_a_different_profile_set() {
    let s = Scratch::new("cache-profile-scope");
    let pending = s.profile("Pending", &[("Preferences", prefs(&json!(8192)))]);
    let paired = s.profile("Paired", &[("Preferences", prefs(&json!(0)))]);
    let prober = Prober::default();
    let now = Instant::now();
    assert_eq!(prober.probe(vec![pending], ID, now), Probe::AwaitingEnable);
    assert_eq!(prober.probe(vec![paired], ID, now), Probe::NotOff);
}

// A busy full-suite run can schedule the reader later than its injected delay.
// Wait for completion instead of assuming another fixed sleep is long enough.
fn wait_for_worker(prober: &Prober) {
    let started = Instant::now();
    while prober.state.lock().unwrap().running {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "probe did not complete"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn probe_fixtures() {
    let s = Scratch::new("fixtures");
    // Off as a bitmask and as a list; waiting for consent when Chrome's external-extension reason (8192) is among
    // them; not off as zero, an empty list or no field.
    for (reasons, expected) in [
        (json!(4), Probe::Off),
        (json!([1]), Probe::Off),
        (json!(8192), Probe::AwaitingEnable),
        (json!(8193), Probe::AwaitingEnable),
        (json!([8192]), Probe::AwaitingEnable),
        (json!([1, 8192]), Probe::AwaitingEnable),
        (json!(0), Probe::NotOff),
        (json!([]), Probe::NotOff),
    ] {
        let p = s.profile("Profile 1", &[("Preferences", prefs(&reasons))]);
        assert_eq!(probe(vec![p]), expected, "{reasons}");
    }
    // Secure Preferences decides first.
    let p = s.profile(
        "Secure",
        &[
            ("Secure Preferences", prefs(&json!(1))),
            ("Preferences", prefs(&json!(0))),
        ],
    );
    assert_eq!(probe(vec![p]), Probe::Off);
    // Missing files, wrong types, malformed JSON, nesting past 128, an oversize file: Unknown.
    assert_eq!(probe(vec![s.profile("Missing", &[])]), Probe::Unknown);
    let p = s.profile("WrongType", &[("Preferences", prefs(&json!("off")))]);
    assert_eq!(probe(vec![p]), Probe::Unknown);
    let p = s.profile(
        "Malformed",
        &[("Preferences", b"{\"extensions\":".to_vec())],
    );
    assert_eq!(probe(vec![p]), Probe::Unknown);
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200)).into_bytes();
    let p = s.profile("Deep", &[("Preferences", deep)]);
    assert_eq!(probe(vec![p]), Probe::Unknown);
    let p = s.profile("Oversize", &[]);
    let file = std::fs::File::create(p.join("Preferences")).unwrap();
    file.set_len(MAX_BYTES + 1).unwrap();
    assert_eq!(probe(vec![p]), Probe::Unknown);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // A symlinked file is skipped before it is opened.
        let target = s.0.join("elsewhere.json");
        std::fs::write(&target, prefs(&json!(1))).unwrap();
        let p = s.profile("LinkFile", &[]);
        symlink(&target, p.join("Preferences")).unwrap();
        assert_eq!(probe(vec![p]), Probe::Unknown);
        // A symlinked profile folder is not a profile.
        let real = s.profile("RealProfile", &[("Preferences", prefs(&json!(1)))]);
        symlink(&real, s.0.join("LinkProfile")).unwrap();
        assert_eq!(probe(vec![s.0.join("LinkProfile")]), Probe::Unknown);
        assert_eq!(
            profiles(&s.0, ID)
                .iter()
                .filter(|p| p.ends_with("LinkProfile"))
                .count(),
            0
        );
        // A FIFO with no writer: skipped by lstat, and the open would not block either.
        let p = s.profile("Fifo", &[]);
        // `mkfifo` exists on Linux and macOS; rustix leaves its mknod/mkfifo calls out on Apple systems.
        let made = std::process::Command::new("mkfifo")
            .arg(p.join("Preferences"))
            .status()
            .unwrap();
        assert!(made.success(), "mkfifo");
        assert_eq!(probe(vec![p.clone()]), Probe::Unknown);
        assert!(
            open_no_follow(&profile_dir(&p).unwrap(), Path::new("Preferences")).is_some(),
            "non-blocking open"
        );
        // A device: lstat skips it, and fstat refuses it even when opened directly.
        let dev = s.profile("Device", &[]);
        symlink("/dev/zero", dev.join("Preferences")).unwrap();
        assert_eq!(probe(vec![dev]), Probe::Unknown);
        assert!(open_regular(Path::new("/dev/zero")).is_none());
    }
}

#[test]
fn probe_rapid_polls_one_worker() {
    let s = Scratch::new("rapid");
    let p = s.profile("Profile 1", &[("Preferences", prefs(&json!(0)))]);
    let prober = Prober::default();
    prober.slow_for_test(Duration::from_millis(1500));
    let started = Instant::now();
    // The first caller waits at most 500 ms; the others get the last result at once.
    assert_eq!(
        prober.probe(vec![p.clone()], ID, Instant::now()),
        Probe::Unknown
    );
    assert!(started.elapsed() < WAIT + Duration::from_millis(200));
    for _ in 0..20 {
        assert_eq!(
            prober.probe(vec![p.clone()], ID, Instant::now()),
            Probe::Unknown
        );
    }
    assert_eq!(prober.started_for_test(), 1, "one probe thread at a time");
    wait_for_worker(&prober);
    prober.slow_for_test(Duration::ZERO);
    assert_eq!(prober.probe(vec![p], ID, Instant::now()), Probe::NotOff);
    assert_eq!(prober.started_for_test(), 2);
}

#[test]
fn probe_same_metadata_change_expires() {
    let s = Scratch::new("cache");
    let p = s.profile("Profile 1", &[("Preferences", prefs(&json!(1)))]);
    let file = p.join("Preferences");
    let prober = Prober::default();
    let t0 = Instant::now();
    assert_eq!(prober.probe(vec![p.clone()], ID, t0), Probe::Off);
    // Same size, same modification time, same file: rewritten as not off.
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    {
        use std::io::{Seek, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        f.seek(std::io::SeekFrom::Start(0)).unwrap();
        let text = prefs(&json!(0));
        let mut same = String::from_utf8(text).unwrap();
        while same.len() < prefs(&json!(1)).len() {
            same.push(' ');
        }
        f.write_all(same.as_bytes()).unwrap();
        f.set_modified(modified).unwrap();
    }
    assert_eq!(
        prober.probe(vec![p.clone()], ID, t0 + Duration::from_secs(4)),
        Probe::Off
    );
    assert_eq!(
        prober.probe(vec![p.clone()], ID, t0 + POSITIVE_FOR),
        Probe::NotOff
    );
    // A negative answer is never cached; a changed file is read again at once.
    std::fs::write(&file, prefs(&json!([2, 3]))).unwrap();
    assert_eq!(prober.probe(vec![p], ID, t0 + POSITIVE_FOR), Probe::Off);
}

#[test]
fn installed_version_reads_the_newest_version_folder() {
    let s = Scratch::new("version");
    let p = s.profile("Profile 1", &[]);
    for name in ["0.1.10_0", "0.1.9_1", "Temp", "x.y_0"] {
        std::fs::create_dir_all(p.join("Extensions").join(ID).join(name)).unwrap();
    }
    assert_eq!(installed_version(&p, ID).as_deref(), Some("0.1.10"));
    assert_eq!(installed_version(&s.0.join("Nope"), ID), None);
}

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

#[test]
fn expired_off_is_unknown_during_a_pending_read() {
    let scratch = Scratch::new("pending-expiry");
    let profile = scratch.profile("Profile A", &[("Preferences", prefs(&json!(1)))]);
    let prober = Prober::default();
    let now = Instant::now();
    assert_eq!(prober.probe(vec![profile.clone()], ID, now), Probe::Off);
    let path = profile.join("Preferences");
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, prefs(&json!(0))).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    prober.slow_for_test(Duration::from_millis(1500));
    let expired = now + POSITIVE_FOR;
    assert_eq!(
        prober.probe(vec![profile.clone()], ID, expired),
        Probe::Unknown,
        "timeout must not reuse expired Off"
    );
    for _ in 0..10 {
        assert_eq!(
            prober.probe(vec![profile.clone()], ID, expired),
            Probe::Unknown,
            "running worker must not reuse expired Off"
        );
    }
    wait_for_worker(&prober);
    prober.slow_for_test(Duration::ZERO);
    assert_eq!(prober.probe(vec![profile], ID, expired), Probe::NotOff);
}

#[test]
fn off_expiry_is_checked_while_running_and_after_the_callers_wait() {
    for remaining in [Duration::from_secs(1), Duration::from_millis(200)] {
        let scratch = Scratch::new(&format!("cross-expiry-{}", remaining.as_millis()));
        let profile = scratch.profile("Profile A", &[("Preferences", prefs(&json!(1)))]);
        let prober = Prober::default();
        let now = Instant::now();
        assert_eq!(prober.probe(vec![profile.clone()], ID, now), Probe::Off);
        // Different size invalidates the cache before its TTL, starting a refresh while Off is still fresh.
        std::fs::write(profile.join("Preferences"), prefs(&json!([]))).unwrap();
        prober.slow_for_test(Duration::from_millis(1500));
        assert_eq!(
            prober.probe(
                vec![profile.clone()],
                ID,
                (now + POSITIVE_FOR).checked_sub(remaining).unwrap()
            ),
            if remaining > WAIT {
                Probe::Off
            } else {
                Probe::Unknown
            },
        );
        for _ in 0..10 {
            assert_eq!(
                prober.probe(vec![profile.clone()], ID, now + POSITIVE_FOR),
                Probe::Unknown
            );
        }
        wait_for_worker(&prober);
        prober.slow_for_test(Duration::ZERO);
        assert_eq!(
            prober.probe(vec![profile], ID, now + POSITIVE_FOR),
            Probe::NotOff
        );
    }
}
