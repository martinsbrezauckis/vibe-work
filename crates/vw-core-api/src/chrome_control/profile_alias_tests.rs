//! Legacy profile ids through the agent and Debug API; scratch upgrade data and an owned helper double.
use super::{gateway, owner, test_support};
use serde_json::json;
use std::sync::Arc;

const BARE: &str = "12345678-1234-1234-1234-123456789abc";
const SCOPED: &str = "chrome:12345678-1234-1234-1234-123456789abc";

#[allow(clippy::unwrap_used, reason = "isolated test fixture initialization")]
async fn upgraded(name: &str, scoped: bool) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        let home = dir.join("legacy-home");
        let state = home.join("VibeWork/ChromeControl");
        std::fs::create_dir_all(&state).unwrap();
        for (file, value) in [
            ("paired.json", json!(true)),
            ("enabled.json", json!(true)),
            ("permissions.json", json!({"level":"act"})),
            (
                "setup.json",
                json!({"files":[],"browsers":["Google Chrome"],"userData":[]}),
            ),
        ] {
            std::fs::write(state.join(file), value.to_string()).unwrap();
        }
        config.chrome_control.user_home = Some(home);
    })
    .await;
    assert!(
        core.shared
            .chrome_control
            .paired_once
            .load(std::sync::atomic::Ordering::SeqCst),
        "the old pairing-history file is read before any helper status"
    );
    let shared = &core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"saved-chat"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared.access.observe_request(
        "session/agent/update",
        "saved-chat",
        &json!({"agentName":"auto-approve"}),
    );
    let profile = if scoped { SCOPED } else { BARE };
    let status = json!({"structuredContent":{"profiles":[{"profileId":profile,"connected":true}],"sessions":[]}});
    let reply = status.clone();
    test_support::answering(shared, move |tool, args| {
        let requested = if tool == "chrome_status" {
            &args["_tabContext"]["profileId"]
        } else {
            &args["profileId"]
        };
        if requested.is_null() {
            return reply.clone();
        }
        if requested != profile {
            return owner::failure("No Chrome profile has this ID", "PROFILE_UNKNOWN");
        }
        if tool == "chrome_session" {
            json!({"structuredContent":{"sessionId":"task","profileId":profile,"state":"active"}})
        } else {
            json!({"structuredContent":{"status":"completed","requestId":"context","data":{"title":"Saved tab","text":"Context","url":"https://8.8.8.8/"}}})
        }
    });
    owner::update_pairing(&Arc::downgrade(shared), &status);
    core
}

#[tokio::test]
async fn a_saved_bare_profile_call_opens_the_scoped_chrome_profile_after_upgrade() {
    let core = upgraded("profile-alias-upgrade", true).await;
    // This is the old call shape retained in a saved chat, not the extension's new scoped inventory.
    let saved_call = json!({"action":"open","profileId":BARE});
    let result = gateway::call(&core.shared, "saved-chat", "chrome_session", saved_call)
        .await
        .unwrap();
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["profileId"], SCOPED,
        "{result}"
    );
    assert!(super::store::paired_once(
        core.shared.chrome_control.dir().unwrap()
    ));
    core.shutdown().await;
}

#[tokio::test]
async fn debug_tab_context_accepts_a_bare_profile_with_a_scoped_helper() {
    let core = upgraded("profile-alias-debug", true).await;
    let command = serde_json::from_value(
        json!({"type":"chromeControl","request":{"op":"tabContext","profileId":BARE,"tabId":7}}),
    )
    .unwrap();
    let result = core.execute(crate::Origin::DebugApi, command).await;
    assert!(
        matches!(
            result,
            Ok(crate::CommandResult::ChromeSelection { selection: Some(_) })
        ),
        "{result:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_chrome_only_legacy_helper_keeps_its_bare_wire_id() {
    let core = upgraded("profile-alias-legacy-helper", false).await;
    let result = gateway::call(
        &core.shared,
        "saved-chat",
        "chrome_session",
        json!({"action":"open","profileId":BARE}),
    )
    .await
    .unwrap();
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["profileId"], BARE,
        "{result}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_bare_alias_never_selects_an_edge_profile_with_the_same_uuid() {
    let core = upgraded("profile-alias-no-edge", true).await;
    let seen = test_support::answering(&core.shared, |_, _| {
        owner::failure("No Chrome profile has this ID", "PROFILE_UNKNOWN")
    });
    owner::update_pairing(
        &Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"profiles":[{"profileId":format!("edge:{BARE}"),"connected":true}]}}),
    );
    let _ = core
        .shared
        .chrome_control
        .owner
        .call("chrome_session", json!({"action":"open","profileId":BARE}))
        .await;
    assert_eq!(seen.lock().unwrap().last().unwrap().1["profileId"], SCOPED);
    core.shutdown().await;
}
