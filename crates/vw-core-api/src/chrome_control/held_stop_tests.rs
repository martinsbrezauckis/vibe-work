//! The person's Stop while the browser is paused and an agent call waits in its pre-tool receipt: the chat names the
//! call "Not run: you pressed Stop while the browser was paused." however the runtime then reports it.
//!
//! PREDICT (written before the fix): the app ends the held call with the paused-browser Stop answer, but nothing in
//! the history the chat reads says so unless the runtime turns that answer into a skipped step with the receipt's
//! reason. When the Stop's interrupt reaches the runtime first, the runtime cancels the hook ("Action cancelled")
//! after its refusal notice (`Denied tool 'preview_open'`) or before any answer, and the chat shows the generic
//! "vibe-work safety reminder refused the call" (or nothing). These tests feed exactly those runtime reports after
//! the pill Stop and the chat-line Stop, and look for the app's mark on the call's hook notices: none exists yet.
use super::person_stop_tests::{fixture_with, status};
use crate::paused_tools::BROWSER;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

const CALL: &str = "typescript:21:ui9CbaQqe:1:tool:3";

async fn core(name: &str) -> crate::Core {
    let core = fixture_with(name, "chrome:p", |config, dir| {
        config.agent_tools = Some(crate::AgentToolsConfig::new(
            dir.join("no-tools-spawned"),
            dir.join("tools"),
            None,
        ));
    })
    .await;
    let tools = core.shared.agent_tools.as_ref().unwrap();
    tools.table.for_session("a").unwrap();
    tools.table.for_session("b").unwrap();
    core
}

/// Vibe's pre-tool hook notices of one call: the start (sent while the call waits) and the hook's refusal.
fn start(session: &str, call: &str) -> Value {
    json!({"id":format!("start-{session}"),"type":"notice","level":"info","message":"Running hooks","sessionId":session,
        "detail":{"kind":"hook_run_started","scope":"pre_tool","status":null,"toolCallId":call,"toolName":null,"hookName":null,"content":null}})
}
fn denied(session: &str, call: &str, tool: &str) -> Value {
    json!({"id":format!("denied-{session}"),"type":"notice","level":"error","message":format!("Denied tool '{tool}'"),"sessionId":session,
        "detail":{"kind":"hook_completed","scope":"pre_tool","status":"error","toolCallId":call,"toolName":null,
        "hookName":"vibe-work safety reminder","content":format!("Denied tool '{tool}'")}})
}

fn added(session: &str, event: u64, entry: &Value) -> Value {
    json!({"jsonrpc":"2.0","method":"history/entryAdded","params":{"sessionId":session,"eventId":event,"entry":entry}})
}

/// A runtime for chat "a" that accepts the interrupt, ends the turn, and `delay` later reports `later` (history
/// notifications), as Vibe reports a hook it cancelled. Returns the interrupts it received.
fn runtime(core: &crate::Core, later: Vec<Value>, delay: Duration) -> Arc<Mutex<Vec<Value>>> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (client, server) = tokio::io::duplex(65536);
    let (read, write) = tokio::io::split(client);
    let connection =
        vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default()).start(read, write);
    core.shared.channels.bind_test_peer("a", connection, (0, 1));
    let seen = calls.clone();
    tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(server);
        let mut lines = BufReader::new(read).lines();
        let mut later = Some(later);
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "turn/interrupt", "{request}");
            seen.lock().unwrap().push(request["params"].clone());
            let reply = json!({"jsonrpc":"2.0","id":request["id"],"result":{"accepted":true}});
            let completed = json!({"jsonrpc":"2.0","method":"turn/completed","params":{
                "sessionId":"a","eventId":1,"turn":{"id":request["params"]["expectedTurnId"],"status":"interrupted"}}});
            for frame in [reply, completed] {
                write
                    .write_all(format!("{frame}\n").as_bytes())
                    .await
                    .unwrap();
            }
            tokio::time::sleep(delay).await;
            for frame in later.take().unwrap_or_default() {
                write
                    .write_all(format!("{frame}\n").as_bytes())
                    .await
                    .unwrap();
            }
        }
    });
    calls
}

/// The agent's call reaches the app's pre-tool receipt route (the hook) while the browser is paused, and waits.
async fn held_call(
    core: &crate::Core,
    session: &str,
    tool: &str,
    input: Value,
) -> tokio::task::JoinHandle<Result<Value, String>> {
    let shared = core.shared.clone();
    let payload =
        json!({"session_id":session,"tool_call_id":CALL,"tool_name":tool,"tool_input":input});
    let hook =
        tokio::spawn(
            async move { crate::ask::tool_calls::receipts::issue(&shared, &payload).await },
        );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!hook.is_finished(), "the call waits while paused");
    hook
}

/// The `vwHeldCall` mark of the entry `id` anywhere in `value`.
fn mark(value: &Value, id: &str) -> Option<String> {
    match value {
        Value::Object(map) if map.get("id").and_then(Value::as_str) == Some(id) => map
            .get("vwHeldCall")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Value::Object(map) => map.values().find_map(|v| mark(v, id)),
        Value::Array(list) => list.iter().find_map(|v| mark(v, id)),
        _ => None,
    }
}

async fn until(what: &str, check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out: {what}"));
}

fn stored(core: &crate::Core, id: &str) -> Option<Value> {
    core.shared.store.view("a").and_then(|view| {
        view.state["history"]
            .as_array()?
            .iter()
            .find(|entry| entry["id"] == id)
            .cloned()
    })
}

/// Pill Stop; the runtime cancels the hook when the interrupt reaches it (here: the hook's request is dropped at
/// once), after its refusal notice. The notice that arrives after the interrupt is stored with the mark, and the
/// start notice that arrived while the call waited is shown again with it.
#[tokio::test]
async fn a_pill_stop_marks_the_held_call_when_the_runtime_cancels_its_hook() {
    let core = core("held-stop-pill").await;
    let mut events = core.subscribe(None);
    let _ = core.shared.store.apply(vw_session::Notification::new(
        "history/entryAdded",
        json!({"sessionId":"a","eventId":1,"entry":start("a", CALL)}),
    ));
    let calls = runtime(
        &core,
        vec![added("a", 2, &denied("a", CALL, "chrome_execute"))],
        Duration::ZERO,
    );
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let hook = held_call(
        &core,
        "a",
        "mcp_vw.chrome_execute",
        json!({"sessionId":"task-a","action":"page.navigate","args":{"url":"https://example.org"}}),
    )
    .await;
    status(&core, "stopping", Instant::now());
    until("the interrupt reaches the runtime", || {
        !calls.lock().unwrap().is_empty()
    })
    .await;
    hook.abort();
    until("the refusal notice is stored", || {
        stored(&core, "denied-a").is_some()
    })
    .await;
    assert_eq!(
        stored(&core, "denied-a").unwrap()["vwHeldCall"],
        "stopped",
        "the refusal notice that followed the interrupt names the person's Stop"
    );
    let shown = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = serde_json::to_value(events.next().await.unwrap()).unwrap();
            if let Some(mark) = mark(&value, "start-a") {
                break mark;
            }
        }
    })
    .await
    .expect("the chat's history is shown again with the held call's mark");
    assert_eq!(shown, "stopped");
    let snapshot = crate::sessions::snapshot(&core.shared, "a").unwrap();
    assert_eq!(mark(&snapshot.state, "start-a").as_deref(), Some("stopped"));
    core.shutdown().await;
}

/// Chat-line Stop of a held App preview call (`preview_open`); the hook gets its Stop answer, but the runtime reports
/// the call only after the interrupt, and late. The late refusal notice is still marked.
#[tokio::test]
async fn a_chat_line_stop_marks_a_held_preview_open_whose_answer_comes_after_the_interrupt() {
    let core = core("held-stop-chat-line").await;
    let calls = runtime(
        &core,
        vec![added("a", 2, &denied("a", CALL, "preview_open"))],
        Duration::from_millis(400),
    );
    core.shared
        .paused_tools
        .pause("a", BROWSER, "the built-in browser");
    let hook = held_call(
        &core,
        "a",
        "mcp_vw.preview_open",
        json!({"url":"http://127.0.0.1:8790/key.html"}),
    )
    .await;
    // The chat line's Stop button sends this command from the window.
    let stop = tokio::spawn({
        let core = crate::Core {
            shared: core.shared.clone(),
        };
        let command = crate::command::Command::InterruptTurn {
            session_id: "a".into(),
            expected_turn_id: "one".into(),
        };
        async move { core.execute(crate::Origin::Ui, command).await.map(|_| ()) }
    });
    let answer = tokio::time::timeout(Duration::from_secs(3), hook)
        .await
        .expect("the held call ends")
        .unwrap();
    assert_eq!(answer, Err(crate::paused_tools_gate::STOPPED.to_owned()));
    let _ = tokio::time::timeout(Duration::from_secs(3), stop).await;
    assert!(
        !calls.lock().unwrap().is_empty(),
        "the Stop interrupted the turn"
    );
    until("the late refusal notice is stored", || {
        stored(&core, "denied-a").is_some()
    })
    .await;
    assert_eq!(stored(&core, "denied-a").unwrap()["vwHeldCall"], "stopped");
    core.shutdown().await;
}

/// The turn's end marks its held call `turnEnded`; a resumed call, the same call id in another chat, and a call of
/// another chat's Stop are never marked.
#[tokio::test]
async fn only_the_held_call_that_stop_or_turn_end_ended_is_marked() {
    let core = core("held-stop-scope").await;
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let resumed = held_call(&core, "a", "mcp_vw.browser_read", json!({})).await;
    core.shared.paused_tools.resume("a", None);
    let _ = tokio::time::timeout(Duration::from_secs(3), resumed).await;
    let mut history = json!({"history":[start("a", CALL), start("b", CALL)]});
    crate::sessions::project_result(
        &core.shared,
        "session/history/list",
        &json!({}),
        &mut history,
    );
    assert_eq!(mark(&history, "start-a"), None, "a resumed call ran");
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let ended = held_call(&core, "a", "mcp_vw.browser_read", json!({})).await;
    core.shared.paused_tools.turn_ended("a");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), ended)
            .await
            .unwrap()
            .unwrap(),
        Err(crate::paused_tools_gate::TURN_ENDED.to_owned())
    );
    let mut history = json!({"history":[start("a", CALL), start("b", CALL)]});
    crate::sessions::project_result(
        &core.shared,
        "session/history/list",
        &json!({}),
        &mut history,
    );
    assert_eq!(mark(&history, "start-a").as_deref(), Some("turnEnded"));
    assert_eq!(
        mark(&history, "start-b"),
        None,
        "the same call id in another chat"
    );
    core.shutdown().await;
}
