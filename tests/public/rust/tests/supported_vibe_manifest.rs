//! Supported Vibe manifest checks against a local channel and throwaway signing keys.
//! PREDICT: valid offers appear; tampering, missing signatures, and rollback state failures clear offers.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use common::processes::scratch;
use common::support_rig::{
    Channel, entry, install_marker, manifest, running_rig, sign_as, support, upload_requirements,
};
use common::update_rig::Key;
use vw_vibe_install::{InstallError, SupportError, VibeUpdateView};

fn update(rig: &common::vibe_rig::Rig) -> VibeUpdateView {
    *rig.installer.view().update
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_manifest_offers_the_newer_tested_vibe_and_names_the_untested_one() {
    let channel = Channel::start().await;
    let key = Key::new();
    let rig = running_rig(&scratch("support-s1"), &channel);
    channel.publish(
        &key,
        &manifest(
            5,
            Some("2.27.0"),
            vec![
                entry("2.26.0", "0.1.0", None),
                entry("2.26.2", "0.1.0", None),
            ],
        ),
    );
    upload_requirements(&channel, &["2.26.0", "2.26.2"]);

    rig.installer
        .check_support(&support(&channel, &key))
        .await
        .unwrap();
    let view = update(&rig);
    assert_eq!(view.available.as_deref(), Some("2.26.2"));
    assert_eq!(view.upcoming.as_deref(), Some("2.27.0"));
    assert_eq!(view.check_error, None);
    assert!(view.checked_at.is_some());
    assert_eq!(channel.hits("vibe-support.json"), 1);
    assert_eq!(channel.hits("vibe-support.json.sig"), 1);
    assert_eq!(
        channel.hits("vibe-2.26.2.requirements.txt"),
        0,
        "no install data before consent"
    );
}

async fn refused(
    name: &str,
    publish: impl FnOnce(&Channel, &Key),
) -> (VibeUpdateView, SupportError) {
    let channel = Channel::start().await;
    let key = Key::new();
    let rig = running_rig(&scratch(name), &channel);
    publish(&channel, &key);
    let error = rig
        .installer
        .check_support(&support(&channel, &key))
        .await
        .expect_err("the check must give no offer");
    (update(&rig), error)
}

fn no_offer(view: &VibeUpdateView) {
    assert_eq!(view.available, None, "{view:?}");
    assert_eq!(view.upcoming, None, "{view:?}");
    assert!(view.check_error.is_some(), "{view:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_manifest_gives_no_offer() {
    let (view, error) = refused("support-s2", |channel, key| {
        let bytes = manifest(5, Some("2.27.0"), vec![entry("2.26.2", "0.1.0", None)]);
        channel.publish(key, &bytes);
        let mut tampered = bytes.clone();
        // "2.26.2" -> "2.26.3" in the first version string: still valid JSON, different bytes.
        let at = tampered.windows(6).position(|w| w == b"2.26.2").unwrap();
        tampered[at + 5] = b'3';
        channel.put("vibe-support.json", tampered);
    })
    .await;
    no_offer(&view);
    assert!(matches!(error, SupportError::Signature(_)), "{error:?}");
    assert!(
        view.check_error
            .unwrap()
            .contains("not signed with this build's update key")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_signed_by_another_key_gives_no_offer() {
    let (view, error) = refused("support-s3", |channel, _key| {
        let other = Key::new();
        channel.publish(
            &other,
            &manifest(5, None, vec![entry("2.26.2", "0.1.0", None)]),
        );
    })
    .await;
    no_offer(&view);
    assert!(matches!(error, SupportError::Signature(_)), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_older_manifest_is_a_rollback_and_gives_no_offer() {
    let channel = Channel::start().await;
    let key = Key::new();
    let dir = scratch("support-s4");
    let rig = running_rig(&dir, &channel);
    let config = support(&channel, &key);
    let newer = manifest(20, None, vec![entry("2.26.2", "0.1.0", None)]);
    channel.publish(&key, &newer);
    rig.installer.check_support(&config).await.unwrap();
    assert_eq!(update(&rig).available.as_deref(), Some("2.26.2"));

    // An old, validly signed list (it would even offer more) is replayed.
    channel.publish(
        &key,
        &manifest(10, None, vec![entry("2.26.5", "0.1.0", None)]),
    );
    let error = rig.installer.check_support(&config).await.unwrap_err();
    assert_eq!(
        error,
        SupportError::Rollback {
            found: 10,
            seen: 20
        }
    );
    no_offer(&update(&rig));
    let seen = std::fs::read_to_string(dir.join("data/vibe/support-seen.json")).unwrap();
    assert!(seen.contains("20"), "{seen}");

    // The same list again is fine (a re-fetch is not a rollback).
    channel.publish(&key, &newer);
    rig.installer.check_support(&config).await.unwrap();
    assert_eq!(update(&rig).available.as_deref(), Some("2.26.2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn older_or_equal_versions_are_never_offered() {
    let channel = Channel::start().await;
    let key = Key::new();
    let rig = running_rig(&scratch("support-s5"), &channel);
    channel.publish(
        &key,
        &manifest(
            5,
            Some("2.26.0"),
            vec![
                entry("2.25.0", "0.1.0", None),
                entry("2.26.0", "0.1.0", None),
            ],
        ),
    );
    rig.installer
        .check_support(&support(&channel, &key))
        .await
        .unwrap();
    let view = update(&rig);
    assert_eq!(view.available, None);
    assert_eq!(view.upcoming, None);
    assert_eq!(view.check_error, None);
    assert!(matches!(
        rig.installer.prepare_update(),
        Err(InstallError::NoUpdate(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_manifest_or_signature_gives_no_offer_and_clears_an_earlier_one() {
    let channel = Channel::start().await;
    let key = Key::new();
    let rig = running_rig(&scratch("support-s6"), &channel);
    let config = support(&channel, &key);
    channel.publish(
        &key,
        &manifest(5, Some("2.27.0"), vec![entry("2.26.2", "0.1.0", None)]),
    );
    rig.installer.check_support(&config).await.unwrap();
    assert_eq!(update(&rig).available.as_deref(), Some("2.26.2"));

    channel.remove("vibe-support.json.sig");
    let error = rig.installer.check_support(&config).await.unwrap_err();
    assert!(error.to_string().contains("404"), "{error}");
    no_offer(&update(&rig));

    channel.remove("vibe-support.json");
    let error = rig.installer.check_support(&config).await.unwrap_err();
    assert!(error.to_string().contains("404"), "{error}");
    no_offer(&update(&rig));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signature_made_for_another_file_is_refused() {
    let (view, error) = refused("support-s7", |channel, key| {
        let bytes = manifest(5, None, vec![entry("2.26.2", "0.1.0", None)]);
        channel.put("vibe-support.json", bytes.clone());
        channel.put(
            "vibe-support.json.sig",
            sign_as(key, &bytes, "latest.json").into_bytes(),
        );
    })
    .await;
    no_offer(&view);
    assert_eq!(error, SupportError::OtherFile("latest.json".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn versions_for_other_app_versions_or_without_a_wheel_here_are_not_offered() {
    let channel = Channel::start().await;
    let key = Key::new();
    let rig = running_rig(&scratch("support-s8"), &channel);
    let mut no_wheel = entry("2.26.4", "0.1.0", None);
    no_wheel["wheels"] = serde_json::json!({ "riscv64-unknown-none": {
        "filename": "x.whl", "bytes": 1, "sha256": "22".repeat(32) } });
    channel.publish(
        &key,
        &manifest(
            5,
            Some("2.27.0"),
            vec![
                entry("2.26.2", "0.2.0", None),
                entry("2.26.3", "0.0.1", Some("0.0.9")),
                no_wheel,
            ],
        ),
    );
    rig.installer
        .check_support(&support(&channel, &key))
        .await
        .unwrap();
    let view = update(&rig);
    assert_eq!(view.available, None, "{view:?}");
    assert_eq!(view.upcoming.as_deref(), Some("2.27.0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsupported_own_install_stays_in_setup_with_the_required_version() {
    let channel = Channel::start().await;
    let key = Key::new();
    let dir = scratch("support-s9");
    let root = dir.join("data/vibe");
    // The recorded choice points to an executable path; support checks never launch it.
    install_marker(&dir.join("own"), "2.25.0");
    let own = vw_vibe_install::layout::Layout::new(dir.join("own"), "2.25.0")
        .unwrap()
        .server();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("choice.json"),
        serde_json::json!({"path": own, "version": "2.25.0"}).to_string(),
    )
    .unwrap();
    let rig = running_rig(&dir, &channel);
    channel.publish(
        &key,
        &manifest(5, Some("2.27.0"), vec![entry("2.26.2", "0.1.0", None)]),
    );
    rig.installer
        .check_support(&support(&channel, &key))
        .await
        .unwrap();
    let view = update(&rig);
    assert_eq!(view.available, None);
    assert_eq!(view.upcoming, None);
    let install = rig.installer.view();
    assert!(matches!(
        install.phase,
        vw_vibe_install::InstallPhase::NotInstalled {}
    ));
    assert_eq!(install.existing.len(), 1);
    let own = &install.existing[0];
    assert!(!own.supported);
    assert_eq!(
        own.warning.as_deref(),
        Some("This install is Vibe 2.25.0; Vibe Work needs Vibe 2.26.0.")
    );
    assert!(matches!(
        rig.installer.prepare_update(),
        Err(InstallError::NoUpdate(_))
    ));
}

#[test]
fn other_schemas_and_malformed_entries_are_refused() {
    let key = Key::new();
    let check = |bytes: &[u8]| {
        vw_vibe_install::support::verify(
            bytes,
            &sign_as(&key, bytes, "vibe-support.json"),
            &key.pubkey(),
        )
    };
    let mut schema2: serde_json::Value =
        serde_json::from_slice(&manifest(5, None, vec![entry("2.26.2", "0.1.0", None)])).unwrap();
    schema2["schema"] = 2.into();
    assert_eq!(
        check(&serde_json::to_vec(&schema2).unwrap()),
        Err(SupportError::Schema(2))
    );
    let mut bad = entry("2.26.2", "0.1.0", None);
    bad["version"] = "../2.26.2".into();
    assert!(matches!(
        check(&manifest(5, None, vec![bad])),
        Err(SupportError::Malformed(_))
    ));
    let mut bad = entry("2.26.2", "0.1.0", None);
    bad["requirements"] = "../../etc/passwd".into();
    assert!(matches!(
        check(&manifest(5, None, vec![bad])),
        Err(SupportError::Malformed(_))
    ));
    assert!(check(&manifest(5, None, vec![entry("2.26.2", "0.1.0", None)])).is_ok());
}

#[tokio::test]
async fn the_companion_check_runs_with_each_update_check_only() {
    let rig = common::update_rig::Rig::new("support-s12");
    let calls = Arc::new(AtomicU32::new(0));
    let counted = calls.clone();
    rig.manager.set_companion_check(Arc::new(move || {
        let counted = counted.clone();
        Box::pin(async move {
            counted.fetch_add(1, Ordering::SeqCst);
        })
    }));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0, "never on its own");
    rig.manager.check().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    rig.manager.check().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// A rig with a signed sequence-20 manifest that would offer a newer Vibe.
async fn seen_rig(
    name: &str,
) -> (
    Channel,
    common::vibe_rig::Rig,
    vw_vibe_install::SupportConfig,
    std::path::PathBuf,
) {
    let channel = Channel::start().await;
    let key = Key::new();
    let dir = scratch(name);
    let rig = running_rig(&dir, &channel);
    let config = support(&channel, &key);
    channel.publish(
        &key,
        &manifest(20, None, vec![entry("2.26.2", "0.1.0", None)]),
    );
    (channel, rig, config, dir.join("data/vibe"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_rollback_record_gives_no_offer_until_it_is_removed() {
    let (_channel, rig, config, root) = seen_rig("support-s16").await;
    let record = root.join("support-seen.json");
    std::fs::write(&record, b"{\"sequence\": 30").unwrap();

    let error = rig.installer.check_support(&config).await.unwrap_err();
    assert!(matches!(error, SupportError::State(_)), "{error:?}");
    let view = update(&rig);
    no_offer(&view);
    let shown = view.check_error.unwrap();
    assert!(
        shown.contains("rollback record") && shown.contains("support-seen.json"),
        "{shown}"
    );
    assert_eq!(
        std::fs::read(&record).unwrap(),
        b"{\"sequence\": 30",
        "the damaged record is evidence: never rewritten by a check"
    );

    // The documented recovery: delete the record; the next check starts from 0 like a fresh install.
    std::fs::remove_file(&record).unwrap();
    rig.installer.check_support(&config).await.unwrap();
    assert_eq!(update(&rig).available.as_deref(), Some("2.26.2"));
    assert!(std::fs::read_to_string(&record).unwrap().contains("20"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rollback_record_that_is_not_a_file_gives_no_offer() {
    let (_channel, rig, config, root) = seen_rig("support-s17").await;
    std::fs::create_dir_all(root.join("support-seen.json")).unwrap();
    let error = rig.installer.check_support(&config).await.unwrap_err();
    assert!(matches!(error, SupportError::State(_)), "{error:?}");
    no_offer(&update(&rig));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_sequence_that_cannot_be_saved_gives_no_offer() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_channel, rig, config, root) = seen_rig("support-s18").await;
    let before = std::fs::metadata(&root).unwrap().permissions();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500)).unwrap();
    let outcome = rig.installer.check_support(&config).await;
    std::fs::set_permissions(&root, before).unwrap();

    let error = outcome.unwrap_err();
    assert!(
        matches!(&error, SupportError::State(text) if text.contains("cannot save")),
        "{error:?}"
    );
    no_offer(&update(&rig));
    assert!(!root.join("support-seen.json").exists());

    rig.installer.check_support(&config).await.unwrap();
    assert_eq!(update(&rig).available.as_deref(), Some("2.26.2"));
    let saved = std::fs::read_to_string(root.join("support-seen.json")).unwrap();
    assert_eq!(saved, r#"{"sequence":20}"#);
    let leftovers: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("support-seen.json."))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_planted_temporary_symlink_is_never_written_through() {
    let (_channel, rig, config, root) = seen_rig("support-s19").await;
    let victim = root.parent().unwrap().join("victim.txt");
    std::fs::write(&victim, b"keep me").unwrap();
    std::os::unix::fs::symlink(&victim, root.join("support-seen.json.tmp")).unwrap();

    rig.installer.check_support(&config).await.unwrap();
    assert_eq!(update(&rig).available.as_deref(), Some("2.26.2"));
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
    assert!(
        std::fs::read_to_string(root.join("support-seen.json"))
            .unwrap()
            .contains("20")
    );
}
