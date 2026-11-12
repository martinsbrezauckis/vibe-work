//! Request diagnostics have explicit, bounded reads and never accept network mutation arguments.
//! Native fixture checks statuses, safe headers and actual transport failures separately.
use serde_json::json;
#[test]
fn request_list_and_headers_have_checked_arguments() {
    assert!(
        vw_browser::parse(&json!({"action":"network.list","failedOnly":true,"limit":10})).is_ok()
    );
    assert!(vw_browser::parse(&json!({"action":"network.headers","requestId":"r1"})).is_ok());
    for args in [
        json!({"action":"network.headers"}),
        json!({"action":"network.list","limit":0}),
        json!({"action":"network.headers","requestId":"r1","unredacted":"yes"}),
    ] {
        assert!(vw_browser::parse(&args).is_err(), "{args}");
    }
}
