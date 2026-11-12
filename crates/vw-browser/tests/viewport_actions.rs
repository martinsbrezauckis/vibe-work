//! Typed responsive-preview requests; invalid dimensions never reach native placement.
//! Covers phone/tablet presets and reset without changing the page's user agent.
#![allow(clippy::unwrap_used)]
use serde_json::json;
#[test]
fn responsive_sizes_and_reset_are_executable() {
    for args in [
        json!({"action":"tabs.emulate","width":390,"height":844}),
        json!({"action":"tabs.emulate","preset":"tablet"}),
        json!({"action":"tabs.emulate","reset":true}),
    ] {
        assert!(vw_browser::parse(&args).is_ok(), "{args}");
    }
}
#[test]
fn invalid_or_unsupported_emulation_is_not_silently_ignored() {
    for args in [
        json!({"action":"tabs.emulate","width":0,"height":500}),
        json!({"action":"tabs.emulate","width":390}),
        json!({"action":"tabs.emulate","preset":"phone","width":800,"height":600}),
        json!({"action":"tabs.emulate","reset":true,"preset":"phone"}),
        json!({"action":"tabs.emulate","preset":"phone","mobile":true}),
        json!({"action":"tabs.emulate","colorScheme":"sepia"}),
    ] {
        assert!(vw_browser::parse(&args).is_err(), "{args}");
    }
}

#[test]
fn color_scheme_can_be_changed_without_resizing_the_page() {
    for scheme in ["light", "dark", "system"] {
        assert!(vw_browser::parse(&json!({"action":"tabs.emulate","colorScheme":scheme})).is_ok());
    }
}
