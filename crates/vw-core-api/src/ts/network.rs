//! Settings > Network declarations for the core TS generator; depends on the public network view types.
use ts_rs::{Config, TS};

/// Settings > Network types, in their own module.
pub(super) fn network_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::NetworkRequest::decl(cfg),
        crate::NetworkChanges::decl(cfg),
        crate::NetworkResult::decl(cfg),
        crate::NetworkView::decl(cfg),
        crate::NetworkFieldView::decl(cfg),
        crate::NetworkProblem::decl(cfg),
        crate::NetworkTestKind::decl(cfg),
        crate::NetworkTestView::decl(cfg),
    ]
}
