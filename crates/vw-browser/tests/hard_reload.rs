//! Cache-bypass requests remain distinct from ordinary reloads and reject malformed flags.
//! Runs the public action parser used by the core before dispatching to the native engine.
use serde_json::json;
use vw_browser::parse;

#[test]
fn cache_bypass_is_a_distinct_navigation_operation() {
    let plain = parse(&json!({"action":"reload"})).unwrap();
    let cached = parse(&json!({"action":"reload","bypassCache":false})).unwrap();
    let fresh = parse(&json!({"action":"reload","bypassCache":true})).unwrap();
    assert_eq!(plain, cached);
    assert_ne!(plain, fresh, "cache bypass must reach the native engine");
    assert_eq!(fresh.name(), "reload");
}

#[test]
fn cache_bypass_does_not_silently_ignore_invalid_arguments() {
    assert!(parse(&json!({"action":"reload","bypassCache":"true"})).is_err());
    assert!(parse(&json!({"action":"back","bypassCache":true})).is_err());
}
