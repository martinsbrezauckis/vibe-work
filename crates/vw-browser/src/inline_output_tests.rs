//! Budget edge cases for bounded browser models: safety metadata, URLs, large selects and image blocks.
//! Runs without a window; exercises the same final fitter as agent reads and receipts.
#![allow(clippy::unwrap_used)]

use crate::observe;
use serde_json::{Value, json};

fn safety() -> Value {
    json!({
        "snapshot":"0123456789abcdef", "ok":true,"action":"click","Outcome":"a field or control changed state",
        "guardLayers":{"network":true,"frames":true},
        "promptGuard":{"flagged":true,"signals":[{"channel":"hiddenText","signal":"instructionOverride","confidence":"high"}]},
        "note":observe::UNTRUSTED_NOTE,
        "untrustedPageContent":{}
    })
}

#[test]
fn long_navigation_addresses_cannot_escape_the_whole_result_budget() {
    let mut out = safety();
    let address = format!("https://example.test/{}", "long-path/".repeat(200));
    out["untrustedPageContent"] = json!({"url":address,"changed":{"url":{"from":address,"to":address}}, "text":"report ".repeat(2000)});
    let before = out.clone();
    observe::fit(&mut out, 64_000);
    assert!(
        observe::size(&out) <= 4000,
        "remaining bytes: {}",
        observe::size(&out)
    );
    for key in [
        "snapshot",
        "ok",
        "action",
        "Outcome",
        "guardLayers",
        "promptGuard",
        "note",
    ] {
        assert_eq!(out[key], before[key], "{key}");
    }
    assert_eq!(out["untrustedPageContent"]["changed"]["urlOmitted"], true);
}

#[test]
fn many_tabs_fit_and_keep_a_matching_inventory_and_cursor() {
    let mut out =
        json!({"action":"list_tabs","ok":true,"tabs":[],"untrustedPageContent":{"tabs":[]}});
    for i in 0..100 {
        out["tabs"].as_array_mut().unwrap().push(json!({"id":format!("tab-{i}"),"site":"example.test","owner":"agent","active":false,"suspended":false}));
        out["untrustedPageContent"]["tabs"].as_array_mut().unwrap().push(json!({"id":format!("tab-{i}"),"title":"news ".repeat(24),"url":format!("https://example.test/{}", "article/".repeat(200))}));
    }
    observe::fit(&mut out, 4000);
    assert!(observe::size(&out) <= 4000, "{}", observe::size(&out));
    let tabs = out["tabs"].as_array().unwrap();
    assert_ne!(tabs.as_slice(), &[] as &[Value]);
    assert_eq!(out["nextTab"], tabs.last().unwrap()["id"]);
    assert_eq!(
        tabs.len(),
        out["untrustedPageContent"]["tabs"]
            .as_array()
            .unwrap()
            .len()
    );
}

#[test]
fn omitted_tab_details_do_not_suggest_a_missing_cursor() {
    let mut out = json!({"action":"list_tabs","ok":true,"tabs":[{"id":"first"},{"id":"second"}],
        "untrustedPageContent":{"tabs":[{"id":"first","url":format!("https://example.test/{}", "path/".repeat(390))},
            {"id":"second","url":format!("https://example.test/{}", "path/".repeat(390))}]}});
    observe::fit(&mut out, 4000);
    assert!(out.get("nextTab").is_none());
    assert!(!out["trimmed"]["how"].as_str().unwrap().contains("nextTab"));
    assert_eq!(out["tabs"].as_array().unwrap().len(), 2);
}

#[test]
fn upload_receipt_keeps_all_file_outcomes_within_the_budget() {
    let mut out = safety();
    out["result"] = json!({"action":"upload","files":(0..16).map(|i| json!({"name":format!("report-{}-{i}.txt", "é".repeat(100)),"size":i+1})).collect::<Vec<_>>()});
    observe::fit(&mut out, 3000);
    assert!(
        observe::size(&out) <= 3000,
        "upload receipt: {} bytes",
        observe::size(&out)
    );
    let files = out["result"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 16);
    for (i, file) in files.iter().enumerate() {
        assert_eq!(file["size"], i + 1);
    }
    assert_eq!(out["result"]["fileNamesOmitted"], true);
    assert_eq!(out["ok"], true);
}

#[test]
fn long_browser_errors_keep_the_diagnosis_and_recovery_inline() {
    let mut out = json!(format!(
        "Outcome unknown: https://example.test/{}. Do not repeat the action; use browser_read to check the current page.",
        "long-path/".repeat(2000)
    ));
    crate::filter_agent_output(&mut out);
    assert!(
        observe::size(&out) <= 4000,
        "error bytes: {}",
        observe::size(&out)
    );
    let text = out.as_str().unwrap();
    assert!(text.starts_with("Outcome unknown:"));
    assert!(
        text.ends_with("Do not repeat the action; use browser_read to check the current page.")
    );
    assert!(text.contains("omitted"));
}

#[test]
fn closed_tab_warnings_fit_without_losing_the_closed_outcome() {
    let mut out = json!({"action":"close_tab","ok":true,"tab":{"id":"owned-tab","site":"example.test"},"result":"closed",
        "warning":format!("tab closed, but replacement page could not be shown: {}; the tab is already closed, so do not repeat the close", "details ".repeat(2000))});
    observe::fit(&mut out, observe::ACT_BUDGET);
    crate::filter_agent_output(&mut out);
    assert!(
        observe::size(&out) <= 4000,
        "close bytes: {}",
        observe::size(&out)
    );
    assert_eq!(out["ok"], true);
    assert_eq!(out["result"], "closed");
    assert_eq!(out["tab"]["id"], "owned-tab");
    let warning = out["warning"].as_str().unwrap();
    assert!(warning.starts_with("tab closed, but replacement page could not be shown:"));
    assert!(warning.ends_with("the tab is already closed, so do not repeat the close"));
}

#[test]
fn controls_keep_identity_and_images_survive_the_text_budget() {
    let mut out = safety();
    out["vwImage"] = json!({"mimeType":"image/png","data":"pixels".repeat(10_000)});
    out["untrustedPageContent"] = json!({"refs":[{"ref":"e12345678","role":"combobox","name":"Select city","frameOrigin":"https://frame.example.test","options":vec!["Long city name ".repeat(10);20]}],"text":"article ".repeat(500)});
    let image = out["vwImage"].clone();
    observe::fit(&mut out, 3000);
    assert_eq!(out["vwImage"], image);
    out.as_object_mut().unwrap().remove("vwImage");
    assert!(observe::size(&out) <= 3000);
    let control = &out["untrustedPageContent"]["refs"][0];
    assert_eq!(control["ref"], "e12345678");
    assert_eq!(control["frameOrigin"], "https://frame.example.test");
    assert_eq!(out["size"]["bytes"], observe::size(&out));
    let once = out.clone();
    observe::fit(&mut out, 3000);
    assert_eq!(out, once, "the final boundary must be idempotent");
}

#[test]
fn final_browser_scrub_cannot_expand_past_the_inline_budget() {
    let mut out = safety();
    out["untrustedPageContent"] =
        json!({"title":"API key documentation", "text":"1234567890123456 ".repeat(175), "refs":[]});
    observe::fit(&mut out, 4000);
    crate::filter_agent_output(&mut out);
    assert!(
        observe::size(&out) <= 4000,
        "post-scrub bytes: {}",
        observe::size(&out)
    );
    assert_eq!(out["snapshot"], "0123456789abcdef");
    assert_eq!(out["size"]["bytes"], observe::size(&out));
}

#[test]
fn long_text_and_links_do_not_displace_controls_that_fit() {
    let mut out = safety();
    let refs:Vec<_> = (0..42).map(|i| json!({"ref":format!("e{i:08x}"),"role":if i < 12 {"button"} else {"link"},"name":format!("Control {i}")})).collect();
    out["untrustedPageContent"] =
        json!({"refs":refs,"text":"Long article paragraph. ".repeat(150)});
    observe::fit(&mut out, 4000);
    let controls = out["untrustedPageContent"]["refs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["role"] == "button")
        .count();
    assert_eq!(controls, 12, "text should shrink before useful controls");
}
