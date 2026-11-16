//! A title the person typed with Rename is guarded like every other chat title.
//!
//! PREDICT (before the fix): the rename request carries the typed text to Vibe unchanged, so Vibe stores and
//! echoes a title holding the vault value; the guarded reply is the only copy that may be shown.
//! Depends on: the feature test core and an invented vault value. Covers `withhold_rename`.
use super::*;
use serde_json::json;
use vw_vault::SecretString;

pub(crate) const VALUE: &str = "zebrafish3c9e71a4b8d2";

pub(crate) fn know(shared: &Shared, value: &str) {
    shared.titles.values.write().unwrap().push(VaultScanSecret {
        id: "invented".into(),
        name: "invented".into(),
        login_id: None,
        modified_ms: 0,
        website_password: false,
        value: SecretString::exact(value),
    });
}

#[tokio::test]
async fn a_rename_request_holds_no_vault_value_but_keeps_a_whole_prefix_word() {
    let f = crate::feature_test_core::Fixture::new().await;
    let shared = &f.core.shared;
    know(shared, VALUE);
    for (typed, shown) in [
        ("zebrafish3c9e71a4b8d2 full", "[secret withheld] full"),
        ("zebrafish3c9e71a4 check", "[secret withheld] check"),
        ("zebrafish notes", "zebrafish notes"),
    ] {
        let mut params = json!({"sessionId":"chat","title":typed});
        withhold_rename(shared, "session/rename", &mut params);
        assert_eq!(params["title"], shown, "{typed}");
        assert_eq!(params["sessionId"], "chat");
    }
    let mut other = json!({"title":"zebrafish3c9e71a4b8d2 full"});
    withhold_rename(shared, "session/pin", &mut other);
    assert_eq!(other["title"], "zebrafish3c9e71a4b8d2 full");
    f.finish().await;
}
