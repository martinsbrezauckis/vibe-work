//! Session approvals cover disclosed browser actions in one chat across its browsers.
//! Uses the real grant matcher; no browser, transport, or native consent.
use super::grants::{Grants, Scopeable, scope_of};
use serde_json::{Value, json};

fn card(tool: &str, action: &str, tier: u8) -> Value {
    let input = if tool == "browser_act" {
        json!({"action":action,"text":"first"})
    } else {
        json!({"sessionId":"browser-a","action":action,"args":{"text":"first"}})
    };
    json!({"vwToolAsk":true,"vwBrowser":vw_vocabulary::texts::CHROME_NAME,"detail":{"kind":"approval","vwBrowserTier":tier,
        "effect":{"toolName":format!("mcp_vw.{tool}"),"input":input},"requiredPermissions":[]}})
}

fn batch_card(steps: &Value, tier: u8) -> Value {
    let mut entry = card("chrome_execute", "page.type", tier);
    entry["detail"]["effect"]["input"] = json!({"sessionId":"browser-b","steps":steps});
    entry
}

#[test]
fn browser_session_grants_cover_disclosed_actions_across_browsers_and_leases() {
    let grants = Grants::default();
    let first = card("chrome_execute", "page.type", 1);
    assert!(
        matches!(scope_of(&first), Scopeable::Grant { .. }),
        "a browser tool package must be scopeable"
    );
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    let mut next = card("chrome_execute", "files.upload", 1);
    next["detail"]["effect"]["input"]["args"] = json!({"paths":["another-file.txt"]});
    assert!(
        grants.matching("chat", &next).is_some(),
        "one yes covers the package's next step"
    );
    for (chat, entry) in [
        ("other", next.clone()),
        ("chat", card("chrome_execute", "raw.execute", 2)),
    ] {
        assert!(grants.matching(chat, &entry).is_none());
    }
    assert!(
        grants
            .matching("chat", &card("browser_act", "type", 1))
            .is_some()
    );
    next["detail"]["effect"]["input"]["sessionId"] = json!("browser-b");
    assert!(
        grants.matching("chat", &next).is_some(),
        "a chat grant follows its browser lease"
    );
    for browser in [
        vw_vocabulary::texts::EDGE_NAME,
        vw_vocabulary::texts::FIREFOX_NAME,
        vw_vocabulary::texts::SAFARI_NAME,
    ] {
        next["vwBrowser"] = json!(browser);
        assert!(
            grants.matching("chat", &next).is_some(),
            "the chat grant should cover {browser}"
        );
    }
}

#[test]
fn a_higher_browser_package_includes_disclosed_lower_tier_actions_across_tools() {
    let grants = Grants::default();
    let first = card("chrome_execute", "raw.execute", 2);
    assert!(matches!(scope_of(&first), Scopeable::Grant { .. }));
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    assert!(
        grants
            .matching("chat", &card("chrome_execute", "page.type", 1))
            .is_some()
    );
    assert!(
        grants
            .matching("chat", &card("browser_act", "type", 1))
            .is_some()
    );
}

#[test]
fn a_disclosed_write_grant_covers_a_batch_with_that_write_and_an_ordinary_read() {
    let grants = Grants::default();
    let first = card("chrome_execute", "page.type", 1);
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    let mut batch = batch_card(
        &json!([
            {"action": "page.type", "args": {"text": "next"}},
            {"action": "page.read", "args": {}}
        ]),
        1,
    );
    batch["vwBrowser"] = json!(vw_vocabulary::texts::EDGE_NAME);
    assert!(
        grants.matching("chat", &batch).is_some(),
        "a batch with an already approved write should reuse the chat grant"
    );
    assert!(grants.matching("other-chat", &batch).is_none());
}

#[test]
fn a_batch_grant_remembers_its_writes_for_a_later_browser() {
    let grants = Grants::default();
    let first = batch_card(
        &json!([
            {"action":"page.type","args":{}},
            {"action":"page.read","args":{}}
        ]),
        1,
    );
    assert!(matches!(scope_of(&first), Scopeable::Grant { .. }));
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    assert!(
        grants
            .matching("chat", &card("browser_act", "type", 1))
            .is_some()
    );
    assert!(
        grants
            .matching("other-chat", &card("browser_act", "type", 1))
            .is_none()
    );
}

#[test]
fn batch_grants_require_every_disclosed_write_and_a_valid_shape() {
    let grants = Grants::default();
    let first = card("browser_act", "type", 1);
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    let own_write = batch_card(
        &json!([
            {"action":"page.type","args":{}},
            {"action":"page.read","args":{}}
        ]),
        1,
    );
    assert!(grants.matching("chat", &own_write).is_some());
    for request in [
        batch_card(&json!([{"action":"page.insert_text","args":{}}]), 1),
        batch_card(
            &json!([{"action":"page.type","args":{}}, {"action":"raw.execute","args":{}}]),
            2,
        ),
        batch_card(&json!([{"action":"future.unknown","args":{}}]), 1),
        batch_card(&json!([{"action":"page.type","args":{},"extra":true}]), 1),
        batch_card(&json!([{"action":"page.type","args":null}]), 1),
        batch_card(&json!([{"action":"page.read","args":{}}]), 1),
    ] {
        assert!(grants.matching("chat", &request).is_none(), "{request}");
    }
    let mut unexpected_top_level = own_write;
    unexpected_top_level["detail"]["effect"]["input"]["action"] = json!("page.type");
    assert!(grants.matching("chat", &unexpected_top_level).is_none());
}

#[test]
fn a_one_time_write_does_not_transfer_to_a_batch() {
    let grants = Grants::default();
    grants.note_callback(
        "one",
        "chat",
        "chat",
        scope_of(&card("browser_act", "type", 1)),
    );
    grants.forget_callback("one");
    let batch = batch_card(&json!([{"action":"page.type","args":{}}]), 1);
    assert!(grants.matching("chat", &batch).is_none());
}

#[test]
fn connected_browser_grant_does_not_cover_undisclosed_builtin_action() {
    let grants = Grants::default();
    let first = card("chrome_execute", "page.type", 1);
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    assert!(
        grants
            .matching("chat", &card("browser_act", "files.save_download", 1))
            .is_none()
    );
}

#[test]
fn a_one_time_browser_answer_does_not_create_a_chat_grant() {
    let grants = Grants::default();
    let first = card("chrome_execute", "page.type", 1);
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants.forget_callback("one");
    assert!(
        grants
            .matching("chat", &card("browser_act", "type", 1))
            .is_none()
    );
}

#[test]
fn malformed_browser_requests_cannot_borrow_a_chat_grant() {
    let grants = Grants::default();
    let first = card("chrome_execute", "page.type", 1);
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    let held = grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    let mut no_lease = first.clone();
    no_lease["detail"]["effect"]["input"]["sessionId"] = json!("");
    assert_eq!(scope_of(&no_lease), Scopeable::No);
    assert!(grants.matching("chat", &no_lease).is_none());
    for tier in [0, 3] {
        let request = card("browser_act", "type", tier);
        assert_eq!(scope_of(&request), Scopeable::No);
        assert!(grants.matching("chat", &request).is_none());
    }
    let Scopeable::Grant {
        mut permissions, ..
    } = scope_of(&card("browser_act", "type", 1))
    else {
        panic!("scopeable typing");
    };
    let mut pattern: Value = serde_json::from_str(&permissions[0].session_pattern).unwrap();
    pattern["requested"] = json!("");
    permissions[0].session_pattern = pattern.to_string();
    assert!(!super::browser_package::covers(
        &held,
        "chat",
        "mcp_vw.browser_act",
        &permissions
    ));
}

#[test]
fn builtin_typing_grant_discloses_dialogs_before_covering_them() {
    let grants = Grants::default();
    let first = card("browser_act", "type", 1);
    let Scopeable::Grant { labels, covers, .. } = scope_of(&first) else {
        panic!("scopeable typing");
    };
    for label in [
        "Accept a dialog in a browser this chat uses",
        "Type into a dialog in a browser this chat uses",
        "Save a downloaded file in a browser this chat uses",
    ] {
        assert!(
            labels.iter().any(|text| text == label),
            "package omitted: {label}; {covers:?}"
        );
    }
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    for action in ["page.dialog.accept", "page.dialog.prompt"] {
        assert!(
            grants
                .matching("chat", &card("browser_act", action, 1))
                .is_some()
        );
    }
    assert!(
        grants
            .matching("chat", &card("chrome_execute", "page.type", 1))
            .is_some()
    );
}

#[test]
fn undescribed_browser_actions_cannot_get_or_reuse_a_session_grant() {
    let grants = Grants::default();
    let first = card("browser_act", "type", 1);
    grants.note_callback("one", "chat", "chat", scope_of(&first));
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    for tool in ["browser_act", "chrome_execute"] {
        let unknown = card(tool, "future.undescribed", 1);
        assert_eq!(scope_of(&unknown), Scopeable::No);
        assert!(grants.matching("chat", &unknown).is_none());
    }
}

#[test]
fn an_unknown_action_without_a_tier_cannot_offer_the_whole_browser_tool() {
    let mut unknown = card("browser_act", "future.undescribed", 1);
    unknown["detail"]
        .as_object_mut()
        .unwrap()
        .remove("vwBrowserTier");
    assert_eq!(scope_of(&unknown), Scopeable::No);
}

#[test]
fn builtin_package_discloses_every_executable_action_at_its_tier() {
    use crate::chrome_control::builtin;
    use vw_ask::chrome::Tier;
    for (tier, example) in [
        (Tier::AcceptEdits, "type"),
        (Tier::FullAccess, "clipboard.write"),
    ] {
        let Scopeable::Grant {
            labels,
            permissions,
            ..
        } = scope_of(&card("browser_act", example, tier as u8))
        else {
            panic!("scopeable package");
        };
        let covered: Value = serde_json::from_str(&permissions[0].session_pattern).unwrap();
        for (wire, action) in vw_agent_tools::browser_input::actions() {
            let needed = builtin::required(&json!({"action":wire}))
                .expect("every executable action has a tier");
            if needed > Tier::Ask && needed <= tier {
                assert!(
                    covered["actions"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(action)),
                    "package omitted {wire}"
                );
            }
        }
        assert_eq!(labels.len(), covered["actions"].as_array().unwrap().len());
        assert!(
            labels.iter().all(|label| !label.contains('{')),
            "unfilled label: {labels:?}"
        );
    }
}

#[test]
fn an_older_incomplete_package_cannot_silently_cover_new_actions() {
    let grants = Grants::default();
    let mut first = scope_of(&card("browser_act", "type", 1));
    let Scopeable::Grant { permissions, .. } = &mut first else {
        panic!("scopeable typing");
    };
    let mut pattern: Value = serde_json::from_str(&permissions[0].session_pattern).unwrap();
    pattern["actions"]
        .as_array_mut()
        .unwrap()
        .retain(|action| action != "page.dialog.accept");
    permissions[0].session_pattern = pattern.to_string();
    grants.note_callback("one", "chat", "chat", first);
    grants
        .keep(grants.claim("one").unwrap(), vw_ask::AnsweredBy::Person, 1)
        .unwrap();
    assert!(
        grants
            .matching("chat", &card("browser_act", "page.dialog.accept", 1))
            .is_none()
    );
}
