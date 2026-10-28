//! Identifies the Chrome first-use tool for the hook-time core policy.
/// Whether this is the first-use Chrome tool.
#[must_use]
pub fn is_session_gateway(name: &str) -> bool {
    name.eq_ignore_ascii_case("mcp_vw.use_your_chrome")
}
