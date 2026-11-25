use super::*;
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "Requires a real helper, Firefox and its own X display"]
#[allow(
    clippy::too_many_lines,
    reason = "keep the isolated three-mode native lifecycle scenario linear"
)]
async fn native_session_opens_firefox_without_a_settings_click() {
    use std::io::{Read, Write};
    let helper = std::path::PathBuf::from(std::env::var_os("VW_FFXOPEN_HELPER").unwrap());
    let root =
        std::path::PathBuf::from(std::env::var_os("VW_TEST_ROOT").unwrap()).join("native-keys");
    let keys = super::super::owner_credential_tests::memory_keys(&root).await;
    let core = crate::offline_core::start_with("firefox-native-auto-open", |config, dir| {
        config.keys = Some(keys.clone());
        config.agent_tools = Some(crate::AgentToolsConfig::new(
            dir.join("tools"),
            dir.join("data"),
            None,
        ));
        let package = dir.join("package");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::copy(&helper, package.join("bin").join(setup::helper_name())).unwrap();
        config.chrome_control.package = Some(package);
    })
    .await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    let server = std::thread::spawn(move || {
        while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
            if let Ok((mut stream, _)) = listener.accept() {
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let _ = stream.read(&mut [0; 4096]);
                let body = "<!doctype html><title>native-auto-open</title><h1>native-auto-open</h1><button>Test button</button>";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            } else {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    });
    super::super::test_support::chat(&core.shared, "native");
    core.shared
        .chrome_control
        .set_browser_enabled(FIREFOX, false)
        .unwrap();
    let off = super::super::gateway::call(
        &core.shared,
        "native",
        "chrome_session",
        json!({"action":"open","profileId":"firefox:agent"}),
    )
    .await
    .unwrap();
    assert!(off.to_string().contains("turn it on in Settings"), "{off}");
    println!("NATIVE off: precise Settings refusal; switch unchanged");
    core.shared
        .chrome_control
        .set_browser_enabled(FIREFOX, true)
        .unwrap();
    for mode in ["ask", "accept-edits", "auto-approve"] {
        let _ = core
            .shared
            .store
            .load(
                json!({
                    "session":{"id":"native","status":{"type":"running","activeTurnId":mode}},
                    "history":[],"turns":[{"id":mode,"status":"in_progress"}]
                }),
                vw_session::LoadKind::New,
            )
            .unwrap();
        core.shared.access.observe_request(
            "session/agent/update",
            "native",
            &json!({"agentName":mode}),
        );
        let args = json!({"action":"open","profileId":"firefox:agent"});
        let prepared = crate::agent_tools::prepare(
            &core.shared,
            "native",
            "native",
            "native-open",
            "chrome_session",
            &mut args.clone(),
        )
        .await
        .unwrap();
        assert!(
            prepared.is_none(),
            "session startup follows Chrome/Edge without a card"
        );
        assert_eq!(core.shared.asks.list().len(), 0);
        let opened = super::super::gateway::call(&core.shared, "native", "chrome_session", args)
            .await
            .unwrap();
        let body = &opened["vwMcpResult"]["structuredContent"];
        assert_eq!(body["state"], "active", "{opened}");
        let session = body["sessionId"].as_str().unwrap();
        println!("NATIVE {mode}: closed -> active session; no Settings click or startup card");
        for (action, args) in [
            ("tabs.create", json!({"url":url})),
            ("page.read", json!({})),
        ] {
            let answer = super::super::gateway::call(
                &core.shared,
                "native",
                "chrome_execute",
                json!({"sessionId":session,"action":action,"args":args}),
            )
            .await
            .unwrap();
            assert!(
                answer["vwMcpResult"]["structuredContent"]["error"].is_null(),
                "{answer}"
            );
            if action == "page.read" {
                assert!(answer.to_string().contains("native-auto-open"), "{answer}");
            }
            println!("NATIVE {mode}: {action} passed");
        }
        if let Some(folder) = std::env::var_os("VW_FFXOPEN_SCREENSHOTS") {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let image = std::path::PathBuf::from(folder).join(format!("{mode}-window.png"));
            assert!(
                std::process::Command::new("import")
                    .args(["-window", "root"])
                    .arg(image)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let other = if mode == "ask" {
            super::super::test_support::chat(&core.shared, "other");
            let answer = super::super::gateway::call(
                &core.shared,
                "other",
                "chrome_session",
                json!({"action":"open","profileId":"firefox:agent"}),
            )
            .await
            .unwrap();
            Some(
                answer["vwMcpResult"]["structuredContent"]["sessionId"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        } else {
            None
        };
        if mode == "auto-approve" {
            super::super::end_chat(&core.shared, "native", true);
        } else {
            let ended = super::super::gateway::call(
                &core.shared,
                "native",
                "chrome_session",
                json!({"action":"stop","sessionId":session}),
            )
            .await
            .unwrap();
            assert!(
                ended["vwMcpResult"]["structuredContent"]["error"].is_null(),
                "{ended}"
            );
        }
        if let Some(other) = other {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let status = core
                .shared
                .chrome_control
                .owner
                .call("chrome_status", json!({}))
                .await;
            assert_eq!(
                status.value()["structuredContent"]["firefoxWindow"],
                "Window open",
                "another chat still needs the process"
            );
            let ended = super::super::gateway::call(
                &core.shared,
                "other",
                "chrome_session",
                json!({"action":"stop","sessionId":other}),
            )
            .await
            .unwrap();
            assert!(
                ended["vwMcpResult"]["structuredContent"]["error"].is_null(),
                "{ended}"
            );
            println!("NATIVE ask: second chat kept Firefox alive until its own stop");
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let answer = core
                .shared
                .chrome_control
                .owner
                .call("chrome_status", json!({}))
                .await;
            if answer.value()["structuredContent"]["firefoxWindow"] == "Window closed" {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "window did not close: {:?}",
                answer.value()
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        println!(
            "NATIVE {mode}: {} -> owned process/window closed",
            if mode == "auto-approve" {
                "chat deleted"
            } else {
                "session stop"
            }
        );
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    server.join().unwrap();
    core.shutdown().await;
}

async fn window_core(name: &str) -> crate::Core {
    let root = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(
            || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp"),
            std::path::PathBuf::from,
        )
        .join(format!("{name}-keys-{}", std::process::id()));
    let keys = super::super::owner_credential_tests::memory_keys(&root).await;
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.keys = Some(keys);
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    super::super::test_support::chat(&core.shared, "s");
    core
}

/// The fake owner refuses a session open until the owned child receives an explicit window-open.
async fn immediate_reopen_fixture(
    name: &str,
) -> (
    crate::Core,
    std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use vw_platform::sync::LockRecover;
    let core = window_core(name).await;
    let chrome = &core.shared.chrome_control;
    assert!(chrome.set_browser_enabled(FIREFOX, true).is_ok());
    *chrome.owner.firefox_pairing.lock_or_recover() = (PairingState::WindowOpen, None);
    assert!(
        super::super::profile_bindings::remember(chrome, "s", "firefox:agent", Some("old")).is_ok()
    );
    chrome
        .tasks
        .lock_or_recover()
        .insert("old".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old", "s", "active", "firefox:agent", None);
    let window_open = Arc::new(AtomicBool::new(true));
    let seen = super::super::test_support::answering(&core.shared, move |tool, args| {
        if args["_firefoxWindow"] == "close" {
            window_open.store(false, Ordering::SeqCst);
            return json!({"structuredContent":{"firefoxWindow":"Window closed"}});
        }
        if args["_firefoxWindow"] == "open" {
            window_open.store(true, Ordering::SeqCst);
            return json!({"structuredContent":{"firefoxWindow":"Window open"}});
        }
        if tool == "chrome_status" {
            let state = if window_open.load(Ordering::SeqCst) {
                "Window open"
            } else {
                "Window closed"
            };
            return json!({"structuredContent":{"firefoxWindow":state}});
        }
        if tool == "chrome_session" && args["action"] == "stop" {
            window_open.store(false, Ordering::SeqCst);
            return json!({"structuredContent":{"status":"completed","state":"stopped","sessionId":"old"}});
        }
        if tool == "chrome_session" && args["action"] == "open" {
            return if window_open.load(Ordering::SeqCst) {
                json!({"structuredContent":{"sessionId":"fresh","profileId":"firefox:agent","state":"active"}})
            } else {
                json!({"isError":true,"structuredContent":{"error":{"code":"BROWSER_UNAVAILABLE","message":"Firefox is unavailable; fixture window closed."}}})
            };
        }
        json!({"structuredContent":{"status":"completed"}})
    });
    (core, seen)
}

#[tokio::test]
async fn confirmed_stop_immediately_reopens_owned_firefox_without_a_poll() {
    use vw_platform::sync::LockRecover;
    let (core, seen) = immediate_reopen_fixture("firefox-immediate-reopen-after-stop").await;
    let stopped = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"stop","sessionId":"old"}),
    )
    .await;
    assert!(stopped.is_ok(), "{stopped:?}");
    let opened = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"firefox:agent"}),
    )
    .await;
    assert_eq!(
        opened
            .as_ref()
            .ok()
            .map(|v| &v["vwMcpResult"]["structuredContent"]["state"]),
        Some(&json!("active")),
        "{opened:?}"
    );
    assert!(
        seen.lock_or_recover()
            .iter()
            .any(|(_, args)| args["_firefoxWindow"] == "open"),
        "{seen:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn close_browser_immediately_reopens_owned_firefox() {
    use vw_platform::sync::LockRecover;
    let (core, seen) = immediate_reopen_fixture("firefox-immediate-reopen-after-close").await;
    let closed = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"firefox:agent"}),
    )
    .await;
    assert!(closed.is_ok(), "{closed:?}");
    let opened = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"firefox:agent"}),
    )
    .await;
    assert_eq!(
        opened
            .as_ref()
            .ok()
            .map(|v| &v["vwMcpResult"]["structuredContent"]["state"]),
        Some(&json!("active")),
        "{opened:?}"
    );
    assert!(
        seen.lock_or_recover()
            .iter()
            .any(|(_, args)| args["_firefoxWindow"] == "open"),
        "{seen:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_existing_helper_does_not_bypass_owner_key_setup() {
    let core = super::super::output_tests::core("firefox-auto-no-keys").await;
    let seen = super::super::test_support::answering(&core.shared, |_, _| {
        super::super::output_tests::mcp(json!({"firefoxWindow":"Window open"}))
    });
    let error = window(&core.shared, true).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains(vw_vocabulary::texts::CHROME_NEEDS_KEYRING),
        "{error}"
    );
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["_firefoxWindow"] == "open")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn session_open_starts_a_closed_firefox_and_keeps_start_failures() {
    for mode in ["ask", "accept-edits", "auto-approve"] {
        for state in [PairingState::WindowClosed, PairingState::BrowserOutdated] {
            for failure in [
                None,
                Some((
                    "BROWSER_NOT_INSTALLED",
                    "Firefox is not installed. Install Firefox.",
                )),
                Some((
                    "BROWSER_OUTDATED",
                    "Update Firefox to version 159 or later.",
                )),
                Some((
                    "BROWSER_START_FAILED",
                    "Firefox could not start: fixture failure.",
                )),
            ] {
                let label = format!(
                    "fx-auto-{mode}-{}-{}",
                    state == PairingState::BrowserOutdated,
                    failure.map_or_else(|| "ok".into(), |(code, _)| code.to_ascii_lowercase())
                );
                let core = window_core(&label).await;
                core.shared.access.observe_request(
                    "session/agent/update",
                    "s",
                    &json!({"agentName":mode}),
                );
                *core
                    .shared
                    .chrome_control
                    .owner
                    .firefox_pairing
                    .lock()
                    .unwrap() = (state, None);
                let seen = super::super::test_support::answering(
                    &core.shared,
                    move |tool, args| {
                        if args["_firefoxWindow"] == "open" {
                            return super::super::output_tests::mcp(match failure {
                                Some((fault_code, message)) => {
                                    json!({"error":{"code":fault_code,"message":message}})
                                }
                                None => json!({"firefoxWindow":"Window open"}),
                            });
                        }
                        if tool == "chrome_session" {
                            super::super::output_tests::mcp(
                                json!({"sessionId":"fx","profileId":"firefox:agent","state":"active"}),
                            )
                        } else {
                            super::super::output_tests::mcp(json!({}))
                        }
                    },
                );
                let result = super::super::gateway::call(
                    &core.shared,
                    "s",
                    "chrome_session",
                    json!({"action":"open","profileId":"firefox:agent"}),
                )
                .await;
                let text = format!("{result:?}");
                assert!(
                    seen.lock()
                        .unwrap()
                        .iter()
                        .any(|(_, args)| args["_firefoxWindow"] == "open"),
                    "{text}"
                );
                if let Some((_, message)) = failure {
                    assert!(text.contains(message), "{text}");
                    assert!(
                        !seen
                            .lock()
                            .unwrap()
                            .iter()
                            .any(|(tool, _)| tool == "chrome_session")
                    );
                } else {
                    assert!(text.contains("active"), "{text}");
                    assert!(!text.contains("OWNER_REQUIRED"), "{text}");
                }
                assert!(
                    core.shared.asks.list().is_empty(),
                    "startup follows Chrome/Edge in {mode}"
                );
                core.shutdown().await;
            }
        }
    }
}

#[tokio::test]
async fn another_browser_lease_refuses_before_firefox_can_start() {
    let core = super::super::output_tests::core("firefox-auto-lease").await;
    super::super::profile_bindings::remember(
        &core.shared.chrome_control,
        "s",
        "chrome:existing",
        Some("chrome-lease"),
    )
    .unwrap();
    let seen = super::super::test_support::answering(&core.shared, |_, _| json!({}));
    let error = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"firefox:agent"}),
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("Close that browser session first"),
        "{error}"
    );
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["_firefoxWindow"] == "open")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn closed_firefox_status_is_ready_not_owner_required() {
    let core = super::super::output_tests::core("firefox-ready-status").await;
    *core
        .shared
        .chrome_control
        .owner
        .firefox_pairing
        .lock()
        .unwrap() = (PairingState::WindowClosed, None);
    super::super::test_support::answering(&core.shared, |_, _| {
        super::super::output_tests::mcp(json!({"firefoxWindow":"Window closed"}))
    });
    let answer = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_status",
        json!({"profileId":"firefox:agent"}),
    )
    .await
    .unwrap();
    assert!(!answer.to_string().contains("OWNER_REQUIRED"), "{answer}");
    assert!(answer.to_string().contains("automatically"), "{answer}");
    core.shared.chrome_control.owner.stop().await;
    *core
        .shared
        .chrome_control
        .owner
        .firefox_pairing
        .lock()
        .unwrap() = (PairingState::WindowClosed, None);
    let answer = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_status",
        json!({"profileId":"firefox:agent"}),
    )
    .await
    .unwrap();
    assert!(!answer.to_string().contains("OWNER_REQUIRED"), "{answer}");
    assert!(answer.to_string().contains("automatically"), "{answer}");
    assert!(
        !core.shared.chrome_control.owner.is_running(),
        "status alone must not start Firefox or its helper"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn session_open_never_enables_firefox() {
    let core = super::super::output_tests::core("firefox-auto-off").await;
    core.shared
        .chrome_control
        .set_browser_enabled(FIREFOX, false)
        .unwrap();
    let seen = super::super::test_support::answering(&core.shared, |_, _| json!({}));
    let answer = super::super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"firefox:agent"}),
    )
    .await
    .unwrap();
    assert!(
        answer.to_string().contains("turn it on in Settings"),
        "{answer}"
    );
    assert!(!core.shared.chrome_control.firefox_enabled());
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["_firefoxWindow"] == "open")
    );
    core.shutdown().await;
}

#[test]
fn a_closed_window_is_ready_for_automatic_open_not_a_person_step() {
    let view = state_view(PairingState::WindowClosed, true, true);
    assert_eq!(view.label, "Ready");
    assert!(view.instruction.contains("automatically"));
    assert!(view.instruction.contains("session has ended"));
    assert!(!view.instruction.contains("Open window"));
}

#[test]
fn unavailable_package_precedes_switched_off_when_window_is_not_open() {
    assert_eq!(
        state_view(PairingState::WindowClosed, false, false).state,
        super::super::AddonState::Unavailable
    );
    assert_eq!(
        state_view(PairingState::NotInstalled, false, false).state,
        super::super::AddonState::Unavailable
    );
    assert_eq!(
        state_view(PairingState::WindowClosed, false, true).state,
        super::super::AddonState::SwitchedOff
    );
    assert_eq!(
        state_view(PairingState::WindowOpen, false, false).state,
        super::super::AddonState::SwitchedOff
    );
}

#[tokio::test]
async fn firefox_facts_use_its_window_controls_and_connection_state() {
    let core = crate::offline_core::start_with("firefox-window-facts", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    super::super::test_support::answering(&core.shared, |_, _| json!({"structuredContent":{}}));
    for (state, label, instruction) in [
        (PairingState::WindowClosed, "Ready", "automatically"),
        (PairingState::WindowOpen, "Window open", "chrome_session"),
        (PairingState::Connecting, "Starting", "Wait"),
        (
            PairingState::BrowserOutdated,
            "Update Firefox",
            "open a new browser session",
        ),
        (
            PairingState::NotInstalled,
            "Not installed",
            "Ask the user to install Firefox",
        ),
    ] {
        *core
            .shared
            .chrome_control
            .owner
            .firefox_pairing
            .lock()
            .unwrap() = (state, None);
        let facts = super::super::installation::addons(&core.shared);
        let firefox = facts
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == "firefox")
            .expect("Firefox facts are required when its module is enabled");
        assert_eq!(firefox["label"], label);
        assert!(
            firefox["instruction"]
                .as_str()
                .unwrap()
                .contains(instruction),
            "{firefox}"
        );
        assert!(
            !firefox["instruction"]
                .as_str()
                .unwrap()
                .contains("Connect Firefox")
        );
        if state == PairingState::NotInstalled {
            assert_eq!(
                firefox["instruction"],
                "Firefox is missing. Ask the user to install Firefox, then open a new browser session with profileId firefox:agent."
            );
        }
    }
    core.shutdown().await;
}
