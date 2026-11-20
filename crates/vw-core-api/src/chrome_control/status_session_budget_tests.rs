//! `chrome_status` over its return budget still lists this chat's live browser session.
//! The session is opened through the gateway (`chrome_session open`), then read back with `chrome_status` in the same
//! chat; the fake helper answers with a full helper task row and a status whose capability tables exceed the budget.
//!
//! PREDICT (written before the fix): `output::guard_status` first trims everything except the capability tables to
//! the budget left after those tables. When the tables alone exceed the budget, that limit is zero, so the largest
//! array, the one-row `sessions`, is halved to `[]`; the tables are then omitted anyway and the empty list stays.
//! The chat's active session therefore reads as `"sessions": []` while `chrome_session get` says "active".
use super::output_tests::status;
use super::{gateway, test_support};
use serde_json::{Value, json};

const OPENED: &str = "session-1f254ba6-ba04-4f6f-b682-3c56578bafd8";

/// The helper's task view for an active chat session with a working frame and one owned tab.
fn task_row(profile: &str) -> Value {
    json!({"sessionId":OPENED,"clientId":"vibework-owner","clientName":"Vibe Work","purpose":"Vibe Work chat",
        "profileId":profile,"state":"active","desiredState":"active","leaseEpoch":2,"policyRevision":1,
        "target":{"active":true,"documentId":"D35745D2C1BD745A4594C734BD902826","origin":"http://127.0.0.1:8777",
            "tabId":916_838_055,"title":"PRE15 keys","url":"http://127.0.0.1:8777/keys.html","windowId":916_838_054},
        "windowId":916_838_054,"scope":{"tabIds":[916_838_055]},"capture":"off",
        "lastConfirmed":{"at":9_836_027,"state":"active"},"groupTabs":true,
        "clientSessionId":"b1d12838-0d98-cfef-dcb4-59569279151c","unresolved":[],"suspensionReason":null,
        "popups":{"closedCount":0,"complete":true,"openCount":0,"trackingTruncated":false},
        "frame":{"colour":0,"label":"vw:chrome batch status probe","revision":2,"state":"working"},
        "personPause":null,"browser":"Google Chrome"})
}

/// A status whose per-profile capability lists cannot fit the return budget, listing this chat's opened session and
/// another client's session that this chat must never see.
fn crowded_status(args: &Value, profile: &str) -> Value {
    let mut result = status(args);
    let names: Vec<Value> = (0..512)
        .map(|i| json!(format!("cap{i:03}{}", "x".repeat(122))))
        .collect();
    for row in result["structuredContent"]["profiles"]
        .as_array_mut()
        .unwrap()
    {
        row["extensionCapabilities"] = json!(names);
    }
    result["structuredContent"]["sessions"] = json!([task_row(profile),
        {"sessionId":"session-other-chat","profileId":profile,"state":"active","purpose":"Another chat"}]);
    result
}

async fn open_then_status(label: &str, crowded: bool) -> Value {
    let core = super::output_tests::core(label).await;
    let shared = &core.shared;
    // The fixture's own `t`/`t2` leases are not part of this chat's opened session.
    shared.chrome_control.tasks.lock().unwrap().clear();
    let profile = "chrome:profile-0";
    // As on the reporting machine: the chat uses the person's default Chrome profile, without a card.
    shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .defaults
        .insert("chrome".into(), profile.into());
    test_support::answering(shared, move |tool, args| {
        if tool == "chrome_session" && args["action"] == "open" {
            json!({"isError":false,"content":[{"type":"text","text":"opened"}],
                "structuredContent":task_row(profile)})
        } else if crowded {
            crowded_status(args, profile)
        } else {
            let mut result = status(args);
            result["structuredContent"]["sessions"] = json!([task_row(profile)]);
            result
        }
    });
    let opened = gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
    .unwrap();
    assert_eq!(
        opened["vwMcpResult"]["structuredContent"]["sessionId"], OPENED,
        "{opened}"
    );
    let answer = gateway::call(shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    core.shutdown().await;
    answer
}

fn listed(answer: &Value) -> Vec<Value> {
    answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"]["sessions"]
        .as_array()
        .unwrap_or_else(|| panic!("no sessions list: {answer}"))
        .clone()
}

#[tokio::test]
async fn status_within_budget_lists_the_opened_session() {
    let answer = open_then_status("chrome-status-session-fits", false).await;
    let rows = listed(&answer);
    assert_eq!(rows.len(), 1, "{answer}");
    assert_eq!(rows[0]["sessionId"], OPENED);
    assert_eq!(rows[0]["state"], "active");
}

#[tokio::test]
async fn over_budget_status_keeps_the_chats_active_session() {
    let answer = open_then_status("chrome-status-session-crowded", true).await;
    let body = &answer["vwMcpResult"]["structuredContent"];
    assert!(
        body["untrustedPageContent"]["capabilities"]["availability"]
            .as_str()
            .is_some_and(|text| text.contains("omitted")),
        "the fixture must exceed the status budget: {body}"
    );
    let rows = listed(&answer);
    assert_eq!(rows.len(), 1, "this chat's session only: {body}");
    assert_eq!(rows[0]["sessionId"], OPENED);
    assert_eq!(rows[0]["state"], "active");
    assert_eq!(rows[0]["profileId"], "chrome:profile-0");
    assert!(
        !body.to_string().contains("session-other-chat"),
        "another chat's session is never listed: {body}"
    );
    assert!(body.to_string().len() <= 9000, "status exceeded its budget");
    assert!(
        body["outputLimit"]
            .as_str()
            .is_some_and(|note| note.contains("9000")),
        "the capped status says so: {body}"
    );
}

/// PREDICT (written before session rows were kept whole): with so many connected profiles that the kept profile table
/// alone fills the budget, the trim limit is zero even after the capability tables are omitted, so the one-row
/// `sessions` array is halved to `[]` again. The row must stay, with its session id, state and profile.
#[tokio::test]
async fn a_status_crowded_by_profiles_keeps_the_session_row() {
    let core = super::output_tests::core("chrome-status-session-profiles").await;
    let shared = &core.shared;
    shared.chrome_control.tasks.lock().unwrap().clear();
    let profile = "chrome:profile-0";
    shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .defaults
        .insert("chrome".into(), profile.into());
    test_support::answering(shared, move |tool, args| {
        if tool == "chrome_session" && args["action"] == "open" {
            return json!({"isError":false,"content":[{"type":"text","text":"opened"}],
                "structuredContent":task_row(profile)});
        }
        let mut result = crowded_status(args, profile);
        let rows = result["structuredContent"]["profiles"]
            .as_array_mut()
            .unwrap();
        let template = rows[1].clone();
        for i in 2..120 {
            let mut row = template.clone();
            row["profileId"] = json!(format!("chrome:profile-{i}"));
            row["name"] = json!(format!("Profile {i}"));
            rows.push(row);
        }
        result
    });
    gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
    .unwrap();
    let answer = gateway::call(shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    core.shutdown().await;
    let rows = listed(&answer);
    assert_eq!(rows.len(), 1, "{answer}");
    assert_eq!(rows[0]["sessionId"], OPENED);
    assert_eq!(rows[0]["state"], "active");
    assert_eq!(rows[0]["profileId"], profile);
}
