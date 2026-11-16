//! End-to-end seam for the update offer's guard list: a debug build may add fixture Vibe versions to the versions the
//! launcher guards were checked with, so an end-to-end run can prove the offer path with a version that only exists in
//! its fixture manifest. A release build never reads the variable: its offer uses only the guard list.
//!
//! Depends on: `vw_supervisor::guarded_vibe_versions`. Called by: test hosts (never by the app's update check).

/// The environment variable a debug build reads: comma-separated plain versions (`3.1.3,3.1.4`).
pub const E2E_EXTRA_GUARDED_VIBE: &str = "VW_E2E_EXTRA_GUARDED_VIBE";

/// The guard list for an update offer in a test host: [`crate::guarded_vibe_versions`], plus in a debug build the
/// plain versions named in [`E2E_EXTRA_GUARDED_VIBE`]. In a release build the variable is not read.
#[must_use]
pub fn guard_list_with_e2e_extra() -> Vec<String> {
    let debug = cfg!(debug_assertions);
    let extra = if debug {
        std::env::var(E2E_EXTRA_GUARDED_VIBE).ok()
    } else {
        None
    };
    with_extra(crate::guarded_vibe_versions(), extra.as_deref(), debug)
}

/// `base` plus the plain versions of `extra` when `debug`; `base` unchanged otherwise. An item that is not a plain
/// version (digits separated by dots) is left out, and a version already listed is not repeated.
#[must_use]
pub fn with_extra(mut base: Vec<String>, extra: Option<&str>, debug: bool) -> Vec<String> {
    if !debug {
        return base;
    }
    let plain = |v: &str| {
        !v.is_empty()
            && v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    for version in extra.unwrap_or_default().split(',').map(str::trim) {
        if plain(version) && !base.iter().any(|known| known == version) {
            base.push(version.to_owned());
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Vec<String> {
        vec!["3.1.2".to_owned()]
    }

    // PREDICT (before the seam): the e2e fixture's offered version is outside the guard list, so `evaluate` withholds
    // it (available null); with the debug-only extension it is in the list. A release build ignores the variable.
    #[test]
    fn a_debug_build_adds_the_fixture_versions() {
        assert_eq!(
            with_extra(base(), Some("3.1.3"), true),
            vec!["3.1.2".to_owned(), "3.1.3".to_owned()]
        );
        assert_eq!(
            with_extra(base(), Some(" 3.1.3 , 3.1.2,3.2.0 "), true),
            vec!["3.1.2".to_owned(), "3.1.3".to_owned(), "3.2.0".to_owned()]
        );
    }

    #[test]
    fn a_release_build_ignores_the_variable() {
        assert_eq!(with_extra(base(), Some("3.1.3"), false), base());
        assert_eq!(with_extra(base(), None, false), base());
    }

    #[test]
    fn only_plain_versions_are_added() {
        for junk in ["", "3.1.x", "3..1", "v3.1.3", "3.1.3;rm", ",", "3.1.3-rc1"] {
            assert_eq!(with_extra(base(), Some(junk), true), base(), "{junk:?}");
        }
        assert_eq!(with_extra(base(), None, true), base());
    }

    /// Prints the live list (run by `the_live_list_follows_the_build_profile` in a child with the variable set).
    #[test]
    #[ignore = "run as a child of the_live_list_follows_the_build_profile"]
    fn print_live_list() {
        println!("LIVE {}", guard_list_with_e2e_extra().join(","));
    }

    // The live function with the variable set, in this test binary's own profile: `cargo test` (debug) adds the
    // version, `cargo test --release` (debug assertions off, as in a release build) leaves the guard list unchanged.
    #[test]
    fn the_live_list_follows_the_build_profile() {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "e2e_guard::tests::print_live_list",
                "--nocapture",
            ])
            .env(E2E_EXTRA_GUARDED_VIBE, "9.9.9")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let live = text
            .lines()
            .find_map(|line| line.strip_prefix("LIVE "))
            .unwrap_or_else(|| panic!("no LIVE line: {text}"));
        let mut expected = crate::guarded_vibe_versions();
        if cfg!(debug_assertions) {
            expected.push("9.9.9".to_owned());
        }
        assert_eq!(live, expected.join(","));
    }
}
