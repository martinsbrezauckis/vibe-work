//! Checks the helper digest required by release app builds. Called by `build.rs` before Tauri embeds resources.

/// Refuses a release binary with development-only runtime switches enabled.
///
/// # Errors
/// `PROFILE=release` with Cargo's debug-assertions cfg enabled.
pub fn check_debug_assertions(profile: &str, enabled: bool) -> Result<(), &'static str> {
    if profile == "release" && enabled {
        Err(
            "release builds must not enable debug assertions; remove CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS or -C debug-assertions",
        )
    } else {
        Ok(())
    }
}

/// Accepts a lower-case SHA-256 for release builds; development builds can use an unpinned package.
///
/// # Errors
/// A supplied release digest is malformed; no digest means no Chrome Control package.
pub fn check(profile: &str, pin: Option<&str>) -> Result<(), &'static str> {
    if profile != "release" || pin.is_none() {
        return Ok(());
    }
    if let Some(pin) = pin
        && pin.len() == 64
        && pin
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Ok(());
    }
    Err("release builds require VW_CHROME_HELPER_SHA256 from the final signed Chrome helper")
}

/// Requires a pinned running-image identity for macOS release builds.
///
/// # Errors
/// The signed helper's code-directory hash is absent or malformed.
pub fn check_cdhash(profile: &str, package: bool, pin: Option<&str>) -> Result<(), &'static str> {
    if profile != "release" || (!package && pin.is_none()) {
        return Ok(());
    }
    if package
        && pin.is_some_and(|pin| {
            pin.len() == 40
                && pin
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
    {
        return Ok(());
    }
    Err("macOS release builds require VW_CHROME_HELPER_CDHASH from the final signed Chrome helper")
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn release_accepts_absence_but_checks_a_supplied_digest() {
        assert_eq!(check("release", None), Ok(()));
        for pin in [
            Some(""),
            Some("1234"),
            Some(&"g".repeat(64)),
            Some(&"A".repeat(64)),
        ] {
            assert!(check("release", pin).is_err());
        }
        assert_eq!(check("release", Some(&"a".repeat(64))), Ok(()));
    }

    #[test]
    fn macos_release_requires_its_running_code_identity() {
        assert!(super::check_cdhash("release", false, None).is_ok());
        assert!(super::check_cdhash("release", true, None).is_err());
        assert!(super::check_cdhash("release", false, Some(&"a".repeat(40))).is_err());
        for pin in [
            Some(""),
            Some("1234"),
            Some(&"g".repeat(40)),
            Some(&"A".repeat(40)),
        ] {
            assert!(super::check_cdhash("release", true, pin).is_err());
        }
        assert!(super::check_cdhash("release", true, Some(&"a".repeat(40))).is_ok());
        assert!(super::check_cdhash("debug", false, None).is_ok());
    }

    #[test]
    fn release_cannot_enable_development_switches() {
        assert!(super::check_debug_assertions("release", true).is_err());
        assert!(super::check_debug_assertions("release", false).is_ok());
        assert!(super::check_debug_assertions("debug", true).is_ok());
    }

    #[test]
    fn development_keeps_its_unpinned_package() {
        assert_eq!(check("debug", None), Ok(()));
    }
}
