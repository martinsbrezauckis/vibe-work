//! Read and browser tools in each access mode: Accept edits and Full access skip their default card;
//! explicit tool rules and other modes keep their approval.
use super::permission;
use serde_json::{Value, json};
use std::time::Duration;
use vw_agent_tools::wire::tools as names;
use vw_browser::capture::What;

const READS: [&str; 10] = [
    names::PREVIEW_LIST,
    names::PREVIEW_SCREENSHOT,
    names::PREVIEW_CONSOLE,
    names::SEARCH,
    names::PROCESSES_LIST,
    names::HANDBOOK_CHECK,
    names::HANDBOOK_ANSWERS,
    names::PORT_LEASE,
    names::PANEL_READ,
    names::BROWSER_READ,
];

/// True when the call waited on an approval card; false when it was allowed at once.
async fn asks(
    shared: &std::sync::Arc<crate::core::Shared>,
    tool: &'static str,
    args: Value,
) -> bool {
    let caller = shared.clone();
    let task =
        tokio::spawn(async move { permission(&caller, "s", "s", "call", tool, &args).await });
    // Reading stricter rules uses the blocking pool, which can be busy in the full suite.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !task.is_finished() && shared.asks.list().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{tool} waits without a card"));
    if task.is_finished() {
        let answer = task.await.unwrap();
        assert!(answer.is_ok(), "{tool}: {answer:?}");
        return false;
    }
    assert!(
        !shared.asks.list().is_empty(),
        "{tool} waits without a card"
    );
    task.abort();
    let _ = task.await;
    true
}

#[tokio::test]
async fn reads_run_without_a_card_in_accept_edits_and_full_access_only() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    for mode in [
        "ask",
        "accept-edits",
        "smart-approve",
        "auto-approve",
        "vw-plan",
        "explore",
    ] {
        shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        let free = matches!(mode, "accept-edits" | "auto-approve");
        for tool in READS {
            assert_eq!(
                asks(shared, tool, json!({})).await,
                !(free || mode == "ask" && tool == names::BROWSER_READ),
                "{mode} {tool}"
            );
        }
        for (tool, args) in [
            (names::BROWSER_CAPTURE, json!({"what":What::Page})),
            (
                names::BROWSER_READ,
                json!({"tab":"new","url":"https://example.com/"}),
            ),
        ] {
            assert_eq!(
                asks(shared, tool, args).await,
                !(free || mode == "ask"),
                "{mode} {tool}"
            );
        }
        for (tool, args) in [
            (names::BROWSER_ACT, json!({"action":"click","ref":"r"})),
            (names::PREVIEW_OPEN, json!({"url":"http://localhost:5173/"})),
            (names::PROCESS_START, json!({"command":"echo hello"})),
            // Preview resizing and handbook repairs keep their card.
            (names::PREVIEW_SCREENSHOT, json!({"width":900})),
            (names::HANDBOOK_CHECK, json!({"repair":"stylesheet"})),
        ] {
            assert_eq!(
                asks(shared, tool, args).await,
                mode != "auto-approve"
                    && !(tool == names::BROWSER_ACT
                        && matches!(mode, "ask" | "accept-edits" | "smart-approve")),
                "{mode} {tool}"
            );
        }
    }
    fixture.finish().await;
}

#[tokio::test]
async fn builtin_ask_runs_choices_and_navigation_and_asks_for_typing_and_sending() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    let mut differences = Vec::new();
    for action in [
        "navigate",
        "back",
        "forward",
        "reload",
        "click",
        "hover",
        "select",
        "check",
        "scroll",
        "wait",
        "list_tabs",
        "close_tab",
        "type",
        "drag",
        "upload",
    ] {
        let expected = ["type", "drag", "upload"].contains(&action);
        if asks(shared, names::BROWSER_ACT, json!({"action":action})).await != expected {
            differences.push(action);
        }
    }
    for (key, expected) in [
        ("Tab", false),
        ("Escape", false),
        ("ArrowDown", false),
        ("Enter", true),
        ("a", true),
    ] {
        if asks(
            shared,
            names::BROWSER_ACT,
            json!({"action":"press","key":key}),
        )
        .await
            != expected
        {
            differences.push(key);
        }
    }
    fixture.finish().await;
    assert!(
        differences.is_empty(),
        "wrong built-in decisions: {differences:?}"
    );
}

#[tokio::test]
async fn builtin_choices_run_without_cards_in_each_browser_mode() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    let mut differences = Vec::new();
    for mode in ["ask", "accept-edits", "auto-approve"] {
        shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        for args in [
            json!({"action":"select","ref":"choice","value":"existing"}),
            json!({"action":"check","ref":"choice","checked":true}),
            json!({"action":"check","ref":"choice","checked":false}),
        ] {
            if asks(shared, names::BROWSER_ACT, args.clone()).await {
                differences.push(format!("{mode} {args} still asks"));
            }
        }
    }
    fixture.finish().await;
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[tokio::test]
async fn headless_activation_asks_once_in_ask_and_keeps_other_modes() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    for (mode, expected) in [
        ("ask", true),
        ("accept-edits", false),
        ("auto-approve", false),
    ] {
        shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        assert_eq!(
            asks(shared, names::USE_HEADLESS_BROWSER, json!({})).await,
            expected,
            "{mode}"
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn headless_yes_covers_activation_in_this_chat_and_no_keeps_it_off() {
    use crate::{Origin, ask::tool_calls};
    for decision in ["approve", "deny"] {
        let fixture = crate::feature_test_core::Fixture::new().await;
        fixture.load("s", &fixture.root.join("work"), &[], true);
        let shared = &fixture.core.shared;
        shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
        let caller = shared.clone();
        let task = tokio::spawn(async move {
            permission(
                &caller,
                "s",
                "s",
                "activate",
                names::USE_HEADLESS_BROWSER,
                &json!({}),
            )
            .await
        });
        let id = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(card) = tool_calls::open(shared).first() {
                    break card.callback_id.clone();
                }
                assert!(!task.is_finished());
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        tool_calls::answer_by(
            shared,
            Origin::Ui,
            "s",
            &id,
            &json!({"decision":{"type":decision}}),
            vw_ask::AnsweredBy::Person,
        )
        .unwrap();
        assert_eq!(task.await.unwrap().is_ok(), decision == "approve");
        if decision == "approve" {
            assert!(!asks(shared, names::USE_HEADLESS_BROWSER, json!({})).await);
            assert!(
                !shared
                    .agent_tools
                    .as_ref()
                    .unwrap()
                    .on_demand
                    .consented("another-chat")
            );
        } else {
            assert!(asks(shared, names::USE_HEADLESS_BROWSER, json!({})).await);
        }
        fixture.finish().await;
    }
}

/// Only the default card goes: a stricter rule the person set in Vibe keeps it. With file
/// reads set to ask, search still asks in Accept edits (`preview_list`, which reads no files, does not); the tool's own
/// rule keeps its card too; the default config has no card.
#[tokio::test]
async fn a_stricter_rule_the_person_set_keeps_the_card() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"accept-edits"}),
    );
    let config = shared.instructions.own_home.join("config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    assert!(!asks(shared, names::SEARCH, json!({"query":"x"})).await);
    std::fs::write(&config, "[tools.read_file]\npermission = \"ask\"\n").unwrap();
    assert!(asks(shared, names::SEARCH, json!({"query":"x"})).await);
    assert!(asks(shared, names::PANEL_READ, json!({})).await);
    assert!(!asks(shared, names::PREVIEW_LIST, json!({})).await);
    for tool in [
        names::PROCESSES_LIST,
        names::HANDBOOK_CHECK,
        names::HANDBOOK_ANSWERS,
        names::PORT_LEASE,
        names::PANEL_READ,
        names::BROWSER_READ,
    ] {
        std::fs::write(
            &config,
            format!("[tools.vw_{tool}]\npermission = \"ask\"\n"),
        )
        .unwrap();
        assert!(asks(shared, tool, json!({})).await, "{tool}");
    }
    std::fs::write(&config, "[tools.vw_preview_list]\npermission = \"ask\"\n").unwrap();
    assert!(asks(shared, names::PREVIEW_LIST, json!({})).await);
    std::fs::remove_file(&config).unwrap();
    assert!(!asks(shared, names::SEARCH, json!({"query":"x"})).await);
    // A project's own rule counts too.
    let project = fixture.root.join("work/.vibe/config.toml");
    std::fs::create_dir_all(project.parent().unwrap()).unwrap();
    std::fs::write(&project, "[tools.read_file]\npermission = \"ask\"\n").unwrap();
    assert!(asks(shared, names::SEARCH, json!({"query":"x"})).await);
    fixture.finish().await;
}

#[tokio::test]
async fn builtin_browser_ask_runs_navigation_and_choices_but_asks_before_typing() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    for action in [
        "navigate",
        "click",
        "select",
        "check",
        "list_tabs",
        "back",
        "scroll",
    ] {
        assert!(
            !asks(shared, names::BROWSER_ACT, json!({"action":action})).await,
            "{action}"
        );
    }
    assert!(!asks(shared, names::BROWSER_READ, json!({})).await);
    assert!(asks(shared, names::BROWSER_ACT, json!({"action":"type"})).await);
    fixture.finish().await;
}

#[tokio::test]
async fn builtin_dialog_confirmation_keeps_chromes_explicit_send_approval() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    for action in ["page.dialog.accept", "page.dialog.prompt", "upload", "type"] {
        assert!(
            asks(shared, names::BROWSER_ACT, json!({"action":action})).await,
            "{action}"
        );
    }
    for action in [
        "page.dialogs",
        "page.dialog.dismiss",
        "files.list_downloads",
        "files.wait_download",
        "window.open",
        "window.focus",
        "window.close",
        "page.double_click",
        "page.right_click",
        "page.middle_click",
        "page.triple_click",
    ] {
        assert!(
            !asks(shared, names::BROWSER_ACT, json!({"action":action})).await,
            "{action}"
        );
    }
    assert!(
        !asks(
            shared,
            names::BROWSER_ACT,
            json!({"action":"files.download","url":"https://example.test/report.pdf"}),
        )
        .await,
        "an ordinary GET download runs in Ask without a card"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn builtin_secret_reads_refuse_below_full_without_a_card() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    for mode in ["ask", "accept-edits", "auto-approve"] {
        shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        for action in ["cookies.read", "storage.read", "clipboard.read"] {
            let args = json!({"action":action});
            let outcome = tokio::time::timeout(
                Duration::from_millis(500),
                permission(shared, "s", "s", "call", names::BROWSER_ACT, &args),
            )
            .await;
            assert!(
                outcome.is_ok(),
                "{mode} {action} must not offer a mode-upgrade card"
            );
            let outcome = outcome.unwrap();
            if mode == "auto-approve" {
                assert!(outcome.is_ok(), "{action}: {outcome:?}");
            } else {
                assert!(
                    outcome
                        .unwrap_err()
                        .contains(vw_ask::chrome::Tier::FullAccess.label()),
                    "{mode} {action}"
                );
            }
            assert_eq!(shared.asks.list().len(), 0);
        }
    }
    fixture.finish().await;
}

#[tokio::test]
async fn builtin_actions_match_chrome_in_accept_edits() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("s", &fixture.root.join("work"), &[], true);
    let shared = &fixture.core.shared;
    shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"accept-edits"}),
    );
    for action in [
        "scroll",
        "wait",
        "list_tabs",
        "navigate",
        "back",
        "forward",
        "reload",
        "click",
        "hover",
        "type",
        "select",
        "check",
        "press",
        "drag",
        "upload",
    ] {
        assert!(
            !asks(shared, names::BROWSER_ACT, json!({"action":action})).await,
            "{action}"
        );
    }
    for action in ["raw.execute", "future.action", ""] {
        assert!(
            asks(shared, names::BROWSER_ACT, json!({"action":action})).await,
            "{action}"
        );
    }
    assert!(
        !asks(
            shared,
            names::BROWSER_ACT,
            json!({"action":"navigate","tab":"new","url":"https://example.com/"})
        )
        .await
    );
    assert!(
        !asks(
            shared,
            names::BROWSER_READ,
            json!({"tab":"new","url":"https://example.com/"})
        )
        .await
    );
    for what in [
        "design",
        "structure",
        "media",
        "page",
        "bundle",
        "before",
        "after",
    ] {
        assert!(
            !asks(shared, names::BROWSER_CAPTURE, json!({"what":what})).await,
            "capture: {what}"
        );
    }
    let config = shared.instructions.own_home.join("config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "[tools.vw_browser_act]\npermission = \"ask\"\n").unwrap();
    for action in ["scroll", "wait", "list_tabs", "click", "type"] {
        assert!(
            asks(shared, names::BROWSER_ACT, json!({"action":action})).await,
            "explicit rule: {action}"
        );
    }
    for (tool, args) in [
        (names::BROWSER_READ, json!({"tab":"new"})),
        (names::BROWSER_CAPTURE, json!({"what":What::Page})),
    ] {
        std::fs::write(
            &config,
            format!("[tools.vw_{tool}]\npermission = \"ask\"\n"),
        )
        .unwrap();
        assert!(asks(shared, tool, args).await, "explicit rule: {tool}");
    }
    fixture.finish().await;
}
