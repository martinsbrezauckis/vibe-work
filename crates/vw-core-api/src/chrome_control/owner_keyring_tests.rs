//! A computer without a system keyring (Linux with no Secret Service): the cards and Connect name the keyring.
use super::*;
use serde_json::json;

/// PREDICT (written before the change): the owner identity cannot be read, so the helper never starts, but only a
/// log line says so. Both cards then read "Not running" under an "Extension not found" line, Connect only opens the
/// store page, and Edge's Remove answers with the sentence worded for Your Chrome. Expected: both cards are in the
/// keyring state with the one sentence worded for their own browser, and Connect, Edge's store button and Edge's
/// Remove answer with that sentence without opening the store page.
#[tokio::test]
async fn no_system_keyring_is_on_both_cards_and_in_every_answer() {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("browser-keyring-cards-{}", std::process::id()));
    let keys = owner_credential_tests::keys_without_keyring(&root).await;
    let core = crate::offline_core::start_with("browser-keyring-cards", |config, dir| {
        config.keys = Some(keys.clone());
        config.chrome_control.user_home = Some(dir.join("home"));
        config.chrome_control.macos = Some(false);
        std::fs::create_dir_all(dir.join("package/bin")).unwrap();
        std::fs::write(
            dir.join("package/bin").join(setup::helper_name()),
            b"helper",
        )
        .unwrap();
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    test_support::answering(&core.shared, |_, _| json!({}));
    let chrome = &core.shared.chrome_control;
    chrome.owner.stop().await;
    let state = chrome.dir().unwrap();
    std::fs::write(
        state.join("setup.json"),
        br#"{"files":[],"browsers":["Google Chrome","Microsoft Edge"],"selectedBrowsers":["Google Chrome","Microsoft Edge"],"userData":[]}"#,
    )
    .unwrap();
    std::fs::write(state.join("paired-profiles.json"), b"{}").unwrap();
    start_owner_for(&core.shared, None).await;

    let sentence = |browser: &str| {
        vw_vocabulary::texts::CHROME_OWNER_STORE_UNAVAILABLE.replace("{yourBrowser}", browser)
    };
    let view = core.chrome_view();
    #[cfg_attr(not(feature = "your-edge"), allow(unused_mut))]
    let mut cards = vec![(&view.setup, "Your Chrome")];
    #[cfg(feature = "your-edge")]
    cards.push((view.edge.as_deref().unwrap(), "Your Edge"));
    for (card, browser) in cards {
        let addon = card.addon.as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(addon.state).unwrap(),
            "ownerKeyringUnavailable",
            "{browser}"
        );
        assert_eq!(addon.step.as_deref(), Some(sentence(browser).as_str()));
        assert_eq!(addon.instruction, sentence(browser));
    }

    let connect = core
        .chrome_control(
            crate::Origin::Ui,
            commands::ChromeControlRequest::Connect {},
        )
        .await
        .unwrap_err();
    assert_eq!(connect.message, sentence("Your Chrome"));
    #[cfg(feature = "your-edge")]
    {
        let store = core
            .chrome_control(
                crate::Origin::Ui,
                commands::ChromeControlRequest::OpenEdgeStore {},
            )
            .await
            .unwrap_err();
        assert_eq!(store.message, sentence("Your Edge"));
        let removed = core
            .chrome_control(
                crate::Origin::Ui,
                commands::ChromeControlRequest::RemoveEdge {},
            )
            .await
            .unwrap_err();
        assert_eq!(removed.message, sentence("Your Edge"));
        assert!(!removed.message.contains("Your Chrome"));
    }
    core.shutdown().await;
    keys.shutdown().await;
}

/// PREDICT (written before the change): the app chose its credential store once at start, so after the person set
/// up the keyring while the app stayed open, Connect still answered with the keyring sentence until a restart.
/// Expected: the first Connect names the keyring; once the keyring is there, the next Connect no longer does.
#[tokio::test]
async fn a_keyring_set_up_while_the_app_runs_is_used_by_the_next_connect() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("browser-keyring-later-{}", std::process::id()));
    let ready = Arc::new(AtomicBool::new(false));
    let os = Arc::new(vw_vault::remember::MemoryDeviceStore::default());
    let (opener_ready, opener_os) = (Arc::clone(&ready), Arc::clone(&os));
    let late = vw_vault::remember_late::LateDeviceStore::with_retry(
        Box::new(move || {
            if opener_ready.load(Ordering::SeqCst) {
                Ok(Arc::clone(&opener_os) as Arc<dyn vw_vault::remember::DeviceSecretStore>)
            } else {
                Err(vw_vault::VaultError::Remember(
                    "no Secret Service".to_owned(),
                ))
            }
        }),
        std::time::Duration::ZERO,
    );
    let keys = owner_credential_tests::keys_with(&root, late).await;
    let core = crate::offline_core::start_with("browser-keyring-later", |config, dir| {
        config.keys = Some(keys.clone());
        config.chrome_control.user_home = Some(dir.join("home"));
        config.chrome_control.macos = Some(false);
        std::fs::create_dir_all(dir.join("package/bin")).unwrap();
        std::fs::write(
            dir.join("package/bin").join(setup::helper_name()),
            b"helper",
        )
        .unwrap();
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    test_support::answering(&core.shared, |_, _| json!({}));
    let chrome = &core.shared.chrome_control;
    chrome.owner.stop().await;
    let state = chrome.dir().unwrap();
    std::fs::write(
        state.join("setup.json"),
        br#"{"files":[],"browsers":["Google Chrome"],"selectedBrowsers":["Google Chrome"],"userData":[]}"#,
    )
    .unwrap();
    std::fs::write(state.join("paired-profiles.json"), b"{}").unwrap();
    start_owner_for(&core.shared, None).await;
    let keyring = vw_vocabulary::texts::CHROME_OWNER_STORE_UNAVAILABLE
        .replace("{yourBrowser}", "Your Chrome");
    let connect = || async {
        core.chrome_control(
            crate::Origin::Ui,
            commands::ChromeControlRequest::Connect {},
        )
        .await
    };
    assert_eq!(connect().await.unwrap_err().message, keyring);
    // The person starts and unlocks the system keyring; the app stays open.
    ready.store(true, Ordering::SeqCst);
    let again = connect().await;
    assert!(
        again.as_ref().err().is_none_or(|e| e.message != keyring),
        "the keyring is used without a restart: {again:?}"
    );
    assert!(
        !chrome
            .owner
            .keyring_unavailable
            .load(std::sync::atomic::Ordering::SeqCst),
        "the keyring state is gone"
    );
    core.shutdown().await;
    keys.shutdown().await;
}
