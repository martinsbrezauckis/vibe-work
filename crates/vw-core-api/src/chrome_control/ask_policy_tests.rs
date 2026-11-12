//! Browser action/mode contract: ordinary Ask reads run; writes request a package; secret reads require Full access.
//! Uses the authenticated hook's classifier and call-time guard, with no native browser.
use super::{check, mapping, read_consent, read_consent_tests::core};
use serde_json::json;
use vw_ask::chrome::Tier;

const WRITES: &[&str] = &[
    "notifications.create",
    "notifications.clear",
    "page.insert_text",
    "network.intercept",
    "auth.basic",
    "files.drop",
    "webmcp.call",
    "page.type",
    "files.upload",
    "page.commit",
    "cookies.change",
    "storage.change",
    "bookmarks.edit",
    "tabs.group_update",
    "bookmarks.remove",
    "history.delete",
    "page.drag",
    "page.dialog.accept",
    "page.dialog.prompt",
    "auth.fill",
];

#[tokio::test]
async fn choices_execute_without_cards_in_each_browser_mode() {
    let core = core("browser-choice-packages").await;
    let seen = super::test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return super::output_tests::status(args);
        }
        assert!(
            args["_approvedCapabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("page.edit"))
        );
        super::output_tests::mcp(json!({"status":"completed","data":{"action":args["action"]}}))
    });
    let mut differences = Vec::new();
    for mode in ["ask", "accept-edits", "auto-approve"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        for action in ["page.select", "page.check", "page.uncheck"] {
            let input = if action == "page.select" {
                json!({"ref":"choice","value":"existing"})
            } else {
                json!({"ref":"choice"})
            };
            let mut args = json!({"sessionId":"t","action":action,"args":input});
            if read_consent::needed(&core.shared, "s", &args) {
                differences.push(format!("{mode} {action} still asks"));
                continue;
            }
            assert!(
                check::authorize(&core.shared, "s", "chrome_execute", &args).is_ok(),
                "{mode} {action}"
            );
            let prepared = crate::agent_tools::prepare(
                &core.shared,
                "s",
                "s",
                "choice",
                "chrome_execute",
                &mut args,
            )
            .await
            .unwrap();
            let answer = if let Some(execute) = prepared {
                execute.await.unwrap()
            } else {
                super::gateway::call(&core.shared, "s", "chrome_execute", args)
                    .await
                    .unwrap()
            };
            assert_eq!(
                answer["vwMcpResult"]["structuredContent"]["status"], "completed",
                "{mode} {action}"
            );
            assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
        }
    }
    let calls = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "chrome_execute")
        .count();
    core.shutdown().await;
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    assert_eq!(calls, 9);
}

#[tokio::test]
async fn custom_user_agent_text_asks_while_display_emulation_runs() {
    let core = core("browser-emulation-data").await;
    for (args, expected) in [
        (json!({"width":390,"height":844}), false),
        (json!({"reset":true}), false),
        (json!({"userAgent":"custom client metadata"}), true),
    ] {
        assert_eq!(
            read_consent::needed(
                &core.shared,
                "s",
                &json!({"action":"tabs.emulate","args":args})
            ),
            expected
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn every_catalog_action_runs_asks_or_refuses_secret_reads_in_ask() {
    let core = core("browser-action-packages").await;
    let shared = &core.shared;
    let mut differences = Vec::new();
    for action in mapping::action_names() {
        let args = json!({"sessionId":"t","action":action,"args":{"key":"Tab"}});
        let asks = read_consent::needed(shared, "s", &args);
        let expected = WRITES.contains(&action);
        let secret = matches!(
            mapping::need("chrome_execute", &args),
            vw_ask::chrome::Need::SecretRead(_)
        );
        if asks != expected
            || (!expected
                && check::authorize(shared, "s", "chrome_execute", &args).is_ok() == secret)
        {
            differences.push(format!(
                "{action}: expected {}, card={asks}, authorization={:?}",
                if expected { "ask" } else { "run" },
                check::authorize(shared, "s", "chrome_execute", &args)
            ));
        }
    }
    core.shutdown().await;
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[tokio::test]
async fn typing_keys_ask_navigation_runs_and_clipboard_requires_full() {
    let core = core("browser-key-packages").await;
    let mut differences = Vec::new();
    for (key, expected) in [
        ("a", true),
        ("Control+V", false),
        ("Delete", true),
        ("Backspace", true),
        ("Enter", true),
        ("Space", true),
        (" ", true),
        ("Tab", false),
        ("Shift+Tab", false),
        ("Escape", false),
        ("ArrowDown", false),
        ("PageUp", false),
    ] {
        let args = json!({"sessionId":"t","action":"page.key","args":{"key":key}});
        if read_consent::needed(&core.shared, "s", &args) != expected {
            differences.push(key);
        }
    }
    let paste = json!({"sessionId":"t","action":"page.key","args":{"key":"Control+V"}});
    assert_eq!(
        mapping::required("chrome_execute", &paste),
        Some(Tier::FullAccess)
    );
    assert!(
        check::authorize(&core.shared, "s", "chrome_execute", &paste)
            .unwrap_err()
            .contains("Full")
    );
    core.shutdown().await;
    assert!(
        differences.is_empty(),
        "wrong key decisions: {differences:?}"
    );
}

#[tokio::test]
async fn wider_modes_keep_action_tiers_and_higher_write_cards() {
    let core = core("browser-existing-mode-tiers").await;
    for mode in ["accept-edits", "smart-approve", "auto-approve"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        for action in mapping::action_names() {
            let args = json!({"sessionId":"t","action":action,"args":{"key":"Tab"}});
            let required = mapping::required("chrome_execute", &args).unwrap();
            let tier = Tier::from_mode(mode).unwrap();
            let secret = matches!(
                mapping::need("chrome_execute", &args),
                vw_ask::chrome::Need::SecretRead(_)
            );
            assert_eq!(
                read_consent::needed(&core.shared, "s", &args),
                required > tier && !secret,
                "{mode} {action}"
            );
            assert_eq!(
                check::authorize(&core.shared, "s", "chrome_execute", &args).is_ok(),
                mapping::required("chrome_execute", &args).unwrap()
                    <= Tier::from_mode(mode).unwrap(),
                "{mode} {action}"
            );
        }
    }
    core.shutdown().await;
}

#[test]
fn arbitrary_page_javascript_requires_full_mode_because_it_can_read_site_secrets() {
    let args = json!({"action":"raw.execute","args":{"expression":"document.cookie"}});
    for mode in ["ask", "accept-edits"] {
        assert!(
            mapping::check_full(Some(mode), "chrome_execute", &args).is_err(),
            "{mode}"
        );
        assert!(
            super::builtin::check_full(
                Some(mode),
                &json!({"action":"raw.execute","expression":"document.cookie"})
            )
            .is_err(),
            "builtin {mode}"
        );
    }
    assert!(mapping::check_full(Some("auto-approve"), "chrome_execute", &args).is_ok());
}
