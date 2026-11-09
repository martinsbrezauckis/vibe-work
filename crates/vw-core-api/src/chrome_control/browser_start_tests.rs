//! Closed-browser launch regressions through the gateway and approval hook. Fake launchers and reconnects
//! keep the desktop untouched; the helper integration suite separately checks window ownership.
use super::{AddonState, agent_browser::Browser, gateway, owner, test_support};
use crate::ask::tool_calls;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[allow(
    clippy::unwrap_used,
    reason = "the isolated test fixture must be created successfully"
)]
pub(super) async fn fixture(
    name: &str,
    browser: Browser,
    launcher: super::setup::Launcher,
) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        if browser == Browser::Edge {
            config.chrome_control.edge_launcher = Some(launcher);
        } else {
            config.chrome_control.launcher = Some(launcher);
        }
        config.agent_tools = Some(crate::AgentToolsConfig::new(
            dir.join("tools"),
            dir.join("data"),
            None,
        ));
    })
    .await;
    let shared = &core.shared;
    let chrome = &shared.chrome_control;
    test_support::answering(shared, |_, _| json!({"structuredContent":{"profiles":[]}}));
    chrome
        .owner
        .tools
        .lock()
        .unwrap()
        .push(json!({"name":"chrome_status"}));
    let data = chrome
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("browser-data");
    std::fs::create_dir_all(
        data.join("Default/Extensions")
            .join(&chrome.config.extension_id)
            .join("0.1.6_0"),
    )
    .unwrap();
    std::fs::write(chrome.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":[if browser == Browser::Edge {"Microsoft Edge"} else {"Google Chrome"}],"userData":[data]}).to_string()).unwrap();
    let profile = format!("{}:p", browser.id());
    for connected in [true, false] {
        test_support::observe_status(
            shared,
            &json!({"profiles":[{"profileId":profile,"connected":connected,"extensionVersion":"0.1.6"}]}),
        );
    }
    for chat in ["a", "b", "c"] {
        let _ = shared
            .store
            .load(
                json!({"session":{"id":chat},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            chat,
            &json!({"agentName":"auto-approve"}),
        );
    }
    assert_eq!(browser.addon(shared).state, AddonState::ChromeClosed);
    core
}

fn open(browser: Browser) -> Value {
    json!({"action":"open","profileId":format!("{}:p", browser.id())})
}

#[tokio::test]
async fn a_browser_name_resolves_only_after_reconnect_to_one_exact_profile() {
    let core = fixture(
        "browser-start-profile-name",
        Browser::Chrome,
        Arc::new(|_| Ok(())),
    )
    .await;
    let weak = Arc::downgrade(&core.shared);
    core.shared.chrome_control.owner.answering(move |tool, args| {
        let result = if tool == "chrome_status" {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true}]}})
        } else {
            assert_eq!(args["profileId"], "chrome:p", "the helper must receive an exact connected profile");
            json!({"structuredContent":{"sessionId":"task-a","profileId":"chrome:p","state":"active"}})
        };
        owner::update_pairing(&weak, &result); result
    });
    let reply = gateway::call(
        &core.shared,
        "a",
        "chrome_session",
        json!({"action":"open","profileId":"chrome"}),
    )
    .await
    .unwrap();
    assert_eq!(
        reply["vwMcpResult"]["structuredContent"]["sessionId"],
        "task-a"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn reconnect_timeout_never_dispatches_the_call_or_relaunches_on_retry() {
    let launches = Arc::new(Mutex::new(Vec::new()));
    let recorded = launches.clone();
    let core = fixture(
        "browser-start-timeout",
        Browser::Chrome,
        Arc::new(move |url| {
            recorded.lock().unwrap().push(url.to_owned());
            Ok(())
        }),
    )
    .await;
    let seen = core
        .shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"structuredContent":{"profiles":[]}}));
    let mut paused = false;
    for _ in 0..2 {
        let shared = core.shared.clone();
        let call = tokio::spawn(async move {
            gateway::call(&shared, "a", "chrome_session", open(Browser::Chrome)).await
        });
        if !paused {
            // The OS launch worker uses a real thread. Start the fake clock only once it has run,
            // so executor auto-advance cannot race a still-unscheduled spawn_blocking job.
            while launches.lock().unwrap().is_empty() {
                assert!(
                    !call.is_finished(),
                    "the call must wait for its launched browser"
                );
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            tokio::time::pause();
            paused = true;
        }
        let started = tokio::time::Instant::now();
        let reply = call.await.unwrap().unwrap();
        assert!(started.elapsed() <= std::time::Duration::from_millis(10_010));
        assert_eq!(
            reply["vwMcpResult"]["structuredContent"]["error"]["code"], "BROWSER_CONNECT_TIMEOUT",
            "{reply}"
        );
        assert!(
            reply
                .to_string()
                .contains("did not connect within 10 seconds")
                && reply.to_string().contains(&format!(
                    "check that {} is on",
                    vw_vocabulary::label("workNames", "chromeExtension")
                ))
        );
    }
    assert_eq!(launches.lock().unwrap().len(), 1);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool == "chrome_status")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn status_unpaired_switched_off_and_disallowed_modes_never_launch() {
    for case in ["status", "unpaired", "off", "plan", "stopped"] {
        let core = fixture(
            &format!("browser-start-refused-{case}"),
            Browser::Chrome,
            Arc::new(|_| panic!("this call must not launch a browser")),
        )
        .await;
        let shared = &core.shared;
        match case {
            "unpaired" => shared
                .chrome_control
                .paired_once
                .store(false, std::sync::atomic::Ordering::SeqCst),
            "off" => shared.chrome_control.set_enabled(false).unwrap(),
            "plan" => shared.access.observe_request(
                "session/agent/update",
                "a",
                &json!({"agentName":"vw-plan"}),
            ),
            "stopped" => {
                let _ = shared
                    .chrome_control
                    .person_stopped(&[super::person_stop::Stopped {
                        chat: "a".into(),
                        browser: Browser::Chrome,
                        turn: None,
                    }]);
            }
            _ => {}
        }
        let (tool, args) = if case == "status" {
            ("chrome_status", json!({}))
        } else {
            ("chrome_session", open(Browser::Chrome))
        };
        let reply = gateway::call(shared, "a", tool, args).await;
        assert!(
            reply.as_ref().is_err()
                || reply.as_ref().is_ok_and(|reply| {
                    reply["vwMcpResult"]["structuredContent"]["sessionId"].is_null()
                })
        );
        assert_eq!(tool_calls::open(shared).len(), 0);
        core.shutdown().await;
    }
}

#[tokio::test]
async fn stopping_while_the_extension_reconnects_prevents_session_open() {
    let core = fixture(
        "browser-start-stop-during-wait",
        Browser::Chrome,
        Arc::new(|_| Ok(())),
    )
    .await;
    let weak = Arc::downgrade(&core.shared);
    let seen = core.shared.chrome_control.owner.answering(move |_, _| {
        let shared = weak.upgrade().unwrap();
        let _ = shared
            .chrome_control
            .person_stopped(&[super::person_stop::Stopped {
                chat: "a".into(),
                browser: Browser::Chrome,
                turn: None,
            }]);
        json!({"structuredContent":{"profiles":[]}})
    });
    let reason = gateway::call(&core.shared, "a", "chrome_session", open(Browser::Chrome))
        .await
        .unwrap_err();
    assert!(reason.contains("the person pressed Stop"), "{reason}");
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool == "chrome_status")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn three_chats_share_one_launch_then_open_their_own_sessions() {
    for browser in [Browser::Chrome, Browser::Edge] {
        let launches = Arc::new(Mutex::new(Vec::new()));
        let recorded = launches.clone();
        let core = fixture(
            &format!("browser-start-three-{}", browser.id()),
            browser,
            Arc::new(move |url| {
                recorded.lock().unwrap().push(url.to_owned());
                Ok(())
            }),
        )
        .await;
        let weak = Arc::downgrade(&core.shared);
        let recorded = launches.clone();
        let seen = core.shared.chrome_control.owner.answering(move |tool, args| {
            let result = if tool == "chrome_status" {
                json!({"structuredContent":{"profiles":[{"profileId":format!("{}:p", browser.id()),"connected":!recorded.lock().unwrap().is_empty()}]}})
            } else {
                assert_eq!(tool, "chrome_session");
                assert!(args.get("tabId").is_none(), "never borrow a person's tab");
                json!({"structuredContent":{"sessionId":format!("task-{}", args["_clientSessionId"].as_str().unwrap()),"profileId":args["profileId"],"state":"active"}})
            };
            owner::update_pairing(&weak, &result);
            result
        });
        let (a, b, c) = tokio::join!(
            gateway::call(&core.shared, "a", "chrome_session", open(browser)),
            gateway::call(&core.shared, "b", "chrome_session", open(browser)),
            gateway::call(&core.shared, "c", "chrome_session", open(browser)),
        );
        for (chat, reply) in [("a", a), ("b", b), ("c", c)] {
            let reply = reply.unwrap();
            assert_eq!(
                reply["vwMcpResult"]["structuredContent"]["sessionId"],
                format!("task-{chat}"),
                "{reply}"
            );
        }
        assert_eq!(
            *launches.lock().unwrap(),
            [String::new()],
            "launch with no URL; helper owns chat windows"
        );
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(tool, _)| tool == "chrome_session")
                .count(),
            3
        );
        let denied = gateway::call(
            &core.shared,
            "a",
            "chrome_execute",
            json!({"sessionId":"task-b","action":"page.read","args":{}}),
        )
        .await
        .unwrap_err();
        assert!(denied.contains("another chat"), "{denied}");
        core.shutdown().await;
    }
}

#[tokio::test]
async fn launch_failure_keeps_the_reason_and_recovery_instruction() {
    let core = fixture(
        "browser-start-failed",
        Browser::Edge,
        Arc::new(|_| Err("Microsoft Edge could not be opened".into())),
    )
    .await;
    let reply = gateway::call(&core.shared, "a", "chrome_session", open(Browser::Edge))
        .await
        .unwrap();
    assert_eq!(
        reply["vwMcpResult"]["structuredContent"]["error"]["code"], "BROWSER_LAUNCH_FAILED",
        "{reply}"
    );
    let text = reply.to_string();
    assert!(
        text.contains("could not be opened")
            && text.contains("Open Microsoft Edge")
            && text.contains("try again"),
        "{text}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn ask_open_launches_a_paired_browser_without_a_card() {
    let launches = Arc::new(Mutex::new(Vec::new()));
    let recorded = launches.clone();
    let core = fixture(
        "browser-start-ask-open",
        Browser::Chrome,
        Arc::new(move |url| {
            recorded.lock().unwrap().push(url.to_owned());
            Ok(())
        }),
    )
    .await;
    core.shared
        .access
        .observe_request("session/agent/update", "a", &json!({"agentName":"ask"}));
    let weak = Arc::downgrade(&core.shared);
    let recorded = launches.clone();
    core.shared.chrome_control.owner.answering(move |tool,_| {
        let result=if tool=="chrome_status" {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":!recorded.lock().unwrap().is_empty()}]}})
        } else { json!({"structuredContent":{"sessionId":"task-a","profileId":"chrome:p","state":"active"}}) };
        owner::update_pairing(&weak,&result); result
    });
    let preparation = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        crate::agent_tools::prepare(
            &core.shared,
            "a",
            "a",
            "open-call",
            "chrome_session",
            &mut open(Browser::Chrome),
        ),
    )
    .await;
    assert!(
        matches!(preparation, Ok(Ok(None))),
        "opening browser work must prepare without a card"
    );
    assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
    let reply = gateway::call(&core.shared, "a", "chrome_session", open(Browser::Chrome))
        .await
        .unwrap();
    assert_eq!(
        reply["vwMcpResult"]["structuredContent"]["sessionId"],
        "task-a"
    );
    assert_eq!(launches.lock().unwrap().len(), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn paired_profile_directory_requires_one_installed_profile_in_the_same_browser() {
    let core = fixture(
        "browser-launch-paired-profile",
        Browser::Chrome,
        Arc::new(|_| Ok(())),
    )
    .await;
    let shared = &core.shared;
    assert_eq!(
        super::installation::paired_directory(shared, Browser::Chrome).as_deref(),
        Some("Default")
    );
    assert!(super::installation::paired_directory(shared, Browser::Edge).is_none());
    let data = shared
        .chrome_control
        .config
        .user_home
        .as_ref()
        .unwrap()
        .join("browser-data");
    std::fs::create_dir_all(
        data.join("Profile 2/Extensions")
            .join(&shared.chrome_control.config.extension_id),
    )
    .unwrap();
    // Windows external installation seeds each newly loaded profile, disabled until consent.
    let pending = data.join("Profile 2/Preferences");
    std::fs::write(
        &pending,
        json!({"extensions":{"settings":{
            &shared.chrome_control.config.extension_id:{"disable_reasons":8192}
        }}})
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        super::installation::paired_directory(shared, Browser::Chrome).as_deref(),
        Some("Default"),
        "an unpaired disabled copy does not add a launch choice"
    );
    assert_eq!(
        Browser::Chrome.addon(shared).state,
        AddonState::ChromeClosed
    );
    std::fs::remove_file(pending).unwrap();
    assert!(
        super::installation::paired_directory(shared, Browser::Chrome).is_none(),
        "ambiguous profiles keep the normal browser launch"
    );
    std::fs::remove_dir_all(data.join("Default")).unwrap();
    assert_eq!(
        super::installation::paired_directory(shared, Browser::Chrome).as_deref(),
        Some("Profile 2"),
        "a deleted pairing folder cannot strand the sole existing extension profile on the picker"
    );
}
