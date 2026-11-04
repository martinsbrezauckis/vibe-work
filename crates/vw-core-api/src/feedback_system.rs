//! Feedback's small diagnostic allowlist; core-owned values only, without device identifiers or paths.
use crate::{Core, FeedbackSystem};
fn fact(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._()+-".contains(c))
    {
        value.to_owned()
    } else {
        "unknown".to_owned()
    }
}
fn os_version() -> String {
    if let Some((_, version)) = vw_platform::system_version() {
        return fact(&version);
    }
    #[cfg(target_os = "linux")]
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        let field = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key))
                .map(|s| s.trim_matches('"'))
        };
        if let (Some(id), Some(version)) = (field("ID="), field("VERSION_ID=")) {
            return fact(&format!("{id} {version}"));
        }
    }
    "unknown".to_owned()
}
pub(crate) fn collect(core: &Core) -> FeedbackSystem {
    FeedbackSystem {
        app_version: fact(crate::version()),
        vibe_version: fact(core.status().vibe_version.as_deref().unwrap_or("unknown")),
        os: std::env::consts::OS.to_owned(),
        os_version: os_version(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}
