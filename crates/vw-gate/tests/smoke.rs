//! Public smoke test: every workspace crate compiles and links into one binary.
//!
//! `vw-gate` depends on all the other `vw-*` crates, so reaching their public items from here
//! proves the dependency graph builds and links. Behavioural tests live in the private test tree.

#[test]
fn workspace_crates_link_and_report_a_role() {
    let roles = [
        vw_gate::CRATE_ROLE,
        vw_session::CRATE_ROLE,
        vw_rpc::CRATE_ROLE,
        vw_protocol::CRATE_ROLE,
        vw_supervisor::CRATE_ROLE,
        vw_platform::CRATE_ROLE,
        vw_mistral::CRATE_ROLE,
    ];
    for role in roles {
        assert_ne!(role, "");
    }
}

#[test]
fn native_plugin_switches_and_retired_project_methods_are_refused() {
    use vw_gate::{Class, DenyReason, InboundKind, classify, classify_server_request};
    for method in ["plugins/local/list", "plugins/local/setEnabled"] {
        assert_eq!(
            classify(method),
            Some(Class::Denied(DenyReason::InternalOnly)),
            "{method}"
        );
    }
    for method in [
        "projectLinks/create",
        "projectLinks/link",
        "projectLinks/picker/load",
        "projectLinks/picker/loadMore",
    ] {
        assert_eq!(
            classify(method),
            Some(Class::Denied(DenyReason::NotSupported)),
            "{method}"
        );
    }
    assert_eq!(classify_server_request("tools/call"), InboundKind::Denied);
}

#[test]
fn vibe314_approval_policy_has_a_gated_switch_and_full_access_confirmation() {
    use serde_json::json;
    use vw_gate::{Class, P1, TRUSTED_SURFACE_METHODS, classify, full_access_escalation};

    assert!(TRUSTED_SURFACE_METHODS.contains(&"session/approvalPolicy/update"));
    assert!(!TRUSTED_SURFACE_METHODS.contains(&"session/agent/update"));

    assert_eq!(
        classify("session/approvalPolicy/update"),
        Some(Class::Privileged(P1::ApprovalPolicyUpdate))
    );
    assert!(full_access_escalation(
        "session/approvalPolicy/update",
        &json!({"sessionId":"s", "approvalPolicy":"auto-approve"})
    ));
    assert!(full_access_escalation(
        "session/start",
        &json!({"agentConfig":{"approvalPolicy":"auto-approve"}})
    ));
    assert!(full_access_escalation(
        "config/write",
        &json!({"ops":[{"op":"set","path":"/default_approval_policy","value":"auto-approve"}]})
    ));
    assert!(!full_access_escalation(
        "session/agent/update",
        &json!({"sessionId":"s", "agentName":"default"})
    ));
}

#[test]
fn vibe_315_new_methods_are_explicitly_denied() {
    use vw_gate::{Class, DenyReason, classify};

    for method in [
        "loops/run",
        "loops/update",
        "projects/setRoots",
        "session/leftTurns/interrupt",
        "session/leftTurns/read",
        "session/markAsUnseen",
        "session/mcpApp/callTool",
        "session/mcpApp/readResource",
        "turn/retry",
        "voice/narrate",
        "voice/narrateCancel",
        "voice/transcribeCancel",
        "voice/transcribeStart",
        "voice/transcribeStop",
    ] {
        assert_eq!(
            classify(method),
            Some(Class::Denied(DenyReason::NoUserInterface)),
            "{method}"
        );
    }
    assert_eq!(classify("vibe315/unknown"), None);
}
