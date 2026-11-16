//! The browser pause at its routes: which agent calls wait, the receipt's long wait, the person's message and command
//! routes, and that the agent's own tool set has no way to pause or resume.
//!
//! PREDICT (written before the receipt wait and the person routes): a paused chat's browser receipt is issued at
//! once (the call would run while the person uses the page); `PausedTools` does not exist as a command; the person's
//! next message leaves the browser paused.
//! PREDICT (before the App preview joined the gate): `preview_open`, `preview_screenshot` and
//! `preview_console` get their receipt and run at once while the person's browser Pause holds.
use super::*;
use crate::paused_tools::{BROWSER, DEFAULT_WAIT};
use serde_json::json;
use std::sync::Arc;

#[test]
fn page_calls_wait_but_status_and_ending_a_session_do_not() {
    for (tool, args) in [
        ("chrome_execute", json!({"action":"page.read"})),
        ("chrome_session", json!({"action":"open"})),
        ("browser_act", json!({"action":"click"})),
        ("browser_read", json!({})),
        ("browser_capture", json!({})),
        ("preview_open", json!({"port":28100})),
        ("preview_screenshot", json!({})),
        ("preview_console", json!({})),
    ] {
        assert!(browser_call(tool, &args), "{tool} waits");
    }
    for (tool, args) in [
        ("chrome_status", json!({})),
        ("preview_list", json!({})),
        ("chrome_session", json!({"action":"stop"})),
        ("chrome_session", json!({"action":"get"})),
        ("processes_list", json!({})),
    ] {
        assert!(!browser_call(tool, &args), "{tool} never waits");
    }
}

#[test]
fn the_agent_has_no_tool_to_pause_or_resume() {
    let names = vw_agent_tools::wire::tools::SESSION_TOOLS;
    assert!(
        !names
            .iter()
            .any(|name| name.contains("pause") || name.contains("resume")),
        "{names:?}"
    );
}

async fn core(name: &str, running: bool) -> crate::Core {
    let core = crate::offline_core::start_with(name, |_, _| {}).await;
    let _ = core.shared.store.load(json!({
        "session":{"id":"a","status":{"type":if running {"running"} else {"idle"},"activeTurnId":running.then_some("t")}},
        "history":[],"turns":[{"id":"t","status":if running {"in_progress"} else {"completed"}}]
    }), vw_session::LoadKind::New).unwrap();
    core
}

#[tokio::test]
async fn the_long_wait_ends_on_resume_and_on_the_deadline_with_a_plain_answer() {
    let core = core("pause-gate-wait", true).await;
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let waiting = tokio::spawn({
        let shared = core.shared.clone();
        async move { wait(&shared, "a", DEFAULT_WAIT).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!waiting.is_finished());
    core.shared.paused_tools.resume("a", None);
    assert_eq!(waiting.await.unwrap(), Ok(()));
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    assert_eq!(
        wait(&core.shared, "a", std::time::Duration::from_millis(20)).await,
        Err(crate::paused_tools::TIMED_OUT.to_owned())
    );
    core.shutdown().await;
}

#[tokio::test]
async fn the_person_pauses_and_resumes_by_command_and_by_their_next_message() {
    let core = core("pause-gate-command", true).await;
    let pause = crate::Command::PausedTools {
        request: crate::PausedToolsRequest::Pause {
            session_id: "a".into(),
            place: Some("the built-in browser".into()),
        },
    };
    let result = core.execute(crate::Origin::Ui, pause).await.unwrap();
    let crate::CommandResult::PausedTools { view } = result else {
        panic!("{result:?}")
    };
    assert_eq!(view.paused[0].place, "the built-in browser");
    assert!(!view.paused[0].waiting);
    crate::paused_tools_commands::note_command(
        &core.shared,
        &crate::Command::SteerTurn {
            session_id: "a".into(),
            expected_turn_id: "t".into(),
            message: vec![],
            client_user_message_id: None,
        },
    );
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let resume = crate::Command::PausedTools {
        request: crate::PausedToolsRequest::Resume {
            session_id: "a".into(),
        },
    };
    core.execute(crate::Origin::DebugApi, resume).await.unwrap();
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    core.shutdown().await;
}

#[tokio::test]
async fn pause_needs_a_working_agent() {
    let core = core("pause-gate-idle", false).await;
    let pause = crate::Command::PausedTools {
        request: crate::PausedToolsRequest::Pause {
            session_id: "a".into(),
            place: None,
        },
    };
    let error = core.execute(crate::Origin::Ui, pause).await.unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::InvalidParams);
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    core.shutdown().await;
}

#[tokio::test]
async fn the_resumed_note_goes_first_once() {
    let core = core("pause-gate-note", true).await;
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    core.shared.paused_tools.resume("a", None);
    let mut result = json!({"content":[{"type":"text","text":"page body"}]});
    note_resumed(&core.shared, "a", &mut result);
    assert_eq!(
        result["content"][0]["text"],
        crate::paused_tools::RESUMED_NOTE
    );
    let mut again = json!({"content":[{"type":"text","text":"page body"}]});
    note_resumed(&core.shared, "a", &mut again);
    assert_eq!(again["content"].as_array().unwrap().len(), 1);
    let _ = Arc::strong_count(&core.shared);
    core.shutdown().await;
}

/// The pre-tool receipt is where the long wait happens (no transport deadline): a paused chat's browser call is not
/// given its receipt until Resume, and a Stop answers it unrun.
#[tokio::test]
async fn the_receipt_waits_while_paused_and_stop_answers_unrun() {
    let f = crate::feature_test_core::Fixture::new().await;
    let project = f.root.join("project");
    f.load("a", &project, &[], true);
    // The chat's tool credential registers it with the receipt route.
    let _credential = f
        .core
        .shared
        .agent_tools
        .clone()
        .unwrap()
        .table
        .for_session("a");
    let issue = |call: &'static str| {
        let shared = f.core.shared.clone();
        tokio::spawn(async move {
            crate::ask::tool_calls::receipts::issue(
                &shared,
                &json!({"session_id":"a","tool_call_id":call,"tool_name":"mcp_vw.chrome_execute",
                        "tool_input":{"sessionId":"task","action":"page.read","args":{}}}),
            )
            .await
        })
    };
    f.core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let held = issue("paused-1");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!held.is_finished(), "no receipt while paused");
    assert!(f.core.shared.paused_tools.view("a").paused[0].waiting);
    crate::stops::stopped(&f.core.shared, "a");
    assert_eq!(held.await.unwrap(), Err(STOPPED.to_owned()));

    f.core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let held = issue("paused-2");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!held.is_finished());
    f.core.shared.paused_tools.resume("a", None);
    let after = tokio::time::timeout(std::time::Duration::from_secs(10), held)
        .await
        .expect("the receipt continues after Resume")
        .unwrap();
    // What follows is the receipt's ordinary route (it may refuse for its own reasons), never the pause's answers.
    if let Err(reason) = &after {
        assert_ne!(reason, STOPPED);
        assert_ne!(reason, crate::paused_tools::TIMED_OUT);
    }
    f.finish().await;
}

/// A finished turn ends the pause whether it arrives as an event or a snapshot (`turn_finished`), but a late end of
/// an older turn while a newer turn runs does not lift the person's pause.
#[tokio::test]
async fn only_the_running_turns_end_lifts_the_pause() {
    let core = core("pause-gate-turn-end", true).await;
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    crate::paused_tools_commands::turn_finished(&core.shared, "a");
    assert!(
        core.shared.paused_tools.is_paused("a", BROWSER),
        "the latest turn still runs: an older turn's end changes nothing"
    );
    let _ = core
        .shared
        .store
        .load(
            json!({
                "session":{"id":"a","status":{"type":"idle"}},
                "history":[],"turns":[{"id":"t","status":"completed"}]
            }),
            vw_session::LoadKind::New,
        )
        .unwrap();
    crate::paused_tools_commands::turn_finished(&core.shared, "a");
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    core.shutdown().await;
}

/// The App preview's page tools wait for the person's browser Pause like browser tools: at the receipt (the long
/// wait) and again at dispatch for a pause that started after the receipt; Stop answers them unrun.
#[tokio::test]
async fn app_preview_page_tools_wait_while_paused_at_the_receipt_and_at_dispatch() {
    let f = crate::feature_test_core::Fixture::new().await;
    let project = f.root.join("project");
    f.load("a", &project, &[], true);
    let tools = f.core.shared.agent_tools.clone().unwrap();
    let _credential = tools.table.for_session("a");
    f.core
        .shared
        .paused_tools
        .pause("a", BROWSER, "the built-in browser");
    let receipt = tokio::spawn({
        let shared = f.core.shared.clone();
        async move {
            crate::ask::tool_calls::receipts::issue(
                &shared,
                &json!({"session_id":"a","tool_call_id":"preview-1","tool_name":"mcp_vw.preview_open",
                        "tool_input":{"port":28100}}),
            )
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !receipt.is_finished(),
        "no preview_open receipt while paused"
    );
    crate::stops::stopped(&f.core.shared, "a");
    let bound = std::time::Duration::from_secs(20);
    let answer = tokio::time::timeout(bound, receipt).await;
    assert_eq!(answer.expect("answered").unwrap(), Err(STOPPED.to_owned()));

    for tool in ["preview_open", "preview_screenshot", "preview_console"] {
        f.core
            .shared
            .paused_tools
            .pause("a", BROWSER, "the built-in browser");
        let dispatch = tokio::spawn({
            let shared = f.core.shared.clone();
            let tools = tools.clone();
            async move { crate::agent_tools::call_prepared(&shared, &tools, "a", tool, json!({})).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!dispatch.is_finished(), "{tool} does not run while paused");
        crate::stops::stopped(&f.core.shared, "a");
        let answer = tokio::time::timeout(bound, dispatch).await;
        assert_eq!(
            answer.expect("answered").unwrap(),
            Err(STOPPED.to_owned()),
            "{tool}"
        );
    }
    f.finish().await;
}
