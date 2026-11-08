//! Browser routing regressions through the agent gateway and an owned fake helper; no browser or pairing UI.
use super::{gateway, owner, test_support};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_vocabulary::texts::{CHROME_NAME, EDGE_NAME};

fn body(result: &Value) -> &Value {
    let body = &result["vwMcpResult"]["structuredContent"];
    body.get("untrustedPageContent").unwrap_or(body)
}

const CHROME: &str = "12345678-1234-1234-1234-123456789abc";

#[tokio::test]
async fn a_missing_session_does_not_borrow_the_other_connected_browsers_name() {
    let core = fixture(
        "missing-browser-session",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    let error = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"previous-chrome-session","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error,
        "Not run: no browser session with this id; open one with chrome_session."
    );
    core.shutdown().await;
    let core = fixture(
        "missing-known-browser-session",
        json!([
            {"profileId":"edge:p","connected":true}, {"profileId":"chrome:p","connected":true}
        ]),
    )
    .await;
    core.shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .insert("s".into(), "edge:p".into());
    let error = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"previous-chrome-session","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error,
        "Not run: no browser session with this id; open one with chrome_session."
    );
    core.shared.chrome_control.work.lock().unwrap().open(
        "previous-chrome-session",
        "s",
        "stopped",
        "chrome:p",
        None,
    );
    let error = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"previous-chrome-session","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error,
        "Not run: this Chrome session was stopped; open a new one with chrome_session."
    );
    core.shutdown().await;
}

#[tokio::test]
async fn explicit_edge_requests_report_edges_connection_problem() {
    let core = fixture("edge-request-diagnosis", json!([])).await;
    core.shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap()
        .0 = crate::chrome_control::PairingState::OtherOwner;
    for profile in ["edge", EDGE_NAME, "edge:p"] {
        for (tool, args) in [
            ("chrome_status", json!({"profileId":profile})),
            (
                "chrome_session",
                json!({"action":"open","profileId":profile}),
            ),
        ] {
            let result = gateway::call(&core.shared, "s", tool, args).await.unwrap();
            let text = result["vwMcpResult"]["content"][0]["text"]
                .as_str()
                .unwrap();
            assert!(
                text.contains("another Vibe Work") && text.contains("Connect Edge"),
                "{profile} {tool}: {text}"
            );
            assert!(!text.contains("Connect Chrome"), "{text}");
        }
    }
    core.shutdown().await;
}

pub(super) async fn fixture(name: &str, profiles: Value) -> crate::Core {
    fixture_with(name, profiles, |_| {}).await
}

pub(super) async fn fixture_with(
    name: &str,
    profiles: Value,
    configure: impl FnOnce(&mut super::setup::ChromeControlConfig),
) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        let root = dir.join("install");
        vw_vibe_install::extension_store::save_record(
            &root,
            Some(vw_vibe_install::support::ChromeExtension {
                store: Some(vw_vibe_install::ExtensionStore::Available),
                min_version: Some("0.1.3".into()),
            }),
        )
        .unwrap();
        config.vibe_install = Some(vw_vibe_install::VibeInstaller::new(
            vw_vibe_install::InstallerConfig {
                root,
                sources: vw_vibe_install::Sources::pinned(&vw_vibe_install::this_target()),
                parent_env: Vec::new(),
                extra_env: Vec::new(),
                discovery: vw_supervisor::Discovery {
                    explicit: None,
                    path_var: None,
                    home: None,
                },
                limits: vw_vibe_install::Limits::default(),
            },
        ));
        configure(&mut config.chrome_control);
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
    let weak = Arc::downgrade(shared);
    let status = json!({"structuredContent":{"profiles":profiles,"sessions":[]}});
    let reply = status.clone();
    test_support::answering(shared, move |tool, args| {
        let answer = match tool {
            "chrome_status" => reply.clone(),
            "chrome_session" => {
                json!({"structuredContent":{"sessionId":"task","profileId":args["profileId"],"state":"active"}})
            }
            _ => {
                assert_eq!(args["sessionId"], "task");
                json!({"structuredContent":{"status":"completed","data":{"text":"Edge page"}}})
            }
        };
        owner::update_pairing(&weak, &answer);
        answer
    });
    // Model the helper's completed tools handshake, as Chrome's shared state table requires.
    shared
        .chrome_control
        .owner
        .tools
        .lock()
        .unwrap()
        .push(json!({"name":"chrome_status"}));
    std::fs::write(
        shared.chrome_control.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome","Microsoft Edge"],"userData":[]}).to_string(),
    )
    .unwrap();
    owner::update_pairing(&Arc::downgrade(shared), &status);
    core
}

#[tokio::test]
async fn edge_only_status_is_connected_and_names_edge() {
    let core = fixture(
        "edge-agent-status",
        json!([{"profileId":"edge:p","connected":true,"extensionVersion":"0.1.3"}]),
    )
    .await;
    let result = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    let body = body(&result);
    assert_eq!(body["addon"]["stateCode"], "connected", "{result}");
    assert_eq!(
        body["addon"]["instruction"],
        format!("{EDGE_NAME} is connected.")
    );
    assert_eq!(body["profiles"][0]["profileId"], "edge:p");
    core.shutdown().await;
}

#[tokio::test]
async fn edge_only_session_opens_and_executes_on_the_requested_profile() {
    let core = fixture(
        "edge-agent-route",
        json!([{"profileId":"edge:p","connected":true}]),
    )
    .await;
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["profileId"], "edge:p",
        "{result}"
    );
    let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
    assert_eq!(view["work"][0]["browser"], "Edge");
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"task","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    assert!(result.to_string().contains("Edge page"), "{result}");
    core.shutdown().await;
}

#[tokio::test]
async fn both_browsers_keep_explicit_selection_and_bare_chrome_ids() {
    let core = fixture(
        "edge-agent-selection",
        json!([{"profileId":CHROME,"connected":true},{"profileId":"edge:p","connected":true}]),
    )
    .await;
    for (profile, instruction) in [
        (CHROME, format!("{CHROME_NAME} is connected.")),
        ("edge:p", format!("{EDGE_NAME} is connected.")),
    ] {
        let opened = gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
        assert_eq!(
            opened["vwMcpResult"]["structuredContent"]["profileId"],
            profile
        );
        let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
        assert_eq!(
            view["work"][0]["browser"],
            if profile == CHROME { "Chrome" } else { "Edge" }
        );
        let status = gateway::call(&core.shared, "s", "chrome_status", json!({}))
            .await
            .unwrap();
        assert_eq!(
            body(&status)["addon"]["instruction"],
            instruction,
            "{status}"
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn selected_disconnected_edge_keeps_its_reason_and_offer_while_chrome_is_connected() {
    let core = fixture(
        "edge-agent-refusal",
        json!([{"profileId":CHROME,"connected":true},{"profileId":"edge:p","connected":false}]),
    )
    .await;
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    let text = result.to_string();
    assert!(
        text.contains("not in Edge yet")
            && text.contains("Ask the person")
            && text.contains("addon: edge"),
        "{text}"
    );
    assert!(!text.contains("Connect Chrome"), "{text}");
    core.shutdown().await;
}

#[tokio::test]
async fn chrome_only_status_keeps_the_unscoped_profile() {
    let core = fixture(
        "edge-agent-chrome-only",
        json!([{"profileId":CHROME,"connected":true}]),
    )
    .await;
    let result = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        body(&result)["addon"]["instruction"],
        format!("{CHROME_NAME} is connected.")
    );
    assert_eq!(body(&result)["profiles"][0]["profileId"], CHROME);
    core.shutdown().await;
}

#[tokio::test]
async fn owner_retains_edge_versions_without_erasing_them_on_an_action_reply() {
    let core = fixture("edge-agent-versions", json!([{"profileId":CHROME,"connected":true,"extensionVersion":"0.1.4"},{"profileId":"edge:p","connected":true,"extensionVersion":"0.1.3"}])).await;
    let profiles = core
        .shared
        .chrome_control
        .owner
        .connected_profiles
        .snapshot();
    assert!(
        profiles.contains(&("edge:p".into(), Some("0.1.3".into()))),
        "{profiles:?}"
    );
    owner::update_pairing(
        &Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"status":"failed","error":{"code":"TARGET_CLOSED"}}}),
    );
    assert_eq!(
        core.shared
            .chrome_control
            .owner
            .connected_profiles
            .snapshot(),
        profiles
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_old_edge_extension_blocks_only_edge_and_keeps_the_update_instruction() {
    let core = fixture("edge-agent-old-version", json!([{"profileId":CHROME,"connected":true,"extensionVersion":"0.1.3"},{"profileId":"edge:p","connected":true,"extensionVersion":"0.1.2"}])).await;
    let chrome = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":CHROME}),
    )
    .await
    .unwrap();
    assert_eq!(body(&chrome)["profileId"], CHROME);
    let edge = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:p"}),
    )
    .await
    .unwrap();
    assert_eq!(body(&edge)["addon"]["stateCode"], "updateNeeded", "{edge}");
    assert!(
        body(&edge)["addon"]["instruction"]
            .as_str()
            .unwrap()
            .contains("Edge")
    );
    core.shutdown().await;
}
