//! Optional Safari routing keeps the shared mode/ownership checks while allowing local website addresses.
use super::*;
use serde_json::json;

#[tokio::test]
async fn safari_relay_failure_is_visible_without_replacing_chromes_state() {
    let core = crate::offline_core::start_with("safari-relay-failure", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let dir = shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    shared
        .chrome_control
        .owner
        .connected_profiles
        .observe(&json!({"profiles":[
            {"profileId":"chrome:p","connected":true,"extensionVersion":"0.1.5"},
            {"profileId":"safari:p","connected":true,"extensionVersion":crate::version()}
        ]}));
    let before = shared.chrome_control.owner.connected_profiles.snapshot();
    owner::observe(
        &std::sync::Arc::downgrade(shared),
        &json!({
            "method":"notifications/owner/safari", "params":{"state":"unavailable"}
        }),
    )
    .unwrap();
    let view = safari::view(shared).unwrap();
    assert_eq!(view.addon.state, AddonState::HelperSetupRequired);
    assert_eq!(view.addon.label, "Helper setup required");
    assert!(
        view.addon
            .instruction
            .contains("could not start its connection")
    );
    assert!(view.addon.instruction.contains("restart Vibe Work"));
    assert_eq!(
        shared.chrome_control.owner.connected_profiles.snapshot(),
        before
    );
    std::fs::write(dir.join("safari-enabled.json"), b"false").unwrap();
    assert_eq!(
        safari::view(shared).unwrap().addon.state,
        AddonState::SwitchedOff
    );
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    owner::observe(
        &std::sync::Arc::downgrade(shared),
        &json!({
            "method":"notifications/owner/safari", "params":{"state":"ready"}
        }),
    )
    .unwrap();
    assert_eq!(
        safari::view(shared).unwrap().addon.state,
        AddonState::Connected
    );
    core.shutdown().await;
}

#[tokio::test]
async fn safari_profile_arguments_cannot_override_an_existing_browser_lease() {
    let core = crate::offline_core::start_with("safari-bound-browser", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    auto_approve_chats(shared);
    shared.chrome_control.set_enabled(false).unwrap();
    let dir = shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    for (id, profile, override_profile, expected) in [
        (
            "chrome-task",
            "chrome:p",
            "safari:p",
            agent_browser::Browser::Chrome,
        ),
        (
            "safari-task",
            "safari:p",
            "chrome:p",
            agent_browser::Browser::Safari,
        ),
    ] {
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .open(id, "s", "active", profile, None);
        let args =
            json!({"sessionId":id,"profileId":override_profile,"action":"page.read","args":{}});
        assert_eq!(
            agent_browser::selected(shared, "s", "chrome_execute", &args),
            expected
        );
        if expected == agent_browser::Browser::Chrome {
            assert!(check::authorize(shared, "s", "chrome_execute", &args).is_err());
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn safari_off_between_reply_validation_and_binding_cannot_leave_a_live_lease() {
    let core = crate::offline_core::start_with("safari-off-binding-race", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    auto_approve_chats(shared);
    shared.chrome_control.set_enabled(true).unwrap();
    let calls = test_support::answering(shared, |_, _| json!({}));
    let browser = agent_browser::Browser::Safari;
    let pending = person_stop::PendingCall::new(
        shared,
        "s",
        "chrome_session",
        &json!({"action":"open", "profileId":"safari:p"}),
    );
    let mut answer = owner::Answer::Helper(
        json!({"structuredContent":{"sessionId":"late","profileId":"safari:p","state":"active"}}),
    );
    pending.finish(shared, "s", browser, &answer).unwrap();
    safari::disabled(shared).await;
    assert!(
        gateway::scope_result(
            shared,
            "s",
            "chrome_session",
            true,
            browser,
            &pending,
            &mut answer
        )
        .is_err()
    );
    assert!(
        !shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .contains_key("late")
    );
    tokio::task::yield_now().await;
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "stop" && args["sessionId"] == "late")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn safari_off_revokes_only_safari_sessions_and_late_open_replies() {
    let core = crate::offline_core::start_with("safari-off-sessions", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let chrome = &shared.chrome_control;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    chrome.set_enabled(true).unwrap();
    let calls = test_support::answering(shared, |_, _| json!({}));
    for (id, profile) in [("c", "chrome:p"), ("a", "safari:p"), ("old", "safari:p")] {
        chrome
            .work
            .lock()
            .unwrap()
            .open(id, "s", "active", profile, None);
    }
    chrome
        .tasks
        .lock()
        .unwrap()
        .extend([("c".into(), "s".into()), ("old".into(), "s".into())]);
    let pending = person_stop::PendingCall::new(
        shared,
        "s",
        "chrome_session",
        &json!({"action":"open", "profileId":"safari:p"}),
    );
    safari::disabled(shared).await;
    assert!(!chrome.tasks.lock().unwrap().contains_key("old"));
    assert_eq!(
        chrome.tasks.lock().unwrap().get("c").map(String::as_str),
        Some("s")
    );
    assert_eq!(
        chrome.work.lock().unwrap().state_of("old", "s"),
        Some("stopping")
    );
    assert_eq!(
        chrome.work.lock().unwrap().state_of("c", "s"),
        Some("active")
    );
    let late = owner::Answer::Helper(
        json!({"structuredContent":{"sessionId":"late","profileId":"safari:p"}}),
    );
    let refusal = pending
        .finish(shared, "s", agent_browser::Browser::Safari, &late)
        .unwrap_err();
    assert!(
        refusal.contains("Safari") && refusal.contains("stopped"),
        "{refusal}"
    );
    pending
        .finish(shared, "s", agent_browser::Browser::Chrome, &late)
        .unwrap();
    tokio::task::yield_now().await;
    let stops: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, args)| args["action"] == "stop")
        .map(|(_, args)| args["sessionId"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        stops.contains(&"old".into()) && stops.contains(&"late".into()),
        "{stops:?}"
    );
    assert!(!stops.contains(&"c".into()));
    core.shutdown().await;
}

#[tokio::test]
async fn safari_work_keeps_its_browser_label() {
    let core = crate::offline_core::start("safari-work-label").await;
    let chrome = &core.shared.chrome_control;
    chrome
        .work
        .lock()
        .unwrap()
        .open("a", "s", "active", "safari:p", None);
    chrome
        .chat_profiles
        .lock()
        .unwrap()
        .insert("s".into(), "safari:p".into());
    assert_eq!(work::view(chrome)[0].browser.as_deref(), Some("Safari"));
    core.shutdown().await;
}

#[tokio::test]
async fn safari_uses_its_own_switch_and_browser_identity() {
    let core = crate::offline_core::start_with("safari-routing-decision", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    shared.chrome_control.set_enabled(false).unwrap();
    std::fs::create_dir_all(shared.chrome_control.dir().unwrap()).unwrap();
    std::fs::write(
        shared
            .chrome_control
            .dir()
            .unwrap()
            .join("safari-enabled.json"),
        b"true",
    )
    .unwrap();
    let args = json!({"action":"open","profileId":"safari:p"});
    assert_eq!(
        agent_browser::selected(shared, "s", "chrome_session", &args).id(),
        "safari"
    );
    assert!(check::authorize(shared, "s", "chrome_session", &args).is_ok());
    shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"plan"}));
    assert!(check::authorize(shared, "s", "chrome_session", &args).is_err());
    assert!(!shared.chrome_control.enabled());
    core.shutdown().await;
}

#[tokio::test]
async fn safari_local_addresses_do_not_use_chromes_network_switch_or_dns_preflight() {
    let core = crate::offline_core::start("safari-local-sites-decision").await;
    let shared = &core.shared;
    shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "safari:p", None);
    for url in [
        "http://127.0.0.1:8123/",
        "http://10.1.2.3/",
        "http://[::1]/",
        "http://printer.local/",
    ] {
        let mut args = json!({"sessionId":"t","action":"tabs.create","args":{"url":url}});
        network::prepare(shared, "s", &mut args).await.unwrap();
        assert!(args.get("_netPolicy").is_none(), "{args}");
    }
    core.shutdown().await;
}

#[tokio::test]
async fn safari_gateway_opens_and_runs_with_chrome_off_and_keeps_chat_ownership() {
    let core = crate::offline_core::start_with("safari-gateway-enabled", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    auto_approve_chats(shared);
    let profile = json!({"profileId":"safari:p","connected":true,"extensionVersion":crate::version(),"features":["tabs.create","page.read"],"extensionCapabilities":["tabs.create","page.read"]});
    let status = json!({"structuredContent":{"profiles":[profile],"sessions":[{"sessionId":"t","profileId":"safari:p"}]}});
    let calls = test_support::answering(shared, move |tool, args| match tool {
        "chrome_status" => status.clone(),
        "chrome_session" => {
            json!({"structuredContent":{"sessionId":"t","profileId":"safari:p","state":"active"}})
        }
        "chrome_execute" => {
            assert!(args.get("_netPolicy").is_none());
            json!({"structuredContent":{"sessionId":"t","status":"completed","data":{"refs":[]}}})
        }
        _ => panic!("unexpected tool {tool}"),
    });
    shared
        .chrome_control
        .owner
        .connected_profiles
        .observe(&json!({"profiles":[profile]}));
    shared.chrome_control.set_enabled(false).unwrap();
    let marker = shared
        .chrome_control
        .dir()
        .unwrap()
        .join("safari-enabled.json");
    std::fs::write(&marker, b"true").unwrap();
    let opened = gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"safari:p"}),
    )
    .await
    .unwrap();
    assert_eq!(
        opened["vwMcpResult"]["structuredContent"]["sessionId"], "t",
        "{opened}"
    );
    let read = json!({"sessionId":"t","action":"page.read","args":{}});
    gateway::call(shared, "s", "chrome_execute", read.clone())
        .await
        .unwrap();
    gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"tabs.create","args":{"url":"http://localhost:8123/"}}),
    )
    .await
    .unwrap();
    let refused = gateway::call(shared, "other", "chrome_execute", read.clone())
        .await
        .unwrap_err();
    assert!(refused.contains("another chat"), "{refused}");
    assert_eq!(
        agent_browser::selected(
            shared,
            "s",
            "chrome_session",
            &json!({"action":"open","profileId":"chrome:p"})
        )
        .id(),
        "chrome"
    );
    let effects = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "chrome_execute")
        .count();
    assert_eq!(effects, 2);
    std::fs::write(&marker, b"false").unwrap();
    let off = gateway::call(shared, "s", "chrome_execute", read)
        .await
        .unwrap();
    assert!(off.to_string().contains("off in Vibe Work"), "{off}");
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .count(),
        effects
    );
    core.shutdown().await;
}

fn auto_approve_chats(shared: &crate::core::Shared) {
    for chat in ["s", "other"] {
        assert!(
            shared
                .store
                .load(
                    json!({"session":{"id":chat},"history":[],"turns":[]}),
                    vw_session::LoadKind::New,
                )
                .is_ok()
        );
        shared.access.observe_request(
            "session/agent/update",
            chat,
            &json!({"agentName":"auto-approve"}),
        );
    }
}

#[tokio::test]
async fn explicit_safari_repair_restarts_a_failed_relay_but_leaves_a_healthy_owner_alone() {
    let core = crate::offline_core::start_with("safari-repair", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let dir = shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"applied":true}));
    safari::repair_connection(shared).await;
    assert!(
        shared.chrome_control.owner.is_running(),
        "healthy shared work stays connected"
    );
    safari::connection(shared, "unavailable");
    safari::repair_connection(shared).await;
    // This fixture deliberately has no keyring: a restart cannot succeed, but the broken owner must be gone.
    assert!(!shared.chrome_control.owner.is_running());
    assert_eq!(
        safari::view(shared).unwrap().addon.state,
        AddonState::HelperSetupRequired
    );
    core.shutdown().await;
}

#[tokio::test]
async fn safari_pairing_states_count_extension_connections_for_the_pairing_notice() {
    // `safari_package::open` sends the pairing notice to a just-launched Safari only after this count moves.
    let core = crate::offline_core::start_with("safari-seen", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let shared = &core.shared;
    let seen = || {
        shared
            .chrome_control
            .owner
            .safari_seen
            .load(std::sync::atomic::Ordering::SeqCst)
    };
    let pairing = |profile: &str, state: &str| {
        owner::observe(
            &std::sync::Arc::downgrade(shared),
            &json!({"method":"notifications/owner/pairing", "params":{
                "profileId":profile, "state":state, "fingerprint":"a".repeat(64)}}),
        )
        .unwrap();
    };
    pairing("chrome:p", "paired");
    assert_eq!(seen(), 0);
    pairing("safari:p", "paired");
    assert_eq!(seen(), 1);
    pairing("safari:p", "waiting");
    assert_eq!(seen(), 2);
    pairing("safari:p", "unknown");
    assert_eq!(seen(), 2);
    core.shutdown().await;
}
