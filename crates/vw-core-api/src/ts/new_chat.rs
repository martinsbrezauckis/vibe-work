//! New chat screen declarations for the core TS generator (`new_chat.ts`); depends on the `newChat` view types.
use ts_rs::{Config, TS};
pub(super) fn decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::NewChatRequest::decl(cfg),
        crate::NewChatView::decl(cfg),
    ]
}
