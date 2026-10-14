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
