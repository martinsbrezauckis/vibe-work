//! Session approvals cover a browser tool's tiered package, never an argument or another tool.
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

#[test]
fn browser_session_grants_keep_the_tool_and_package_instead_of_the_exact_step() {
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
        ("chat", card("browser_act", "type", 1)),
    ] {
        assert!(grants.matching(chat, &entry).is_none());
    }
    next["detail"]["effect"]["input"]["sessionId"] = json!("browser-b");
    assert!(
        grants.matching("chat", &next).is_none(),
        "a browser lease is not another browser"
    );
}

#[test]
fn a_higher_browser_package_includes_the_lower_tier_but_keeps_the_tool_fence() {
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
            .is_none()
    );
}

#[test]
fn builtin_typing_grant_discloses_dialogs_before_covering_them() {
    let grants = Grants::default();
    let first = card("browser_act", "type", 1);
    let Scopeable::Grant { labels, covers, .. } = scope_of(&first) else {
        panic!("scopeable typing");
    };
    for label in [
        "Accept a dialog in the built-in browser",
        "Type into a dialog in the built-in browser",
        "Save a downloaded file in the built-in browser",
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
