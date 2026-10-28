//! Real hook refusals must reach the core subscription once, with the step identity and no arguments.
#![cfg(unix)]
mod agent_tools;
mod common;
mod procmon;
use agent_tools::{Dirs, rig};
use serde_json::{Value, json};
use vw_agent_tools::hook::Guard;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hook_refusals_publish_one_event_with_the_step_and_redacted_text() {
    let dirs = Dirs::new("security-bell");
    let rig = rig(&dirs, "security-bell").await;
    let session = rig.start_session(&dirs.root, "ask").await;
    let mut events = rig.core.subscribe(Some(session.clone()));
    while events.try_next().is_some() {}
    let endpoint = rig.endpoint_file();
    let id = session.clone();
    let out = tokio::task::spawn_blocking(move || {
        Guard::Git.decide(&json!({"session_id": id, "cwd": "/tmp", "tool_call_id": "call-42", "tool_name": "file_system.bash", "tool_input": {"command": "git push https://user:never-show-this@127.0.0.1/repo main"}}).to_string(), &endpoint)
    }).await.unwrap().unwrap();
    let mut found = Vec::new();
    while let Some(event) = events.try_next() {
        let event = serde_json::to_value(event).unwrap();
        if event["type"] == "guardRefused" {
            found.push(event);
        }
    }
    assert_eq!(
        found.len(),
        1,
        "a guard refusal must publish exactly one event"
    );
    assert_eq!(found[0]["sessionId"], session);
    assert_eq!(found[0]["guard"], "git");
    assert_eq!(found[0]["stepId"], "call-42");
    assert!(!found[0].to_string().contains("never-show-this"));
    assert!(
        serde_json::from_str::<Value>(&out).unwrap()["reason"]
            .as_str()
            .unwrap()
            .starts_with(found[0]["what"].as_str().unwrap().trim_end_matches('…'))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_and_core_assisted_hooks_report_each_final_refusal_once() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use vw_gate::Confirmation;
    let dirs = Dirs::new("security-classes");
    let rig = rig(&dirs, "security-classes").await;
    let session = rig.start_session(&dirs.root, "ask").await;
    let home = dirs.root.join("vibe-home");
    std::fs::create_dir_all(&home).unwrap();
    let mut events = rig.core.subscribe(Some(session.clone()));
    while events.try_next().is_some() {}
    for (guard, input, expected) in [
        (
            "git",
            json!({"command": "git credential fill --password=never-show-this"}),
            "git",
        ),
        (
            "safety",
            json!({"command": "shutdown -h now --token=never-show-this"}),
            "safetyRule",
        ),
        (
            "folder",
            json!({"file_path": home.join("hooks.toml"), "content": "never-show-this"}),
            "folder",
        ),
        (
            "process",
            json!({"command": "kill -9 1 # never-show-this"}),
            "process",
        ),
    ] {
        rig.confirmer.then(Confirmation::Declined);
        let endpoint = rig.endpoint_file();
        let (session2, home2, root) = (session.clone(), home.clone(), dirs.root.clone());
        let output = tokio::task::spawn_blocking(move || {
            let mut child = Command::new(agent_tools::BIN).args(["guard", guard, "--endpoint-file"]).arg(endpoint)
                .env("VIBE_HOME", home2).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            let tool = if guard == "folder" { "file_system.write_file" } else { "file_system.bash" };
            write!(child.stdin.take().unwrap(), "{}", json!({"session_id": session2, "cwd": root, "tool_call_id": format!("call-{guard}"), "tool_name": tool, "tool_input": input})).unwrap();
            child.wait_with_output().unwrap()
        }).await.unwrap();
        assert!(output.status.success());
        let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(answer["decision"], "deny", "{guard}: {answer}");
        let mut found = Vec::new();
        while let Some(event) = events.try_next() {
            let event = serde_json::to_value(event).unwrap();
            if event["type"] == "guardRefused" {
                found.push(event);
            }
        }
        assert_eq!(found.len(), 1, "{guard}: {found:?}");
        assert_eq!(found[0]["guard"], expected);
        assert_eq!(found[0]["sessionId"], session);
        assert_eq!(found[0]["stepId"], format!("call-{guard}"));
        assert!(!found[0].to_string().contains("never-show-this"));
    }
    rig.core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn core_process_tools_refuse_foreign_stops_and_restarts_once() {
    let dirs = Dirs::new("security-process-tools");
    let rig = rig(&dirs, "security-process-tools").await;
    let session = rig.start_session(&dirs.root, "ask").await;
    let who = agent_tools::credential_of(&rig.injected(0));
    let mut events = rig.core.subscribe(Some(session.clone()));
    while events.try_next().is_some() {}
    for tool in ["process_stop", "process_restart"] {
        assert!(
            agent_tools::call(&who, tool, json!({"pid":std::process::id()}))
                .await
                .is_err()
        );
        let mut found = Vec::new();
        while let Some(event) = events.try_next() {
            let event = serde_json::to_value(event).unwrap();
            if event["type"] == "guardRefused" {
                found.push(event);
            }
        }
        assert_eq!(found.len(), 1, "{tool}: {found:?}");
        assert_eq!(found[0]["guard"], "process");
        assert_eq!(found[0]["sessionId"], session);
    }
    rig.core.shutdown().await;
}
