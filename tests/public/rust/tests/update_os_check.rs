//! The update flow's system-version check: a release manifest may name the oldest system a platform's package
//! runs on (`minimumSystemVersion` for macOS and a Windows build, `minimumGlibc` for the C library the Linux
//! AppImage needs); on an older system the flow shows why and never downloads or installs.
//! PREDICT: a too-old system stops at `osTooOld` with nothing downloaded (macOS, Windows and Linux alike); a
//! missing minimum (old manifests) restricts nothing; an unreadable minimum fails the check; after a system update
//! the next check downloads as before; Windows builds and glibc versions compare by number; Linux reads
//! `minimumGlibc`, never `minimumSystemVersion`.

mod common;

use std::sync::atomic::Ordering;

use common::update_rig::{PACKAGE, Rig, is_ready};
use serde_json::json;
use vw_host::update_flow::{InstallMode, Phase, UpdateError, UpdateRequester};
use vw_host::update_os::{
    MINIMUM_GLIBC, MINIMUM_SYSTEM_VERSION, OsTooOld, below_minimum, check_minimum, minimum_field,
};

// ---- the version comparison ----------------------------------------------------------------------------------

#[test]
fn versions_compare_by_number_not_by_text() {
    assert_eq!(below_minimum("11.7.10", "14.0"), Some(true));
    assert_eq!(below_minimum("13.99", "14"), Some(true));
    assert_eq!(below_minimum("14.0", "14.0"), Some(false));
    assert_eq!(below_minimum("14", "14.0.0"), Some(false));
    assert_eq!(below_minimum("15.2", "14.0"), Some(false));
    assert_eq!(below_minimum("10.15.7", "11.0"), Some(true));
    assert_eq!(below_minimum("27.0.1", "14.0"), Some(false));
    assert_eq!(below_minimum("14.0", "99.0"), Some(true));
}

#[test]
fn windows_builds_compare_by_number() {
    // `major.minor.build` from the manifest against this computer's `major.minor.build.UBR`.
    assert_eq!(below_minimum("10.0.26300.9711", "10.0.19045"), Some(false));
    assert_eq!(below_minimum("10.0.19045.6456", "10.0.19045"), Some(false));
    assert_eq!(below_minimum("10.0.19045", "10.0.19045"), Some(false));
    assert_eq!(below_minimum("10.0.19044.5011", "10.0.19045"), Some(true));
    assert_eq!(below_minimum("10.0.17763.6893", "10.0.19045"), Some(true));
    assert_eq!(below_minimum("10.0.9200", "10.0.17763"), Some(true));
    assert_eq!(below_minimum("10.0.26300.9711", "99.0"), Some(true));
}

#[test]
fn glibc_versions_compare_by_number() {
    assert_eq!(below_minimum("2.43", "2.39"), Some(false));
    assert_eq!(below_minimum("2.39", "2.39"), Some(false));
    assert_eq!(below_minimum("2.35", "2.39"), Some(true));
    // 2.4 is older than 2.39 (numbers, not text).
    assert_eq!(below_minimum("2.4", "2.39"), Some(true));
    assert_eq!(below_minimum("2.39.9000", "2.39"), Some(false));
    assert_eq!(below_minimum("2.43", "99.0"), Some(true));
}

#[test]
fn each_system_reads_its_own_manifest_field() {
    assert_eq!(MINIMUM_SYSTEM_VERSION, "minimumSystemVersion");
    assert_eq!(MINIMUM_GLIBC, "minimumGlibc");
    assert_eq!(minimum_field("macOS"), MINIMUM_SYSTEM_VERSION);
    assert_eq!(minimum_field("Windows"), MINIMUM_SYSTEM_VERSION);
    // Linux: the C library the AppImage's programs need, not a distribution's version.
    assert_eq!(minimum_field("Linux"), MINIMUM_GLIBC);
    assert_eq!(minimum_field("this system"), MINIMUM_SYSTEM_VERSION);
}

#[test]
fn unreadable_versions_cannot_be_compared() {
    for (current, minimum) in [
        ("garbage", "14.0"),
        ("14.0", "garbage"),
        ("", "14.0"),
        ("14.0", ""),
        ("14..0", "14.0"),
        ("14.0.", "14.0"),
        ("-14", "14.0"),
        ("14.0", "v14"),
        ("14.0 beta", "14.0"),
        ("1.2.3.4.5", "1.0"),
        ("99999999999999999999", "14.0"),
    ] {
        assert_eq!(
            below_minimum(current, minimum),
            None,
            "{current:?} vs {minimum:?}"
        );
    }
}

#[test]
fn only_a_readable_lower_system_is_too_old() {
    assert_eq!(
        check_minimum("macOS", Some("11.7.10"), Some(&json!("14.0"))),
        Ok(Some(OsTooOld {
            os: "macOS".into(),
            minimum: "14.0".into(),
            current: "11.7.10".into(),
        }))
    );
    // Missing fields in older announcements and unknown computer versions impose no restriction.
    assert_eq!(check_minimum("macOS", Some("11.7.10"), None), Ok(None));
    assert!(check_minimum("macOS", Some("11.7.10"), Some(&json!("soon"))).is_err());
    assert_eq!(check_minimum("macOS", None, Some(&json!("14.0"))), Ok(None));
    for current in ["14.0", "15.2"] {
        assert_eq!(
            check_minimum("macOS", Some(current), Some(&json!("14.0"))),
            Ok(None)
        );
    }
}

// ---- the gate before the download ----------------------------------------------------------------------------

fn too_old() -> OsTooOld {
    OsTooOld {
        os: "macOS".into(),
        minimum: "99.0".into(),
        current: "27.0.1".into(),
    }
}

#[tokio::test]
async fn a_too_old_system_never_downloads() {
    let rig = Rig::new("os-auto");
    rig.announce("1.1.0", PACKAGE);
    rig.require_os(too_old());
    rig.manager.check().await.unwrap();
    assert_eq!(
        rig.phase(),
        Phase::OsTooOld {
            version: "1.1.0".into(),
            os: "macOS".into(),
            minimum: "99.0".into(),
            current: "27.0.1".into(),
        }
    );
    assert_eq!(
        rig.script.downloads.load(Ordering::SeqCst),
        0,
        "nothing is downloaded on a too-old system"
    );
    assert!(!rig.package_file().exists());
    assert!(rig.notices().is_empty(), "no 'downloading' notification");
    // Neither the window nor the Debug API can start the download or the install from here.
    assert_eq!(
        rig.manager.download(UpdateRequester::DebugApi).await,
        Err(UpdateError::WrongState)
    );
    assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 0);
    assert!(rig.manager.install().await.is_err());
    assert_eq!(rig.installs(), 0);
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rig.dir.join("updates.json")).unwrap()).unwrap();
    assert_eq!(status["outcome"], "osTooOld");
    assert_eq!(status["version"], "1.1.0");
}

#[tokio::test]
async fn ask_first_mode_offers_no_download_either() {
    let rig = Rig::new("os-ask");
    rig.manager.set_auto_download(false).unwrap();
    rig.announce("1.1.0", PACKAGE);
    rig.require_os(too_old());
    rig.manager.check().await.unwrap();
    assert!(
        matches!(rig.phase(), Phase::OsTooOld { .. }),
        "{:?}",
        rig.phase()
    );
    assert_eq!(
        rig.manager.download(UpdateRequester::Window).await,
        Err(UpdateError::WrongState)
    );
    assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_link_only_copy_shows_the_same_reason() {
    let rig = Rig::with("os-link", InstallMode::LinkOnly, None, None);
    rig.announce("1.1.0", PACKAGE);
    rig.require_os(too_old());
    rig.manager.check().await.unwrap();
    assert!(
        matches!(rig.phase(), Phase::OsTooOld { .. }),
        "{:?}",
        rig.phase()
    );
    assert!(rig.notices().is_empty());
}

#[tokio::test]
async fn after_a_system_update_the_next_check_downloads() {
    let rig = Rig::new("os-later");
    rig.announce("1.1.0", PACKAGE);
    rig.require_os(too_old());
    rig.manager.check().await.unwrap();
    assert!(matches!(rig.phase(), Phase::OsTooOld { .. }));
    // The same release, checked again on a system that is new enough now.
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    assert!(is_ready(&rig.phase(), "1.1.0"), "{:?}", rig.phase());
    assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn os_too_old_json_shape_is_pinned() {
    let rig = Rig::new("os-shape");
    rig.announce("1.1.0", PACKAGE);
    rig.require_os(too_old());
    rig.manager.check().await.unwrap();
    let json = serde_json::to_value(rig.manager.view()).unwrap();
    assert_eq!(
        json["phase"],
        serde_json::json!({
            "type": "osTooOld",
            "version": "1.1.0",
            "os": "macOS",
            "minimum": "99.0",
            "current": "27.0.1",
        })
    );
}

// ---- the same gate on Windows and Linux ------------------------------------------------------------------

#[tokio::test]
async fn windows_and_linux_stop_before_the_download_too() {
    for (os, minimum, current) in [
        ("Windows", "10.0.19045", "10.0.17763.6893"),
        ("Linux", "2.39", "2.35"),
    ] {
        let rig = Rig::new(&format!("os-{os}"));
        rig.announce("1.1.0", PACKAGE);
        rig.require_os(OsTooOld {
            os: os.into(),
            minimum: minimum.into(),
            current: current.into(),
        });
        rig.manager.check().await.unwrap();
        assert_eq!(
            serde_json::to_value(rig.manager.view()).unwrap()["phase"],
            serde_json::json!({
                "type": "osTooOld",
                "version": "1.1.0",
                "os": os,
                "minimum": minimum,
                "current": current,
            }),
            "{os}"
        );
        assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 0, "{os}");
        assert!(!rig.package_file().exists(), "{os}");
        assert!(rig.notices().is_empty(), "{os}");
        assert_eq!(
            rig.manager.download(UpdateRequester::DebugApi).await,
            Err(UpdateError::WrongState),
            "{os}"
        );
        assert!(rig.manager.install().await.is_err(), "{os}");
        assert_eq!(rig.installs(), 0, "{os}");
    }
}
