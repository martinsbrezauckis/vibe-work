//! Shield modes compile to native rules; Strict is the downloaded lists plus every Balanced rule.
//! Depends on the preview filter compiler; native enforcement is covered by the `WebKitGTK` fixture.
#![allow(clippy::unwrap_used)]
use vw_preview::shields::{Mode, compile};

#[test]
fn off_is_empty_and_balanced_does_not_need_a_download() {
    assert_eq!(compile(Mode::Off, None).unwrap(), "[]");
    let balanced = compile(Mode::Balanced, None).unwrap();
    assert!(balanced.contains("doubleclick"));
    assert!(balanced.contains("css-display-none"));
    assert!(compile(Mode::Strict, None).is_err());
}

#[test]
fn strict_preserves_exceptions_and_cosmetic_rules() {
    let compiled = compile(Mode::Strict, Some("||ads.example.test^\n@@||ads.example.test/keep.js$script\nexample.test##.advertisement\n")).unwrap();
    let rules: serde_json::Value = serde_json::from_str(&compiled).unwrap();
    let rules = rules.as_array().unwrap();
    let block = rules
        .iter()
        .position(|rule| rule["action"]["type"] == "block")
        .unwrap();
    let allow = rules
        .iter()
        .position(|rule| rule["action"]["type"] == "ignore-previous-rules")
        .unwrap();
    assert!(allow > block);
    assert!(
        rules
            .iter()
            .any(|rule| rule["action"]["selector"] == ".advertisement")
    );
}

#[test]
fn strict_includes_every_balanced_rule_after_the_lists() {
    // A list exception for the ad slot's page must not lift Balanced's own hiding or blocking in Strict.
    let lists = "||ads.example.test^\n@@||example.test^$document\nexample.test##.advertisement\n";
    let strict: Vec<serde_json::Value> =
        serde_json::from_str(&compile(Mode::Strict, Some(lists)).unwrap()).unwrap();
    let balanced: Vec<serde_json::Value> =
        serde_json::from_str(&compile(Mode::Balanced, None).unwrap()).unwrap();
    let last_exception = strict
        .iter()
        .rposition(|rule| rule["action"]["type"] == "ignore-previous-rules");
    for rule in &balanced {
        let at = strict
            .iter()
            .position(|r| r == rule)
            .unwrap_or_else(|| panic!("Strict lacks the Balanced rule {rule}"));
        assert!(
            last_exception.is_none_or(|exception| at > exception),
            "the Balanced rule {rule} comes before a list exception that would lift it"
        );
    }
    assert!(
        strict
            .iter()
            .any(|rule| rule["action"]["selector"] == ".advertisement")
    );
}
