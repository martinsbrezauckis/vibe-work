//! Chromium pairing uses the existing native dialog on all platforms; only its offered Connect label approves.
//! Exercises the button ordering and heading helpers used by GTK, `TaskDialog` and `NSAlert`.
use super::*;

#[test]
fn pairing_requires_connect_and_defaults_to_not_now() {
    for browser in ["Chrome", "Edge"] {
        let request = ConfirmationRequest {
            requester: Requester::Core,
            method: vw_core_api::BROWSER_PAIRING_DIALOG_METHOD.into(),
            params: serde_json::json!({}),
            summary: format!(
                "Connect {browser} to Vibe Work?\n\n{browser} needs your approval. Choose Connect to allow this connection."
            ),
            always: None,
        };
        let buttons = dialog_buttons(&request);
        assert_eq!(
            buttons,
            [vw_vocabulary::texts::CONNECTOR_CONNECT, "Not now"]
        );
        assert_eq!(
            system_dialog_order(&buttons),
            ["Not now", vw_vocabulary::texts::CONNECTOR_CONNECT]
        );
        let text = dialog_text(&request).unwrap();
        assert_eq!(
            dialog_heading(&text).0,
            format!("Connect {browser} to Vibe Work?")
        );
        assert_eq!(
            dialog_answer(&request, Some(vw_vocabulary::texts::CONNECTOR_CONNECT)),
            Confirmation::Approved
        );
        for pressed in [
            None,
            Some("Not now"),
            Some("Continue"),
            Some(vw_vocabulary::label("vaultPermissions", "allowed")),
            Some("Cancel"),
        ] {
            assert_eq!(dialog_answer(&request, pressed), Confirmation::Declined);
        }
    }
}
