//! Reference provenance per browser ([`super`]).
//!
//! PREDICT (written before the change): nothing recorded where a reference came from, so a built-in action named a
//! reference from the person's Chrome and went to the built-in page (where the same string can name another
//! element). Each test below fails without the record: the refusal cases run, the allowed cases have no record.

use serde_json::json;

use super::{RefOrigins, refs_in};
use crate::chrome_control::Browser;

fn read(refs: &[&str]) -> std::collections::BTreeSet<String> {
    refs.iter().map(|r| (*r).to_owned()).collect()
}

fn click(r: &str) -> serde_json::Value {
    json!({"action":"click","ref":r})
}

#[test]
fn refs_are_found_in_answers_and_arguments() {
    let answer = json!({"vwMcpResult":{"structuredContent":{"untrustedPageContent":{"data":{"refs":[
        {"ref":"e00000002","role":"button"},{"ref":"e00000003"}]}}}}});
    assert_eq!(refs_in(&answer), read(&["e00000002", "e00000003"]));
    let batch =
        json!({"steps":[{"action":"click","ref":"e1"},{"action":"type","ref":"e2","text":"x"}]});
    assert_eq!(refs_in(&batch), read(&["e1", "e2"]));
    assert!(refs_in(&json!({"action":"scroll"})).is_empty());
}

#[test]
fn a_reference_only_your_chrome_returned_is_refused_on_the_built_in_browser() {
    let origins = RefOrigins::default();
    origins.note("chat", Some(Browser::Chrome), read(&["e00000002"]));
    let refused = origins
        .check_builtin("chat", &click("e00000002"))
        .unwrap_err();
    assert!(refused.contains("e00000002"), "{refused}");
    assert!(
        refused.contains(Browser::Chrome.name()),
        "names the browser: {refused}"
    );
    assert!(refused.contains("chrome_execute"), "{refused}");
    // Batch steps are checked too.
    let steps = json!({"steps":[{"action":"click","ref":"e00000002"}]});
    assert!(origins.check_builtin("chat", &steps).is_err());
}

#[test]
fn a_shared_reference_follows_the_browser_read_last() {
    let origins = RefOrigins::default();
    origins.note("chat", None, read(&["e00000002"]));
    origins.note("chat", Some(Browser::Edge), read(&["e00000002"]));
    let refused = origins
        .check_builtin("chat", &click("e00000002"))
        .unwrap_err();
    assert!(refused.contains(Browser::Edge.name()), "{refused}");
    // Reading the built-in page again makes its reference the current one.
    origins.note("chat", None, read(&["e00000002"]));
    assert_eq!(origins.check_builtin("chat", &click("e00000002")), Ok(()));
}

#[test]
fn ordinary_built_in_use_is_unchanged() {
    let origins = RefOrigins::default();
    // No read at all, or only built-in reads: nothing is refused here.
    assert_eq!(origins.check_builtin("chat", &click("e00000009")), Ok(()));
    origins.note("chat", None, read(&["e00000001"]));
    assert_eq!(origins.check_builtin("chat", &click("e00000001")), Ok(()));
    // A Chrome read that did not return this reference does not touch it.
    origins.note("chat", Some(Browser::Chrome), read(&["e00000005"]));
    assert_eq!(origins.check_builtin("chat", &click("e00000001")), Ok(()));
    assert_eq!(
        origins.check_builtin("chat", &json!({"action":"scroll"})),
        Ok(())
    );
}

#[test]
fn a_newer_read_replaces_the_older_references_of_that_browser() {
    let origins = RefOrigins::default();
    origins.note("chat", Some(Browser::Chrome), read(&["e00000002"]));
    // After navigation Chrome's next read returns other references: the old one is no longer Chrome's.
    origins.note("chat", Some(Browser::Chrome), read(&["e00000007"]));
    assert_eq!(origins.check_builtin("chat", &click("e00000002")), Ok(()));
    assert!(origins.check_builtin("chat", &click("e00000007")).is_err());
    // A read that returned no references changes nothing.
    origins.note("chat", Some(Browser::Chrome), read(&[]));
    assert!(origins.check_builtin("chat", &click("e00000007")).is_err());
}

#[test]
fn another_chats_reads_never_count() {
    let origins = RefOrigins::default();
    origins.note("first", Some(Browser::Chrome), read(&["e00000002"]));
    assert_eq!(origins.check_builtin("second", &click("e00000002")), Ok(()));
    assert!(origins.check_builtin("first", &click("e00000002")).is_err());
}
