//! The oldest system a newer version runs on. A release manifest may name it per platform beside `url` and
//! `signature`: macOS and Windows as `"minimumSystemVersion"` (`"11.0"`, the Windows build `"10.0.17763"`), Linux
//! as `"minimumGlibc"` (`"2.39"`, the highest glibc symbol version the `AppImage`'s programs need: what decides
//! whether it starts, whatever the distribution). The app reads the field of its own system and this computer's
//! value ([`minimum_field`]), and on an older system the update flow shows why
//! ([`crate::update_view::Phase::OsTooOld`]) instead of downloading a package that would not start.
//!
//! Versions are dotted numbers (`14`, `14.0`, `11.7.10`; at most four parts, missing parts count as 0). Anything
//! else is unreadable. A present unreadable minimum fails the check before download. An absent field (older
//! announcements), or an unreadable computer version with a valid minimum, imposes no restriction.
//!
//! Depends on: serde, `serde_json`. Called by: `app` (`updater.rs`, backend check), [`crate::update_flow`], tests.

use serde::Serialize;

/// The manifest field of macOS and Windows: the oldest system version (`11.0`, the Windows build `10.0.17763`).
pub const MINIMUM_SYSTEM_VERSION: &str = "minimumSystemVersion";

/// The manifest field of Linux: the oldest C library (glibc) version the `AppImage`'s programs need (`2.39`).
pub const MINIMUM_GLIBC: &str = "minimumGlibc";

/// The manifest field that names the oldest system `os` (as [`OsTooOld::os`]) can run the package on: Linux reads
/// [`MINIMUM_GLIBC`] (never a system version), every other system [`MINIMUM_SYSTEM_VERSION`].
#[must_use]
pub fn minimum_field(os: &str) -> &'static str {
    if os == "Linux" {
        MINIMUM_GLIBC
    } else {
        MINIMUM_SYSTEM_VERSION
    }
}

/// This computer's system is older than the oldest one the offered version runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsTooOld {
    /// The system's name as the user knows it (`macOS`, `Windows`, `Linux`).
    pub os: String,
    /// The oldest version the offered release runs on (from the manifest; on Linux a glibc version).
    pub minimum: String,
    /// This computer's version (Windows: `10.0.<build>.<revision>`; Linux: the running glibc).
    pub current: String,
}

/// The parts of a dotted version, `None` when it is not one.
fn parts(version: &str) -> Option<Vec<u64>> {
    let parts = version
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse::<u64>().ok()
        })
        .collect::<Option<Vec<u64>>>()?;
    (parts.len() <= 4).then_some(parts)
}

/// Whether `current` is lower than `minimum`; `None` when either is unreadable.
/// Announcement fields must first be validated with [`check_minimum`].
#[must_use]
pub fn below_minimum(current: &str, minimum: &str) -> Option<bool> {
    let (mut current, mut minimum) = (parts(current)?, parts(minimum)?);
    let len = current.len().max(minimum.len());
    current.resize(len, 0);
    minimum.resize(len, 0);
    Some(current < minimum)
}

/// Builds the finding for a readable lower system version; `None` when no comparison is available or it is not
/// lower. Use [`check_minimum`] for announcement fields so unreadable minimums produce an error.
#[must_use]
pub fn os_too_old(os: &str, current: Option<&str>, minimum: Option<&str>) -> Option<OsTooOld> {
    let (current, minimum) = (current?, minimum?);
    below_minimum(current, minimum)
        .filter(|below| *below)
        .map(|_| OsTooOld {
            os: os.to_owned(),
            minimum: minimum.to_owned(),
            current: current.to_owned(),
        })
}

/// Checks the announced minimum before deciding whether this system is too old.
/// An absent minimum or an unreadable computer version with a valid minimum imposes no restriction.
///
/// # Errors
/// A present minimum that is not a string accepted by the dotted version parser, even when the computer's
/// version is unknown.
pub fn check_minimum(
    os: &str,
    current: Option<&str>,
    minimum: Option<&serde_json::Value>,
) -> Result<Option<OsTooOld>, String> {
    let Some(minimum) = minimum else {
        return Ok(None);
    };
    let minimum = minimum
        .as_str()
        .filter(|version| parts(version).is_some())
        .ok_or_else(|| {
            if os == "Linux" {
                "an unreadable minimum glibc version".to_owned()
            } else {
                "an unreadable minimum system version".to_owned()
            }
        })?;
    Ok(os_too_old(os, current, Some(minimum)))
}

#[cfg(test)]
mod tests {
    use super::{OsTooOld, check_minimum};
    use serde_json::json;

    #[test]
    fn an_absent_minimum_restricts_nothing() {
        assert_eq!(check_minimum("macOS", Some("15.1"), None), Ok(None));
    }

    #[test]
    fn a_readable_minimum_compares_the_system_version() {
        assert_eq!(
            check_minimum("macOS", Some("15.1"), Some(&json!("99.0"))),
            Ok(Some(OsTooOld {
                os: "macOS".into(),
                minimum: "99.0".into(),
                current: "15.1".into(),
            }))
        );
        assert_eq!(
            check_minimum("macOS", Some("15.1"), Some(&json!("14.0"))),
            Ok(None)
        );
    }

    #[test]
    fn unreadable_minimums_are_bad_announcements() {
        for minimum in [
            json!(7),
            json!(""),
            json!("14.x"),
            json!("1.2.3.4.5"),
            json!("99999999999999999999"),
            json!("14..0"),
            json!("14.0."),
            json!(null),
            json!(true),
            json!([]),
            json!({}),
        ] {
            for os in ["macOS", "Windows", "Linux"] {
                for current in [Some("15.1"), None, Some("unreadable")] {
                    assert!(
                        check_minimum(os, current, Some(&minimum)).is_err(),
                        "{os}: {minimum} must fail even when current is {current:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unreadable_computer_version_restricts_nothing() {
        for current in [None, Some(""), Some("15.x"), Some("1.2.3.4.5")] {
            assert_eq!(
                check_minimum("macOS", current, Some(&json!("99.0"))),
                Ok(None)
            );
        }
    }
}
