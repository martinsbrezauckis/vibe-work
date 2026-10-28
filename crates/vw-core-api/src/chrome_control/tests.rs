//! The Chrome Control state: rows read once from the state folder, and cards woven into a session's history by time.
use super::*;

fn view(id: &str, session: &str, at: u64) -> ChromeAskView {
    ChromeAskView {
        ask_id: id.into(),
        session_id: session.into(),
        turn_id: None,
        created_at_ms: at,
        tool: "mcp_vw.chrome_execute".into(),
        summary: "Send a form or confirm".into(),
        group: Some(Group::Send),
        reason: "why".into(),
        covers: Vec::new(),
        input: json!({}),
        open: false,
    }
}

#[test]
fn cards_are_woven_by_time_and_a_repeated_overlay_replaces_them() {
    let dir = std::env::temp_dir().join(format!("vw-cc-overlay-{}", std::process::id()));
    let chrome = ChromeControl::new(&dir.join("vibe-home"), ChromeControlConfig::default());
    chrome.upsert(view("a2", "s", 25));
    chrome.upsert(view("a1", "s", 15));
    chrome.upsert(view("x", "other", 15));
    let mut state = json!({"history": [
        {"id": "user", "type": "message", "createdAt": 10},
        {"id": "effect", "type": "effect", "createdAt": 20},
        {"id": "answer", "type": "message", "createdAt": 30}
    ]});
    chrome.overlay("s", &mut state);
    let ids: Vec<_> = state["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["user", "chrome:a1", "effect", "chrome:a2", "answer"]);
    let first = state.clone();
    chrome.overlay("s", &mut state);
    assert_eq!(state, first);
    assert_eq!(state["history"][1]["chromeAsk"]["group"], "send");
}

#[test]
fn a_saved_raise_waits_at_browse_and_an_unknown_shape_reads_as_the_default() {
    let data = std::env::temp_dir().join(format!("vw-cc-level-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let home = data.join("vibe-home");
    let chrome = ChromeControl::new(
        &home,
        ChromeControlConfig {
            package: Some(data.join("package")),
            user_home: Some(data.clone()),
            ..ChromeControlConfig::default()
        },
    );
    assert_eq!(
        chrome.dir(),
        Some(data.join("VibeWork").join("ChromeControl").as_path())
    );
    #[cfg(windows)]
    assert!(!chrome.dir().unwrap().to_str().unwrap().contains('/'));
    let full = ChromeSettings {
        level: vw_ask::chrome::Level::Act,
    };
    chrome.save(full).unwrap();
    assert_eq!(
        ChromeControl::new(
            &home,
            ChromeControlConfig {
                package: Some(data.join("package")),
                user_home: Some(data.clone()),
                ..ChromeControlConfig::default()
            }
        )
        .settings(),
        ChromeSettings::default()
    );
    // A file in another shape (written by hand, the earlier per-row version): the default.
    std::fs::write(
        data.join("VibeWork")
            .join("ChromeControl")
            .join("permissions.json"),
        br#"{"version":1,"permissions":{"rows":{"send":"allowed"}}}"#,
    )
    .unwrap();
    assert_eq!(
        ChromeControl::new(
            &home,
            ChromeControlConfig {
                package: Some(data.join("package")),
                user_home: Some(data.clone()),
                ..ChromeControlConfig::default()
            }
        )
        .settings(),
        ChromeSettings::default()
    );
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn absent_package_ignores_saved_raises_and_cannot_write_state() {
    let root = std::env::temp_dir().join(format!("vw-nocc-state-{}", std::process::id()));
    let config = ChromeControlConfig {
        user_home: Some(root.clone()),
        ..Default::default()
    };
    let state = root.join("VibeWork/ChromeControl");
    store::save(
        &state,
        ChromeSettings {
            level: vw_ask::chrome::Level::Act,
        },
    )
    .unwrap();
    let before = std::fs::read(state.join("permissions.json")).unwrap();
    let chrome = ChromeControl::new(&root.join("vibe-home"), config.clone());
    assert!(chrome.dir().is_none());
    assert!(chrome.current().restore.is_none());
    let view = setup::view(&config, Some(&state), &root);
    assert_eq!(view.state, setup::SetupState::Absent);
    assert!(!view.package);
    assert_eq!(view.profiles, Vec::<String>::new());
    assert_eq!(view.store_url, "");
    assert_eq!(view.pairing, setup::ABSENT);
    chrome.save(ChromeSettings::default()).unwrap();
    assert_eq!(
        std::fs::read(state.join("permissions.json")).unwrap(),
        before
    );
    std::fs::remove_dir_all(&root).unwrap();
    let chrome = ChromeControl::new(&root, config);
    chrome.save(ChromeSettings::default()).unwrap();
    assert!(!root.exists());
}

#[test]
fn release_package_requires_a_pin_and_a_damaged_package_is_distinct_from_absence() {
    let root = std::env::temp_dir().join(format!("vw-nocc-bundle-{}", std::process::id()));
    let mut config = ChromeControlConfig {
        helper_sha256: None,
        ..Default::default()
    };
    config.bundled(&root);
    assert!(!config.available());
    config.helper_sha256 = Some("a".repeat(64));
    config.bundled(&root);
    assert!(config.available());
    let view = setup::view(&config, None, &root);
    assert_eq!(view.state, setup::SetupState::NotSetUp);
    assert!(!view.package);
    assert_eq!(
        view.pairing,
        "This Vibe Work installation has no Chrome Control helper. Reinstall Vibe Work."
    );
}

/// Opens (or, on the second call, consumes) the exact-call Chrome card for `input` in chat `s`.
async fn chrome_card(
    shared: &std::sync::Arc<crate::core::Shared>,
    then: impl FnOnce(),
) -> Result<vw_ask::Outcome, String> {
    super::pending::answer(shared, "s", || {
        // `then` runs once the call was taken and before the card's ask exists.
        then();
        let handle = shared.asks.open_own(
            vw_ask::chrome::OTHER_KIND,
            Some("s".into()),
            vw_ask::Requester::Agent,
            vec![vw_ask::Choice::Once, vw_ask::Choice::No],
            None,
        );
        let card = ChromeAskView {
            ask_id: handle.ask.id.clone(),
            ..view("unused", "s", 1)
        };
        (handle, card)
    })
    .await
}

/// An Allow given before the person pressed Stop is not consumed afterwards: the exact retry of the same Chrome
/// action is refused instead of running without a new card.
#[tokio::test]
async fn a_chrome_allow_from_before_a_stop_authorizes_nothing_after_it() {
    let core = crate::offline_core::start("chrome-allow-stop").await;
    let shared = &core.shared;
    let mut pending = Box::pin(chrome_card(shared, || {}));
    assert!(
        std::future::Future::poll(
            pending.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop())
        )
        .is_pending()
    );
    let id = shared.asks.list()[0].id.clone();
    assert!(shared.asks.settle(
        &id,
        vw_ask::Outcome::Allowed {
            scope: vw_ask::Scope::Once,
            by: vw_ask::AnsweredBy::Person,
        }
    ));
    crate::stops::stopped(shared, "s");
    assert!(pending.await.is_err());
}

/// A Stop that lands while the Chrome card is being opened (after its call was checked, before its ask exists)
/// leaves no card open on the stopped chat.
#[tokio::test]
async fn a_chrome_card_that_opens_across_a_stop_closes_at_once() {
    let core = crate::offline_core::start("chrome-card-stop").await;
    let shared = core.shared.clone();
    let stopping = shared.clone();
    let _ = chrome_card(&shared, move || {
        crate::stops::stopped(&stopping, "s");
    })
    .await;
    assert!(
        shared.asks.list().is_empty(),
        "no Chrome card stays open on a stopped chat: {:?}",
        shared.asks.list()
    );
}
