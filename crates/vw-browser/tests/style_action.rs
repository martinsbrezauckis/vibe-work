//! Typed CSS insertion joins the page-script gate instead of pretending to be a read.
//! The script execution fixture separately checks completion and vault exclusion.
use serde_json::json;
#[test]
fn style_insertion_is_explicit_and_bounded() {
    let action = vw_browser::parse(
        &json!({"action":"page.add_style","css":"body { color: rgb(1, 2, 3); }"}),
    )
    .unwrap();
    assert_eq!(action.name(), "page.add_style");
    assert!(vw_browser::parse(&json!({"action":"page.add_style","css":""})).is_err());
}
