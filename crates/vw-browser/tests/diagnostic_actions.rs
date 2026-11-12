//! Named light diagnostics are parsed before browser execution; malformed filters never reach a page.
use serde_json::json;

#[test]
fn diagnostic_actions_have_typed_bounded_arguments() {
    for name in [
        "console.read",
        "page.errors",
        "console.clear",
        "page.errors.clear",
    ] {
        assert_eq!(
            vw_browser::parse(&json!({"action":name})).unwrap().name(),
            name
        );
    }
    for args in [
        json!({"action":"console.read","limit":0}),
        json!({"action":"console.read","limit":101}),
        json!({"action":"console.read","offset":-1}),
        json!({"action":"console.read","level":"unknown"}),
        json!({"action":"console.read","text":"x".repeat(2001)}),
        json!({"action":"console.clear","offset":1}),
    ] {
        assert!(vw_browser::parse(&args).is_err(), "{args}");
    }
}
