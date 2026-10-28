//! Update decisions through a scripted backend using throwaway signing keys.
//! PREDICT: consent installs once; invalid, stale, or tampered packages never reach ready.

mod common;

use std::sync::atomic::Ordering;

use common::update_rig::{Key, PACKAGE, Rig, is_ready};
use vw_host::update_flow::{InstallMode, Phase, Stage, UpdateError, UpdateNotice, UpdateRequester};

// ---- auto mode: download, then ask -------------------------------------------------------------------------

#[tokio::test]
async fn auto_mode_downloads_verifies_and_stops_at_ready() {
    let rig = Rig::new("auto");
    *rig.script.observe.lock().unwrap() = Some(rig.manager.clone());
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    assert!(is_ready(&rig.phase(), "1.1.0"), "{:?}", rig.phase());
    assert_eq!(rig.installs(), 0, "nothing installs by itself");
    assert_eq!(
        rig.hooks.restarts.load(Ordering::SeqCst),
        0,
        "nothing restarts by itself"
    );
    assert_eq!(
        rig.notices(),
        vec![
            UpdateNotice::Available {
                version: "1.1.0".into(),
                downloading: true,
                link_only: false
            },
            UpdateNotice::Ready {
                version: "1.1.0".into()
            }
        ]
    );
    assert_eq!(std::fs::read(rig.package_file()).unwrap(), PACKAGE);
    assert!(matches!(
        rig.script.seen.lock().unwrap().first(),
        Some(Phase::Downloading { received, total: Some(total), .. }) if received == total
    ));
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rig.dir.join("updates.json")).unwrap()).unwrap();
    assert_eq!(status["outcome"], "ready");
    assert_eq!(status["version"], "1.1.0");
}

#[tokio::test]
async fn view_json_shape_is_pinned() {
    let rig = Rig::new("shape");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    let json = serde_json::to_value(rig.manager.view()).unwrap();
    assert_eq!(json["current"], "1.0.0");
    assert_eq!(json["mode"], "inApp");
    assert_eq!(json["autoDownload"], true);
    assert_eq!(
        json["releasePage"],
        "https://github.com/martinsbrezauckis/vibe-work/releases/tag/v1.1.0"
    );
    assert_eq!(json["phase"]["type"], "ready");
    assert_eq!(json["phase"]["version"], "1.1.0");
    assert_eq!(json["phase"]["notes"], "Faster start.");
}

// ---- consent -----------------------------------------------------------------------------------------------

#[tokio::test]
async fn install_without_consent_is_refused() {
    let rig = Rig::new("noconsent");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    assert_eq!(
        rig.manager.install().await,
        Err(UpdateError::ConsentRequired)
    );
    assert_eq!(rig.installs(), 0);
    assert!(is_ready(&rig.phase(), "1.1.0"), "still offered");
}

#[tokio::test]
async fn window_consent_installs_the_downloaded_bytes_and_restarts() {
    let rig = Rig::new("window");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    rig.manager
        .request_install(UpdateRequester::Window)
        .await
        .unwrap();
    assert_eq!(
        rig.script.installs.lock().unwrap().clone(),
        vec![("1.1.0".to_owned(), PACKAGE.to_vec())]
    );
    assert_eq!(rig.hooks.restarts.load(Ordering::SeqCst), 1);
    assert!(matches!(rig.phase(), Phase::Restarting { .. }));
    assert!(!rig.package_file().exists(), "the used package is deleted");
}

#[tokio::test]
async fn debug_api_request_is_the_consent_and_installs_once() {
    // the Debug API acts as the user, so its request records the consent itself ('s dialog is
    // gone). The consent rules stay: one version, single use.
    let rig = Rig::new("dbgapprove");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    rig.manager
        .request_install(UpdateRequester::DebugApi)
        .await
        .unwrap();
    assert_eq!(
        rig.script.installs.lock().unwrap().clone(),
        vec![("1.1.0".to_owned(), PACKAGE.to_vec())]
    );
    assert_eq!(rig.hooks.restarts.load(Ordering::SeqCst), 1);
    // The consent was used up by that attempt.
    assert_eq!(rig.manager.install().await, Err(UpdateError::WrongState));
    assert_eq!(rig.installs(), 1);
}

#[tokio::test]
async fn a_consent_is_single_use() {
    let rig = Rig::new("single");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    *rig.script.install_error.lock().unwrap() = Some("disk full".to_owned());
    assert!(matches!(
        rig.manager.request_install(UpdateRequester::Window).await,
        Err(UpdateError::Failed(_))
    ));
    assert!(matches!(
        rig.phase(),
        Phase::Failed {
            stage: Stage::Install,
            ..
        }
    ));
    // "Try again": the check brings the kept package back to ready, but the used consent is gone.
    *rig.script.install_error.lock().unwrap() = None;
    rig.manager.check().await.unwrap();
    assert!(is_ready(&rig.phase(), "1.1.0"));
    assert_eq!(
        rig.script.downloads.load(Ordering::SeqCst),
        1,
        "no second download"
    );
    assert_eq!(
        rig.manager.install().await,
        Err(UpdateError::ConsentRequired)
    );
    rig.manager
        .request_install(UpdateRequester::Window)
        .await
        .unwrap();
    assert_eq!(rig.installs(), 1);
}

#[tokio::test]
async fn a_consent_for_one_version_never_installs_another() {
    let rig = Rig::new("oneversion");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    rig.manager.consent(UpdateRequester::Window).unwrap();
    // The offer changes before the install: 1.2.0 arrives and downloads.
    rig.announce("1.2.0", b"the 1.2.0 installer");
    rig.manager.check().await.unwrap();
    assert!(is_ready(&rig.phase(), "1.2.0"));
    assert_eq!(
        rig.manager.install().await,
        Err(UpdateError::ConsentRequired)
    );
    assert_eq!(rig.installs(), 0);
}

#[tokio::test]
async fn nothing_to_install_before_ready() {
    let rig = Rig::with(
        "early",
        InstallMode::InApp,
        Some(r#"{"autoDownload": false}"#),
        None,
    );
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    assert!(matches!(rig.phase(), Phase::Available { .. }));
    assert_eq!(
        rig.manager.consent(UpdateRequester::Window),
        Err(UpdateError::WrongState)
    );
    assert_eq!(rig.manager.install().await, Err(UpdateError::WrongState));
    assert_eq!(rig.installs(), 0);
}

// ---- opt-out: ask before downloading -----------------------------------------------------------------------

#[tokio::test]
async fn opt_out_asks_before_downloading() {
    let rig = Rig::with(
        "optout",
        InstallMode::InApp,
        Some(r#"{"autoDownload": false}"#),
        None,
    );
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    assert!(matches!(rig.phase(), Phase::Available { .. }));
    assert_eq!(
        rig.script.downloads.load(Ordering::SeqCst),
        0,
        "no download without the user"
    );
    assert_eq!(
        rig.notices(),
        vec![UpdateNotice::Available {
            version: "1.1.0".into(),
            downloading: false,
            link_only: false
        }]
    );
    rig.manager.download(UpdateRequester::Window).await.unwrap();
    assert!(is_ready(&rig.phase(), "1.1.0"));
    assert_eq!(rig.installs(), 0);
}

#[tokio::test]
async fn debug_api_download_in_opt_out_runs_directly() {
    let rig = Rig::with(
        "optoutdbg",
        InstallMode::InApp,
        Some(r#"{"autoDownload": false}"#),
        None,
    );
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 0);
    rig.manager
        .download(UpdateRequester::DebugApi)
        .await
        .unwrap();
    assert!(is_ready(&rig.phase(), "1.1.0"));
    assert_eq!(rig.installs(), 0, "downloading never installs");
}

// ---- signature and version rules ---------------------------------------------------------------------------

async fn never_ready(rig: &Rig) {
    rig.manager.check().await.unwrap();
    assert!(
        matches!(
            rig.phase(),
            Phase::Failed {
                stage: Stage::Download,
                ..
            }
        ),
        "{:?}",
        rig.phase()
    );
    assert!(
        !rig.package_file().exists(),
        "an unverified package leaves no file"
    );
    assert_eq!(rig.installs(), 0);
    assert_eq!(rig.manager.install().await, Err(UpdateError::WrongState));
    assert!(!rig.notices().contains(&UpdateNotice::Ready {
        version: "1.1.0".into()
    }));
}

#[tokio::test]
async fn a_package_signed_by_another_key_never_reaches_ready() {
    let rig = Rig::new("otherkey");
    rig.announce_signed("1.1.0", PACKAGE, &Key::new(), "1.1.0");
    never_ready(&rig).await;
}

#[tokio::test]
async fn a_package_changed_after_signing_never_reaches_ready() {
    let rig = Rig::new("tampered");
    rig.announce("1.1.0", PACKAGE);
    let mut changed = PACKAGE.to_vec();
    changed[0] ^= 1;
    *rig.script.package.lock().unwrap() = changed;
    never_ready(&rig).await;
}

#[tokio::test]
async fn a_package_signed_for_another_version_never_reaches_ready() {
    let rig = Rig::new("wrongversion");
    rig.announce_signed("1.1.0", PACKAGE, &rig.key, "0.9.0");
    never_ready(&rig).await;
}

#[tokio::test]
async fn an_offer_that_is_not_newer_is_refused() {
    for version in ["1.0.0", "0.9.0"] {
        let rig = Rig::new("older");
        rig.announce(version, PACKAGE);
        rig.manager.check().await.unwrap();
        assert!(matches!(
            rig.phase(),
            Phase::Failed {
                stage: Stage::Check,
                ..
            }
        ));
        assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 0, "{version}");
        assert!(rig.notices().is_empty());
    }
}

#[tokio::test]
async fn up_to_date_is_idle_and_drops_a_stale_package() {
    let rig = Rig::new("uptodate");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    assert!(rig.package_file().exists());
    *rig.script.check.lock().unwrap() = Some(Ok(None));
    rig.manager.check().await.unwrap();
    assert_eq!(rig.phase(), Phase::Idle);
    assert!(!rig.package_file().exists());
}

#[tokio::test]
async fn an_unreadable_minimum_fails_the_check_without_downloading_or_notifying() {
    let rig = Rig::new("unreadable-minimum");
    let error =
        vw_host::update_os::check_minimum("macOS", Some("15.1"), Some(&serde_json::json!(7)))
            .unwrap_err();
    *rig.script.check.lock().unwrap() = Some(Err(error.clone()));
    rig.manager.check().await.unwrap();
    assert_eq!(
        rig.phase(),
        Phase::Failed {
            stage: Stage::Check,
            error: error.clone(),
            version: None,
        }
    );
    assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 0);
    assert!(!rig.package_file().exists());
    assert!(rig.notices().is_empty());
    assert_eq!(rig.installs(), 0);
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rig.dir.join("updates.json")).unwrap()).unwrap();
    assert_eq!(status["outcome"], "failed");
    assert_eq!(status["error"], error);
}

#[tokio::test]
async fn an_unreadable_minimum_keeps_a_verified_ready_package() {
    let rig = Rig::new("ready-unreadable-minimum");
    rig.announce("1.1.0", PACKAGE);
    rig.manager.check().await.unwrap();
    let rig = rig.restart(InstallMode::InApp);
    let ready = rig.phase();
    assert!(is_ready(&ready, "1.1.0"));
    let error =
        vw_host::update_os::check_minimum("macOS", Some("15.1"), Some(&serde_json::json!(7)))
            .unwrap_err();
    *rig.script.check.lock().unwrap() = Some(Err(error));
    rig.manager.check().await.unwrap();
    assert_eq!(rig.phase(), ready);
    assert_eq!(std::fs::read(rig.package_file()).unwrap(), PACKAGE);
    assert_eq!(rig.script.downloads.load(Ordering::SeqCst), 0);
    assert!(rig.notices().is_empty());
    rig.manager
        .request_install(UpdateRequester::Window)
        .await
        .unwrap();
    assert_eq!(rig.script.installs.lock().unwrap()[0].1, PACKAGE);
}
