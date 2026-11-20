//! The state order, labels and card actions of Your Chrome's one state model.
use super::*;

pub(super) fn ready() -> Facts {
    Facts {
        package: true,
        package_text: "package problem".into(),
        set_up: true,
        enabled: true,
        helper_up: true,
        extension_folder: true,
        ..Facts::default()
    }
}

pub(super) fn never_probed() -> Probe {
    panic!("a live state decided; Chrome's files must not be read")
}

#[test]
#[allow(
    clippy::too_many_lines,
    clippy::type_complexity,
    reason = "one table row per state"
)]
fn labels_states() {
    use AddonAction as A;
    use AddonState as S;
    let cases: Vec<(&str, Facts, Probe, S, &str, Option<A>)> = vec![
        (
            "no package",
            Facts {
                package: false,
                ..ready()
            },
            Probe::Unknown,
            S::Unavailable,
            "package problem",
            None,
        ),
        (
            "not set up",
            Facts {
                set_up: false,
                ..ready()
            },
            Probe::Unknown,
            S::NotSetUp,
            "Not connected",
            Some(A::Connect),
        ),
        (
            "no chrome",
            Facts {
                browser_issue: Some(BrowserIssue::Missing),
                ..ready()
            },
            Probe::Unknown,
            S::ChromeMissing,
            "Google Chrome is not installed",
            None,
        ),
        (
            "snap",
            Facts {
                browser_issue: Some(BrowserIssue::Sandboxed),
                ..ready()
            },
            Probe::Unknown,
            S::ChromeSandboxed,
            "Snap or Flatpak Chrome is unsupported",
            None,
        ),
        (
            "brave",
            Facts {
                browser_issue: Some(BrowserIssue::DifferentBrowser),
                ..ready()
            },
            Probe::Unknown,
            S::OtherBrowser,
            "Google Chrome required",
            None,
        ),
        (
            "off",
            Facts {
                enabled: false,
                connected: true,
                ..ready()
            },
            Probe::Unknown,
            S::SwitchedOff,
            "Off",
            Some(A::TurnOn),
        ),
        (
            "connected",
            Facts {
                connected: true,
                ..ready()
            },
            Probe::Off,
            S::Connected,
            "Connected",
            None,
        ),
        (
            "connected old",
            Facts {
                connected: true,
                installed_version: Some("0.2.3".into()),
                min_version: Some("0.2.4".into()),
                ..ready()
            },
            Probe::Unknown,
            S::UpdateNeeded,
            "Update the extension",
            None,
        ),
        (
            "review",
            Facts {
                store_review: true,
                ..ready()
            },
            Probe::Unknown,
            S::InReview,
            "Extension in review",
            None,
        ),
        (
            "other owner",
            Facts {
                other_owner: true,
                waiting: true,
                ..ready()
            },
            Probe::Unknown,
            S::OtherOwner,
            "Paired with another Vibe Work",
            Some(A::OpenPairingPage),
        ),
        (
            "ticket issued",
            Facts {
                connecting: true,
                waiting: true,
                ..ready()
            },
            Probe::Unknown,
            S::Connecting,
            "Connecting",
            None,
        ),
        (
            "waiting",
            Facts {
                waiting: true,
                ..ready()
            },
            Probe::Unknown,
            S::NotPaired,
            "Pair in Chrome",
            Some(A::OpenPairingPage),
        ),
        (
            "another helper holds the connection",
            Facts {
                helper_up: false,
                helper_conflict: true,
                ..ready()
            },
            Probe::Unknown,
            S::OtherHelperRunning,
            "Another Vibe Work helper is running",
            None,
        ),
        (
            "helper down",
            Facts {
                helper_up: false,
                ..ready()
            },
            Probe::Unknown,
            S::HelperNotRunning,
            "Not running",
            Some(A::Connect),
        ),
        (
            "chrome turned it off",
            ready(),
            Probe::Off,
            S::TurnedOffInChrome,
            "Turn on in Chrome",
            None,
        ),
        (
            "external, chrome asks to enable",
            Facts {
                install_state: Some("external".into()),
                ..ready()
            },
            Probe::AwaitingEnable,
            S::EnableInChrome,
            "Enable in Chrome",
            None,
        ),
        (
            "external, turned off by the user on chrome://extensions",
            Facts {
                install_state: Some("external".into()),
                ..ready()
            },
            Probe::Off,
            S::TurnedOffInChrome,
            "Turn on in Chrome",
            None,
        ),
        (
            "external, not added yet",
            Facts {
                install_state: Some("external".into()),
                extension_folder: false,
                ..ready()
            },
            Probe::Unknown,
            S::EnableInChrome,
            "Enable in Chrome",
            None,
        ),
        (
            "restart",
            Facts {
                install_state: Some("restartChrome".into()),
                ..ready()
            },
            Probe::Unknown,
            S::RestartChrome,
            "Restart Chrome",
            None,
        ),
        (
            "old, not connected",
            Facts {
                installed_version: Some("0.2.3".into()),
                min_version: Some("0.2.10".into()),
                ..ready()
            },
            Probe::NotOff,
            S::UpdateNeeded,
            "Update the extension",
            None,
        ),
        (
            "new enough",
            Facts {
                installed_version: Some("0.2.10".into()),
                min_version: Some("0.2.9".into()),
                paired_once: true,
                ..ready()
            },
            Probe::NotOff,
            S::ChromeClosed,
            "Connected when Chrome is open",
            None,
        ),
        (
            "paired, chrome closed",
            Facts {
                paired_once: true,
                ..ready()
            },
            Probe::Unknown,
            S::ChromeClosed,
            "Connected when Chrome is open",
            None,
        ),
        (
            "not added",
            Facts {
                extension_folder: false,
                ..ready()
            },
            Probe::Unknown,
            S::ExtensionNotInstalled,
            "Add to Chrome",
            Some(A::Connect),
        ),
        (
            "unknown",
            ready(),
            Probe::Unknown,
            S::NotConnected,
            "Not connected",
            Some(A::Connect),
        ),
    ];
    for (name, facts, probe, state_expected, label, action) in cases {
        let got = state(&facts, || probe);
        assert_eq!(got, state_expected, "{name}");
        let shown = view(got, &facts.package_text);
        assert_eq!(shown.label, label, "{name}");
        assert_eq!(shown.action, action, "{name}");
        assert!(!shown.instruction.is_empty(), "{name}");
    }
    // A live state never reads Chrome's files.
    for facts in [
        Facts {
            connected: true,
            ..ready()
        },
        Facts {
            waiting: true,
            ..ready()
        },
        Facts {
            connecting: true,
            ..ready()
        },
        Facts {
            helper_up: false,
            ..ready()
        },
    ] {
        state(&facts, never_probed);
    }
}

/// Every instruction names the recovery step; a paired closed browser is opened by the app.
#[test]
fn every_instruction_gives_the_reason_and_the_step() {
    use AddonState as S;
    for s in [
        S::Unavailable,
        S::NotSetUp,
        S::ChromeMissing,
        S::ChromeSandboxed,
        S::OtherBrowser,
        S::SwitchedOff,
        S::InReview,
        S::OtherOwner,
        S::Connecting,
        S::NotPaired,
        S::OtherHelperRunning,
        S::HelperNotRunning,
        S::TurnedOffInChrome,
        S::RestartChrome,
        S::UpdateNeeded,
        S::ChromeClosed,
        S::EnableInChrome,
        S::ExtensionNotInstalled,
        S::NotConnected,
    ] {
        let text = view(s, "x").instruction;
        assert!(
            (s == S::ChromeClosed && text.contains("Call chrome_session"))
                || text.contains("Ask the user")
                || text.contains("ask the user")
                || text.contains("Tell the user"),
            "{s:?}: {text}"
        );
        assert!(text.len() <= 300, "{s:?}: one line");
    }
}
