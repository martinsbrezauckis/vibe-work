//! `CFNetwork` result-kind policy, separated from Core Foundation objects for injected cross-OS tests.
//! The macOS adapter maps checked CF constants to these kinds after the system performs PAC execution.

pub(super) const FAILED: &str = "macOS proxy resolution failed.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Direct,
    Http,
    Https,
    Socks,
    Unknown,
}

pub(super) fn scheme(
    kind: Option<Kind>,
    from_pac: bool,
) -> Result<Option<&'static str>, &'static str> {
    match (kind, from_pac) {
        (Some(Kind::Direct), _) => Ok(None),
        (Some(Kind::Http | Kind::Https), _) => Ok(Some("http")),
        (Some(Kind::Socks), _) => Ok(Some("socks5")),
        (Some(Kind::Unknown) | None, _) => Err(FAILED),
    }
}

#[cfg(test)]
mod tests {
    use super::{FAILED, Kind, scheme};

    #[test]
    fn injected_native_kinds_preserve_manual_and_pac_meanings() {
        for (kind, from_pac, expected) in [
            (Some(Kind::Direct), false, Ok(None)),
            (Some(Kind::Direct), true, Ok(None)),
            (Some(Kind::Http), false, Ok(Some("http"))),
            (Some(Kind::Http), true, Ok(Some("http"))),
            (Some(Kind::Https), false, Ok(Some("http"))),
            (Some(Kind::Https), true, Ok(Some("http"))),
            (Some(Kind::Socks), false, Ok(Some("socks5"))),
            (Some(Kind::Socks), true, Ok(Some("socks5"))),
            (Some(Kind::Unknown), false, Err(FAILED)),
            (Some(Kind::Unknown), true, Err(FAILED)),
            (None, false, Err(FAILED)),
            (None, true, Err(FAILED)),
        ] {
            assert_eq!(scheme(kind, from_pac), expected, "{kind:?} pac={from_pac}");
        }
    }
}
