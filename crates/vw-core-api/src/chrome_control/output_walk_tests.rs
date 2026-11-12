//! Browser-walk regressions for mistaken action names and combined click/read output.
//! Uses the gateway, shipped-helper-shaped observations and current catalog; no GUI or helper writes.
use super::{content, gateway, output, output_tests, owner, test_support};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const PROFILE: &str = if cfg!(feature = "your-edge") {
    "edge:profile-0"
} else {
    "chrome:profile-0"
};

fn groups() -> BTreeMap<String, Vec<String>> {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    let catalog = output_tests::catalog();
    // The frozen fixture contains extension actions; the helper also composes or adapts these.
    let actions = catalog["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .chain([
            "page.add_style",
            "console.clear",
            "page.inspect",
            "page.errors.clear",
        ]);
    for action in actions {
        let (group, name) = action.split_once('.').unwrap();
        groups.entry(group.into()).or_default().push(name.into());
    }
    for action in super::page_plan_catalog::NAMES
        .iter()
        .chain(super::extension_state::NAMES)
        .chain(super::page_capture::NAMES)
    {
        let (group, name) = action.split_once('.').unwrap();
        groups.entry(group.into()).or_default().push(name.into());
    }
    for names in groups.values_mut() {
        names.sort();
    }
    groups
}

#[tokio::test]
async fn unknown_actions_return_only_grouped_correction() {
    let core = output_tests::core("chrome-output-unknown-actions").await;
    let seen = test_support::answering(&core.shared, |_, _| {
        let mut result = output_tests::status(&json!({}));
        result["structuredContent"]["error"] =
            json!({"code":"UNKNOWN_ACTION","message":"Unsupported action"});
        result
    });
    let mut failures = Vec::new();
    for action in [
        "chrome_session",
        "chrome_execute",
        "chrome_status",
        "not.an.action",
    ] {
        let answer = gateway::call(&core.shared, "s", "chrome_status", json!({"action":action}))
            .await
            .unwrap();
        let result = &answer["vwMcpResult"];
        let text = result["content"][0]["text"].as_str().unwrap();
        let body = &result["structuredContent"];
        let message = body["error"]["message"].as_str().unwrap_or_default();
        let compact = body.to_string().chars().count();
        let pretty = serde_json::to_string_pretty(body).unwrap().chars().count();
        println!(
            "SIZE unknown {action}: compact={compact}, pretty={pretty}, text={}",
            text.chars().count()
        );
        let complete = groups().iter().all(|(group, names)| {
            message
                .lines()
                .any(|line| line == format!("{group}: {}", names.join(",")))
        });
        // The correction is the full list of valid action names (all needed facts); it grows with each
        // browser action, so the bound leaves room for the merged 0.1.10 set (1212 compact) and a little more.
        if compact > 1400
            || text.chars().count() > 1200
            || body["error"]["code"] != "UNKNOWN_ACTION"
            || !complete
            || !message.contains(
                "tool names such as chrome_session are not actions; call that tool directly",
            )
            || body.as_object().unwrap().len() != 1
        {
            failures.push(action);
        }
    }
    core.shutdown().await;
    assert!(
        failures.is_empty(),
        "missing bounded correction: {failures:?}"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        0,
        "invalid names need no helper status"
    );
}

fn receipt(action: &str) -> Value {
    let mut body = output_tests::page()["structuredContent"].clone();
    body["profileId"] = json!(PROFILE);
    body["action"] = json!(action);
    body["operationId"] = json!(if action == "page.read" {
        "op-read"
    } else {
        "op-click"
    });
    if action != "page.read" {
        body["capability"] = json!("page.interact");
        let refs: Vec<_> = (1..=31)
            .map(|day| {
                json!({"ref":format!("e{day:08x}"),"role":"option","tag":"div",
            "name":format!("Choose day {day} from the open date picker")})
            })
            .collect();
        body["data"] = json!({"effect":"page_changed","valuePresent":true,"observedForMs":350,
            "url":"https://demoqa.com/automation-practice-form","title":"Practice Form",
            "page":{"tabId":7,"documentId":"document-a","url":"https://demoqa.com/automation-practice-form",
            "delta":true,"text":"","refs":refs,"removedRefs":[]}});
    }
    if action == "files.upload" {
        let files = json!([{"name":"report.txt","size":8}]);
        body["data"]["files"] = files.clone();
        body["data"]["currentFiles"] = files;
        body["data"]["resultObserved"] = json!(true);
    }
    let text_path = if action == "page.read" {
        "/data/text"
    } else {
        "/data/page/text"
    };
    *body.pointer_mut(text_path).unwrap() = json!("");
    let mut marked = output_tests::mcp(body.clone());
    content::mark(&mut marked);
    let target = if action == "page.read" { 13_354 } else { 4505 };
    let remaining = target - marked["structuredContent"].to_string().len();
    if action != "page.read" {
        assert!(
            remaining <= 800,
            "delta text is capped at 800 by the helper"
        );
    }
    *body.pointer_mut(text_path).unwrap() = json!(
        "Date picker and practice form. "
            .repeat(500)
            .chars()
            .take(remaining)
            .collect::<String>()
    );
    if action == "page.read" {
        body["data"]["nextTextOffset"] = json!(remaining);
    }
    output_tests::mcp(body)
}

#[test]
fn every_action_observation_uses_the_small_receipt_budget() {
    let mut failures = Vec::new();
    for action in output_tests::catalog()["actions"].as_array().unwrap() {
        let id = action["id"].as_str().unwrap();
        let result = output::for_model(owner::Answer::Helper(receipt(id)), "chrome_execute", false);
        let (compact, pretty) = output_tests::sizes(&result);
        if matches!(id, "page.click" | "page.read") {
            println!("SIZE {id} receipt: compact={compact}, pretty={pretty}");
        }
        if compact > 4000 || pretty > 6000 {
            failures.push((id.to_owned(), compact, pretty));
        }
        let body = &result["vwMcpResult"]["structuredContent"];
        assert_eq!(body["action"], id);
        assert_eq!(body["status"], "completed");
        assert!(body["untrustedPageContent"].is_object());
        if id != "page.read" {
            assert_eq!(
                body["untrustedPageContent"]["data"]["effect"],
                "page_changed"
            );
        }
    }
    assert!(failures.is_empty(), "receipt budgets: {failures:?}");
}

async fn fixture() -> crate::Core {
    let core = output_tests::core("chrome-output-click-read").await;
    test_support::answering(&core.shared, |tool, args| {
        match tool {
            "chrome_status" => {
                let mut result = output_tests::status(args);
                let status = &mut result["structuredContent"];
                status["profiles"][0]["profileId"] = json!(PROFILE);
                status["profiles"][0]["browser"] = json!(if cfg!(feature = "your-edge") {
                    "edge"
                } else {
                    "chrome"
                });
                status["sessions"][0]["profileId"] = json!(PROFILE);
                // Acknowledge the startup policy retry, which has no action argument.
                status["revision"] = json!(3);
                output_tests::mcp(status.take())
            }
            "chrome_execute" => receipt(args["action"].as_str().unwrap()),
            _ => panic!("unexpected helper call: {tool} {args}"),
        }
    });
    if cfg!(feature = "your-edge") {
        *core
            .shared
            .chrome_control
            .owner
            .edge_pairing
            .lock()
            .unwrap() = (crate::chrome_control::PairingState::Paired, None);
        core.shared
            .chrome_control
            .chat_profiles
            .lock()
            .unwrap()
            .insert("s".into(), PROFILE.into());
        core.shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .open("t", "s", "active", PROFILE, None);
        core.shared
            .chrome_control
            .owner
            .connected_profiles
            .observe(&json!({"profiles":[{
            "profileId":PROFILE,"connected":true,"extensionVersion":"0.1.6"}]}));
    }
    core
}

#[tokio::test]
async fn click_and_read_together_stay_below_the_code_mode_limit() {
    let core = fixture().await;
    // Startup can retry this status request between actions; exercise it without depending on elapsed time.
    super::send_local_network(&core.shared).await;
    assert!(core.shared.chrome_control.local_network_applied());
    let mut failures = Vec::new();
    for args in [json!({}), json!({"maxChars":60000,"maxBytes":64000})] {
        let click = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"page.click","args":{"ref":"e00000001"}}),
        )
        .await
        .unwrap();
        let read = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"page.read","args":args}),
        )
        .await
        .unwrap();
        let result = json!({"click":click["vwMcpResult"]["structuredContent"],"read":read["vwMcpResult"]["structuredContent"]});
        let pretty = serde_json::to_string_pretty(&result)
            .unwrap()
            .chars()
            .count();
        println!(
            "SIZE combined click + read {args}: compact={}, pretty={pretty}",
            result.to_string().chars().count()
        );
        if pretty >= 16_000 {
            failures.push(pretty);
        }
        for answer in [&click, &read] {
            let body = &answer["vwMcpResult"]["structuredContent"];
            assert_eq!(body["status"], "completed");
            assert_eq!(body["profileId"], PROFILE);
        }
    }
    core.shutdown().await;
    assert!(
        failures.is_empty(),
        "combined returns spilled: {failures:?}"
    );
}

#[test]
fn trimmed_click_deltas_request_a_narrow_read_without_inventing_offsets() {
    let answer = output::for_model(
        owner::Answer::Helper(receipt("page.click")),
        "chrome_execute",
        false,
    );
    let body = &answer["vwMcpResult"]["structuredContent"];
    let page = &body["untrustedPageContent"]["data"]["page"];
    assert_eq!(page["documentId"], "document-a");
    assert!(page.get("nextOffset").is_none());
    assert!(page.get("nextTextOffset").is_none());
    let note = body["outputLimit"].as_str().unwrap_or_default();
    assert!(
        note.contains("absent")
            && note.contains("find/ref/frameId")
            && note.contains("do not replay writes"),
        "{note}"
    );
}
