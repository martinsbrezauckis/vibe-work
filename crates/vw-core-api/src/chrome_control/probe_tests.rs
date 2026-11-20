//! The read-only Chrome preferences probe on disposable folders: shapes that must answer Unknown, one probe thread
//! at a time, and the positive cache's 5 s bound.
use super::*;
use serde_json::json;

pub(super) const ID: &str = "gnlamocfdabihichplpepjpehlbafhng";

pub(super) struct Scratch(pub(super) PathBuf);

impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        let root = std::env::var_os("VW_TEST_ROOT")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join(format!("chrome-probe-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Receipts reject linked ancestors, including macOS's /var -> /private/var.
        Self(vw_platform::canonical_plain(&root).unwrap())
    }

    /// A profile with the extension folder and `files` (name, bytes).
    pub(super) fn profile(&self, name: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
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

pub(super) fn prefs(reasons: &serde_json::Value) -> Vec<u8> {
    json!({"extensions":{"settings":{ID:{"disable_reasons":reasons}}}})
        .to_string()
        .into_bytes()
}

pub(super) fn probe(profiles: Vec<PathBuf>) -> Probe {
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
