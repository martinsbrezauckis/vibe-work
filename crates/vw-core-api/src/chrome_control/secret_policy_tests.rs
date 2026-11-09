//! Browser/mode security contract, exercised through both gateways' permission checks.
//! Uses an offline core and no native browser; secret reads cannot acquire authority from a tool card.
use super::{
    agent_browser::Browser, builtin, check, mapping, read_consent, read_consent_tests::core,
};
use serde_json::{Value, json};
use vw_ask::chrome::Tier;

fn browsers() -> Vec<Option<Browser>> {
    let mut browsers = vec![None, Some(Browser::Chrome)];
    for (enabled, browser) in [
        (cfg!(feature = "your-edge"), Browser::Edge),
        (cfg!(feature = "your-safari"), Browser::Safari),
        (cfg!(feature = "your-firefox"), Browser::Firefox),
    ] {
        if enabled {
            browsers.push(Some(browser));
        }
    }
    browsers
}

fn select(shared: &crate::core::Shared, browser: Browser) {
    let profile = format!("{}:policy", browser.id());
    shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .insert("s".into(), profile.clone());
    shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", &profile, None);
    let dir = shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), "true").unwrap();
    assert_eq!(
        super::agent_browser::selected(shared, "s", "chrome_execute", &json!({"sessionId":"t"})),
        browser
    );
}

fn cases() -> Vec<(&'static str, Value, bool)> {
    vec![
        ("cookies.read", json!({}), true),
        (
            "storage.read",
            json!({"area":"local","keys":["session"]}),
            true,
        ),
        (
            "storage.read",
            json!({"area":"session","keys":["token"]}),
            true,
        ),
        ("clipboard.read", json!({}), true),
        (
            "network.body",
            json!({"requestId":"authenticated-json-response"}),
            true,
        ),
        (
            "network.headers",
            json!({"unredacted":true,"requestId":"cookie-request"}),
            true,
        ),
        (
            "network.headers",
            json!({"unredacted":true,"requestId":"set-cookie-response"}),
            true,
        ),
        ("cookies.metadata", json!({}), false),
        ("storage.metadata", json!({"area":"local"}), false),
        ("storage.metadata", json!({"area":"session"}), false),
        ("network.headers", json!({}), false),
        ("network.headers", json!({"unredacted":false}), false),
        ("network.list", json!({}), false),
        ("page.read", json!({}), false),
        ("page.read", json!({"includeText":true}), false),
        ("capture.screenshot", json!({}), false),
    ]
}

async fn builtin_card(
    shared: &std::sync::Arc<crate::core::Shared>,
    args: &Value,
) -> Result<bool, String> {
    let mut permission = std::pin::pin!(crate::ask::tool_calls::permission(
        shared,
        "s",
        "s",
        "write",
        "browser_act",
        args,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::select! {
                outcome = &mut permission => return outcome.map(|()| false),
                () = tokio::time::sleep(std::time::Duration::from_millis(5)) => {
                    if !shared.asks.list().is_empty() {
                        return Ok(true);
                    }
                }
            }
        }
    })
    .await
    .expect("permission must return or open a card")
}

#[tokio::test]
async fn higher_modes_keep_the_same_write_approval_options_in_every_browser() {
    let core = core("browser-write-mode-table").await;
    let mut differences = Vec::new();
    for browser in browsers() {
        if let Some(browser) = browser {
            select(&core.shared, browser);
        }
        for mode in ["ask", "accept-edits", "smart-approve", "auto-approve"] {
            core.shared.access.observe_request(
                "session/agent/update",
                "s",
                &json!({"agentName":mode}),
            );
            for action in [
                "auth.basic",
                "cookies.change",
                "storage.change",
                "clipboard.write",
            ] {
                let args =
                    json!({"sessionId":"t","action":action,"args":{"login":"approved-login"}});
                let needs_card = mode != "auto-approve";
                if let Some(browser) = browser {
                    let consent = read_consent::Consent(Tier::FullAccess);
                    let outcome = check::authorize_read(
                        &core.shared,
                        "s",
                        "chrome_execute",
                        &args,
                        Some(&consent),
                    );
                    if outcome.is_err()
                        || read_consent::needed(&core.shared, "s", &args) != needs_card
                    {
                        differences.push(format!("{browser:?} {mode} {action}: expected card={needs_card}, approved={outcome:?}"));
                    }
                } else {
                    let native = json!({"action":action,"login":"approved-login"});
                    let outcome = builtin_card(&core.shared, &native).await;
                    if outcome != Ok(needs_card) {
                        differences.push(format!(
                            "built-in {mode} {action}: expected card={needs_card}, got {outcome:?}"
                        ));
                    }
                }
            }
        }
    }
    core.shutdown().await;
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[tokio::test]
async fn secret_reads_never_dispatch_or_offer_a_card_even_in_a_write_batch() {
    let core = core("browser-secret-dispatch").await;
    let seen =
        super::test_support::answering(&core.shared, |_, _| panic!("secret read reached helper"));
    for mode in ["ask", "accept-edits", "smart-approve"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        for (action, input, secret) in cases() {
            if !secret {
                continue;
            }
            let args = json!({"sessionId":"t","action":action,"args":input});
            let reason = super::gateway::call(&core.shared, "s", "chrome_execute", args.clone())
                .await
                .unwrap_err();
            assert!(
                reason.contains("because") && reason.contains("switch this chat"),
                "{reason}"
            );
            // An already approved Full-tier package also fails at execution, including after a mode reduction.
            let reason = read_consent::execute(
                &core.shared,
                "s",
                args.clone(),
                read_consent::Consent(Tier::FullAccess),
            )
            .await
            .unwrap_err();
            assert!(reason.contains(Tier::FullAccess.label()), "{reason}");
            let batch = json!({"sessionId":"t","steps":[
                {"action":"cookies.change","args":{"operation":"clear"}},
                {"action":action,"args":input}
            ]});
            let outcome = super::batch::prepare(&core.shared, "s", "s", "batch", batch).await;
            assert!(
                outcome.is_err(),
                "{mode} {action}: batch must refuse before asking"
            );
            let mut native = input;
            native["action"] = json!(action);
            assert!(
                builtin_card(&core.shared, &native).await.is_err(),
                "{mode} {action}"
            );
            assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
            assert!(seen.lock().unwrap().is_empty());
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn discovery_never_offers_a_card_for_secret_reads() {
    let core = core("browser-secret-discovery").await;
    for mode in ["ask", "accept-edits", "smart-approve", "auto-approve"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        let mut status = super::output_tests::status(&json!({}));
        for row in status["structuredContent"]["capabilities"]["actions"]
            .as_array_mut()
            .unwrap()
        {
            row["approvalRequired"] = json!(true);
        }
        check::annotate_modes(&core.shared, "s", "chrome_status", &mut status);
        let rows = status["structuredContent"]["capabilities"]["actions"]
            .as_array()
            .unwrap();
        for action in [
            "cookies.read",
            "storage.read",
            "clipboard.read",
            "network.body",
        ] {
            let row = rows.iter().find(|row| row["action"] == action).unwrap();
            assert_eq!(
                row["allowedInThisMode"],
                mode == "auto-approve",
                "{mode} {row}"
            );
            assert_ne!(row["approvalRequired"], true, "{mode} {row}");
            if mode != "auto-approve" {
                assert_eq!(row["needs"], Tier::FullAccess.required_label());
            }
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn secret_read_receipts_are_revoked_when_the_chat_mode_changes_during_the_call() {
    let core = core("browser-secret-receipt-mode-change").await;
    let weak = std::sync::Arc::downgrade(&core.shared);
    super::test_support::answering(&core.shared, move |_, _| {
        weak.upgrade().unwrap().access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"accept-edits"}),
        );
        super::output_tests::mcp(
            json!({"status":"completed","data":{"value":"late private value"}}),
        )
    });
    let mut differences = Vec::new();
    for (action, input, secret) in cases() {
        if !secret {
            continue;
        }
        core.shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let args = json!({"sessionId":"t","action":action,"args":input});
        if super::escalation::send(&core.shared, "s", "chrome_execute", args)
            .await
            .is_ok()
        {
            differences.push(action);
        }
    }
    core.shutdown().await;
    assert!(
        differences.is_empty(),
        "secret results released after losing {}: {differences:?}",
        Tier::FullAccess.label()
    );
}

#[tokio::test]
async fn site_secret_reads_require_full_access_in_every_browser_and_mode() {
    let core = core("browser-secret-mode-table").await;
    let mut differences = Vec::new();
    for browser in browsers() {
        if let Some(browser) = browser {
            select(&core.shared, browser);
        }
        for mode in ["ask", "accept-edits", "smart-approve", "auto-approve"] {
            core.shared.access.observe_request(
                "session/agent/update",
                "s",
                &json!({"agentName":mode}),
            );
            for (action, input, secret) in cases() {
                let expected = !secret || mode == "auto-approve";
                let args = json!({"sessionId":"t","action":action,"args":input});
                for approved in [false, true] {
                    let outcome = if browser.is_some() {
                        let consent = approved.then_some(read_consent::Consent(Tier::FullAccess));
                        check::authorize_read(
                            &core.shared,
                            "s",
                            "chrome_execute",
                            &args,
                            consent.as_ref(),
                        )
                        .map(|_| ())
                    } else {
                        let mut native = input.clone();
                        native["action"] = json!(action);
                        builtin::check_full(Some(mode), &native)
                    };
                    let row = format!("{browser:?} {mode} {action} {input} approved={approved}");
                    if outcome.is_ok() != expected {
                        differences.push(format!(
                            "{row}: expected allowed={expected}, got {outcome:?}"
                        ));
                    }
                    if !expected && let Err(reason) = outcome {
                        for words in [
                            Tier::FullAccess.label(),
                            "because",
                            "ask the person",
                            "switch this chat",
                        ] {
                            if !reason.contains(words) {
                                differences
                                    .push(format!("{row}: refusal lacks {words:?}: {reason}"));
                            }
                        }
                    }
                }
                if secret && read_consent::needed(&core.shared, "s", &args) {
                    differences.push(format!(
                        "{browser:?} {mode} {action}: secret read offers a card"
                    ));
                }
            }
        }
    }
    core.shutdown().await;
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn browser_read_tiers_are_cumulative_and_builtin_uses_the_shared_table() {
    let mut differences = Vec::new();
    for (action, input, _) in cases().into_iter().chain([
        ("auth.basic", json!({"login":"approved-login"}), false),
        ("cookies.change", json!({"operation":"clear"}), false),
        (
            "storage.change",
            json!({"area":"local","operation":"clear"}),
            false,
        ),
        ("clipboard.write", json!({"text":"note"}), false),
    ]) {
        let external = json!({"action":action,"args":input});
        let mut native = input;
        native["action"] = json!(action);
        for (mode, built, shared) in [
            (
                "ask",
                builtin::ask_required(&native),
                mapping::ask_required("chrome_execute", &external),
            ),
            (
                "accept-edits",
                builtin::required(&native),
                mapping::required("chrome_execute", &external),
            ),
        ] {
            if built != shared {
                differences.push(format!(
                    "{mode} {action}: built-in {built:?}, shared {shared:?}"
                ));
            }
        }
    }
    // All catalog reads allowed without a card in Ask must also run in each wider mode.
    for action in mapping::action_names() {
        for input in [json!({"key":"Tab"}), json!({"unredacted":true})] {
            let args = json!({"action":action,"args":input});
            if mapping::ask_required("chrome_execute", &args) == Some(Tier::Ask)
                && mapping::required("chrome_execute", &args).unwrap() > Tier::AcceptEdits
            {
                differences.push(format!(
                    "{action} {input}: runs in {} but refuses {}",
                    Tier::Ask.label(),
                    Tier::AcceptEdits.label()
                ));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
