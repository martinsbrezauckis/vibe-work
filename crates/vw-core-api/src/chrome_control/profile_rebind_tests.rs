//! A chat's external profile binding is released only by a complete ended-session receipt: completed, disconnected
//! and fenced sessions free it, while incomplete, in-flight or stale results never retire a live binding.
use super::profile_binding_tests::{connected, open};
use super::test_support;
use serde_json::json;

#[cfg(feature = "your-firefox")]
fn write_card(browser: &str, session: &str, action: &str) -> serde_json::Value {
    json!({"vwToolAsk":true,"vwBrowser":browser,"detail":{"kind":"approval","vwBrowserTier":1,
        "effect":{"toolName":"mcp_vw.chrome_execute","input":{"sessionId":session,"action":action,"args":{"ref":"submit","text":"hello"}}},
        "requiredPermissions":[]}})
}

/// PREDICT: complete helper receipts allow Chrome, then Edge, then Firefox in
/// one chat; its write grant carries forward while a second chat stays isolated.
#[allow(
    clippy::too_many_lines,
    reason = "one sequential Chrome, Edge and Firefox rebind checks grant carry and chat isolation"
)]
// It ends in Your Firefox: builds without Firefox support (the edge+safari feature set) cannot run it.
#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn completed_old_session_allows_a_new_browser_without_changing_another_chat() {
    let core = connected("profile-rebind-completed").await;
    open(&core, "chrome:work").await.unwrap();
    let first = write_card(vw_vocabulary::texts::CHROME_NAME, "task", "page.type");
    assert!(core.shared.ask_grants.matching("s", &first).is_none());
    core.shared
        .ask_grants
        .note_callback("write", "s", "s", crate::ask::grants::scope_of(&first));
    core.shared
        .ask_grants
        .keep(
            core.shared.ask_grants.claim("write").unwrap(),
            vw_ask::AnsweredBy::Person,
            1,
        )
        .unwrap();
    test_support::chat(&core.shared, "other");
    core.shared
        .chrome_control
        .picked_profiles
        .lock()
        .unwrap()
        .insert("s".into(), "chrome:work".into());
    super::profile_bindings::remember(
        &core.shared.chrome_control,
        "other",
        "chrome:personal",
        None,
    )
    .unwrap();
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off"}]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    assert!(
        core.shared
            .chrome_control
            .bindings
            .lock()
            .unwrap()
            .check_session("other", "task", "chrome:personal")
            .is_err(),
        "old receipt identity must outlive its chat binding"
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "other").as_deref(),
        Some("chrome:personal")
    );
    assert!(
        !core
            .shared
            .chrome_control
            .picked_profiles
            .lock()
            .unwrap()
            .contains_key("s")
    );
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[{"profileId":"edge:work","name":"Work","connected":true}],"sessions":[{"clientSessionId":"s","sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off"}]}})
        } else if args["action"] == "open" {
            json!({"structuredContent":{"sessionId":"edge-task","profileId":"edge:work","state":"active"}})
        } else {
            json!({"structuredContent":{}})
        }
    });
    open(&core, "edge:work").await.unwrap();
    let send = write_card(vw_vocabulary::texts::EDGE_NAME, "edge-task", "page.commit");
    assert!(
        core.shared.ask_grants.matching("s", &send).is_some(),
        "same chat's write package should cover the next browser's Send action"
    );
    assert!(core.shared.ask_grants.matching("other", &send).is_none());
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("edge:work")
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "other").as_deref(),
        Some("chrome:personal")
    );
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"edge-task","profileId":"edge:work","state":"stopped","capture":"off"}]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    *core
        .shared
        .chrome_control
        .owner
        .firefox_pairing
        .lock()
        .unwrap() = (super::PairingState::WindowOpen, None);
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[{"profileId":"firefox:agent","name":"Work","connected":true}],"sessions":[{"clientSessionId":"s","sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off"},{"clientSessionId":"s","sessionId":"edge-task","profileId":"edge:work","state":"stopped","capture":"off"}]}})
        } else if args["action"] == "open" {
            json!({"structuredContent":{"sessionId":"firefox-task","profileId":"firefox:agent","state":"active"}})
        } else {
            json!({"structuredContent":{}})
        }
    });
    open(&core, "firefox:agent").await.unwrap();
    let send = write_card(
        vw_vocabulary::texts::FIREFOX_NAME,
        "firefox-task",
        "page.commit",
    );
    assert!(core.shared.ask_grants.matching("s", &send).is_some());
    assert!(core.shared.ask_grants.matching("other", &send).is_none());
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("firefox:agent")
    );
    core.shutdown().await;
}

/// PREDICT: an absent old task in a complete helper inventory releases its
/// saved binding after the connection has ended.
#[tokio::test]
async fn disconnected_old_session_releases_the_external_binding() {
    let core = connected("profile-rebind-disconnected").await;
    open(&core, "chrome:work").await.unwrap();
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    assert!(
        !core
            .shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .contains_key("task")
    );
    assert_eq!(
        core.shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .state_of("task", "s"),
        Some("disconnected")
    );
    core.shutdown().await;
}

/// PREDICT: a fenced adapter disconnect releases the old binding even while
/// the helper retains its terminal task row.
#[tokio::test]
async fn a_fenced_adapter_disconnect_releases_the_external_binding() {
    let core = connected("profile-rebind-adapter-disconnect").await;
    open(&core, "chrome:work").await.unwrap();
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"disconnected","capture":"unknown","suspensionReason":"ADAPTER_DISCONNECTED"}]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    core.shutdown().await;
}

/// PREDICT: a missing or unfinished stop receipt is not a completed release and must not
/// let an unrelated profile enter this chat.
#[tokio::test]
async fn incomplete_release_never_frees_a_live_binding() {
    let core = connected("profile-rebind-incomplete").await;
    open(&core, "chrome:work").await.unwrap();
    for result in [
        json!({"structuredContent":{}}),
        json!({"isError":true,"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[]}}),
        json!({"structuredContent":{"version":1,"clientId":"vibework-owner","status":"unknown","profiles":[],"sessions":[]}}),
        json!({"structuredContent":{"version":1,"clientId":"another-client","profiles":[],"sessions":[]}}),
        json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{}]}}),
        json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"stopping","capture":"on"}]}}),
        json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"on"}]}}),
        json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"stopped"}]}}),
        json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"disconnected","capture":"unknown"}]}}),
        json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"edge:work","state":"stopped","capture":"off"}]}}),
    ] {
        super::owner::update_work_for_test(&core.shared, &result);
        assert_eq!(
            super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
            Some("chrome:work")
        );
        assert!(open(&core, "edge:work").await.is_err());
    }
    core.shutdown().await;
}

/// PREDICT: an unleased pick survives empty status, while a later first use of
/// another external profile becomes this chat's locked profile.
#[tokio::test]
async fn a_profile_picked_without_an_open_stays_bound_during_an_empty_status() {
    let core = connected("profile-rebind-picked-only").await;
    super::profile_bindings::remember(&core.shared.chrome_control, "s", "chrome:work", None)
        .unwrap();
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("chrome:work")
    );
    open(&core, "edge:work").await.unwrap();
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("edge:work")
    );
    core.shutdown().await;
}

/// PREDICT: a status result cannot retire a new lease while the gateway holds
/// the binding transaction; a later complete poll may retire an ended lease.
#[tokio::test]
async fn an_in_flight_open_blocks_status_retirement_until_the_next_poll() {
    let core = connected("profile-rebind-open-race").await;
    open(&core, "chrome:work").await.unwrap();
    let held = core.shared.chrome_control.binding_changes.lock().await;
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("chrome:work")
    );
    drop(held);
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_old_status_started_before_the_new_open_cannot_retire_it() {
    let core = connected("profile-rebind-stale-status").await;
    open(&core, "chrome:work").await.unwrap();
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off"}]}}),
    );
    let stale_started = tokio::time::Instant::now();
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[{"profileId":"edge:work","name":"Work","connected":true}],"sessions":[{"clientSessionId":"s","sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off"}]}})
        } else if args["action"] == "open" {
            json!({"structuredContent":{"sessionId":"edge-task","profileId":"edge:work","state":"active"}})
        } else {
            json!({"structuredContent":{}})
        }
    });
    open(&core, "edge:work").await.unwrap();
    super::owner::update_work(
        &std::sync::Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off"}]}}),
        stale_started,
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("edge:work")
    );
    assert_eq!(
        core.shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .state_of("edge-task", "s"),
        Some("active")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_retired_receipt_cannot_resurrect_its_profile_from_stale_active_history() {
    let core = connected("profile-rebind-history-race").await;
    open(&core, "chrome:work").await.unwrap();
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"task","profileId":"chrome:work","state":"stopped","capture":"off"}]}}),
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    super::profile_bindings::migrate(
        &core.shared.chrome_control,
        "s",
        &json!({"sessions":[{"clientSessionId":"s","sessionId":"task","profileId":"chrome:work","state":"active"}]}),
    )
    .unwrap();
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    core.shutdown().await;
}
