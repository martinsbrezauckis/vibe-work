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
