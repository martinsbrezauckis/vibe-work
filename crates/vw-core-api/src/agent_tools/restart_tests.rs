//! A restart card and its one-use receipt carry the same launch snapshot, even if the record changes.
//! Uses the server-free core fixture; run by the core unit suite.
use super::launches::Launch;
use crate::{
    ask::tool_calls::{self, receipts},
    feature_test_core::Fixture,
};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn restart_card_shows_and_retains_the_launch_command_folder_and_port() {
    let f = Fixture::new().await;
    let folder = f.root.join("project");
    f.load("s", &folder, &[], false);
    let shared = &f.core.shared;
    let tools = shared.agent_tools.as_ref().unwrap();
    tools.table.for_session("s").unwrap();
    let process = vw_procmon::identity(std::process::id()).unwrap();
    let launch = Launch {
        session_id: "s".into(),
        process,
        command: "echo approved".into(),
        shell: None,
        cwd: folder.clone(),
        port: Some(42123),
        log: f.root.join("output.log"),
        started_ms: 1,
    };
    tools.launches.insert(launch.clone());
    let owner = shared.clone();
    let job = tokio::spawn(async move {
        receipts::issue(
            &owner,
            &json!({"session_id":"s", "tool_call_id":"restart-1",
            "tool_name":"mcp_vw.process_restart", "tool_input":{"pid":process.pid}}),
        )
        .await
    });
    let ask = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(ask) = f.core.asks().into_iter().next() {
                break ask;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut state = json!({"history":[]});
    tool_calls::overlay(shared, "s", &mut state);
    let input = &state["history"][0]["detail"]["effect"]["input"];
    assert_eq!(
        input["command"], launch.command,
        "the card must name the launch record's command"
    );
    assert_eq!(input["cwd"], json!(folder));
    assert_eq!(input["port"], launch.port.unwrap());
    let mut replacement = launch.clone();
    replacement.command = "echo substituted".into();
    tools.launches.insert(replacement);
    tool_calls::answer_by(
        shared,
        crate::Origin::Ui,
        "s",
        ask.id.strip_prefix("callback:").unwrap(),
        &json!({"decision":{"type":"approve"}}),
        vw_ask::AnsweredBy::Person,
    )
    .unwrap();
    let mut args = job.await.unwrap().unwrap();
    assert!(
        receipts::take(shared, "s", "process_restart", &mut args)
            .unwrap()
            .execution
            .is_none()
    );
    let retained: Launch = serde_json::from_value(args["_vwRestart"].clone()).unwrap();
    assert_eq!(
        retained, launch,
        "execution uses the approved launch, not a new lookup"
    );
    f.finish().await;
}
