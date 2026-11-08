//! The state order, labels and card actions of Your Chrome's one state model.
use super::*;

fn ready() -> Facts {
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

fn never_probed() -> Probe {
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
                installed_version: Some("0.1.3".into()),
                min_version: Some("0.1.4".into()),
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
            "external, turned off by the person on chrome://extensions",
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
                installed_version: Some("0.1.3".into()),
                min_version: Some("0.1.10".into()),
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
                installed_version: Some("0.1.10".into()),
                min_version: Some("0.1.9".into()),
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
                || text.contains("Ask the person")
                || text.contains("Tell the person"),
            "{s:?}: {text}"
        );
        assert!(text.len() <= 300, "{s:?}: one line");
    }
}

/// Chrome's own records (Windows, Chrome 156, the extension added through the per-user registration), trimmed to
/// the fields Chrome changes: a new external install it holds for the person's consent, and the same install after
/// the person enabled it and later turned it off on `chrome://extensions` (then quit and reopened Chrome).
#[test]
fn real_preferences_tell_a_new_external_install_from_one_the_person_turned_off() {
    use super::super::probe::{Prober, profiles};
    const ID: &str = "gnlamocfdabihichplpepjpehlbafhng";
    let new_install = serde_json::json!({"ack_prompt_count": 1, "creation_flags": 9, "disable_reasons": [8192],
        "from_webstore": true, "location": 6, "path": format!("{ID}\\0.1.3_0"), "was_installed_by_default": false});
    let turned_off = serde_json::json!({"ack_external": true, "creation_flags": 9, "disable_reasons": [1],
        "from_webstore": true, "location": 6, "path": format!("{ID}\\0.1.3_0"), "was_installed_by_default": false});
    let root = std::env::temp_dir().join(format!("chrome-addon-real-{}", std::process::id()));
    for (name, entry, expected) in [
        (
            "new external install",
            new_install,
            AddonState::EnableInChrome,
        ),
        (
            "turned off by the person",
            turned_off,
            AddonState::TurnedOffInChrome,
        ),
    ] {
        let _ = std::fs::remove_dir_all(&root);
        let profile = root.join("Profile 1");
        std::fs::create_dir_all(profile.join("Extensions").join(ID).join("0.1.3_0")).unwrap();
        let prefs = serde_json::json!({"extensions": {"settings": {ID: entry}}});
        std::fs::write(profile.join("Secure Preferences"), prefs.to_string()).unwrap();
        let facts = Facts {
            install_state: Some("external".into()),
            ..ready()
        };
        let found = state(&facts, || {
            Prober::default().probe(profiles(&root, ID), ID, std::time::Instant::now())
        });
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(found, expected, "{name}");
    }
}

#[test]
fn external_install_keeps_enable_while_chrome_copies_and_waits() {
    let mut facts = Facts {
        install_state: Some("external".into()),
        extension_folder: false,
        ..ready()
    };
    // Before copy, +2 s folder, +4..11 s preferences lag, then disabled until Enable.
    for (seconds, folder, probe) in [
        (0, false, Probe::Unknown),
        (2, true, Probe::Unknown),
        (4, true, Probe::NotOff),
        (7, true, Probe::Unknown),
        (11, true, Probe::AwaitingEnable),
    ] {
        facts.extension_folder = folder;
        assert_eq!(
            state(&facts, || probe),
            AddonState::EnableInChrome,
            "at +{seconds}s"
        );
    }
    facts.waiting = true;
    assert_eq!(state(&facts, never_probed), AddonState::NotPaired);
    facts.waiting = false;
    facts.connected = true;
    assert_eq!(state(&facts, never_probed), AddonState::Connected);
}

#[test]
fn turning_on_waits_for_a_live_profile_while_chrome_is_open() {
    let mut facts = Facts {
        paired_once: true,
        chrome_running: true,
        enabled: false,
        ..ready()
    };
    assert_eq!(state(&facts, never_probed), AddonState::SwitchedOff);
    facts.enabled = true;
    for seconds in [0, 1] {
        assert_eq!(
            state(&facts, || Probe::NotOff),
            AddonState::Connecting,
            "at +{seconds}s"
        );
    }
    facts.connected = true;
    assert_eq!(state(&facts, never_probed), AddonState::Connected);
    facts.connected = false;
    facts.chrome_running = false;
    assert_eq!(state(&facts, || Probe::NotOff), AddonState::ChromeClosed);
    facts.chrome_running = true;
    facts.extension_folder = false;
    assert_eq!(
        state(&facts, || Probe::Unknown),
        AddonState::ExtensionNotInstalled
    );
}

#[test]
fn chrome_steps_say_where_to_click_and_how_to_restart() {
    let enable = view(AddonState::EnableInChrome, "");
    let restart = view(AddonState::RestartChrome, "");
    if cfg!(target_os = "macos") {
        assert_eq!(
            enable.step.as_deref(),
            Some("Click Action required → New extension added (Vibe Work).")
        );
        assert_eq!(
            restart.step.as_deref(),
            Some("Quit Chrome with Cmd+Q and open it again.")
        );
    } else {
        assert_eq!(
            enable.step.as_deref(),
            Some("Click Enable extension in Chrome.")
        );
        assert_eq!(
            restart.step.as_deref(),
            Some("Close every Chrome window and open it again.")
        );
    }
}

#[test]
fn update_label_follows_the_connected_profile_and_disconnected_oldest_profile() {
    let root = std::env::temp_dir().join(format!("addon-versions-{}", std::process::id()));
    let profiles = [root.join("Profile A"), root.join("Profile 1")];
    for (profile, version) in profiles.iter().zip(["0.1.3_0", "0.1.6_0"]) {
        std::fs::create_dir_all(profile.join("Extensions/id").join(version)).unwrap();
    }
    for (connected, live, expected) in [
        (
            true,
            vec![("A".into(), Some("0.1.3".into()))],
            AddonState::UpdateNeeded,
        ),
        (
            true,
            vec![("B".into(), Some("0.1.6".into()))],
            AddonState::Connected,
        ),
        (false, vec![], AddonState::UpdateNeeded),
    ] {
        let facts = Facts {
            connected,
            installed_version: installed_version(
                &super::super::setup::ChromeControlConfig {
                    macos: Some(false),
                    ..super::super::setup::ChromeControlConfig::default()
                },
                &profiles,
                "id",
                connected,
                &live,
            ),
            min_version: Some("0.1.6".into()),
            ..ready()
        };
        assert_eq!(state(&facts, || Probe::NotOff), expected, "live={live:?}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_profile_versions_follow_status_and_clear_on_disconnect() {
    let profiles = ConnectedProfiles::default();
    for (a, b, expected) in [(true, false, "A"), (false, true, "B")] {
        profiles.observe(&serde_json::json!({"profiles":[
            {"profileId":"A","connected":a,"extensionVersion":"0.1.3"},
            {"profileId":"B","connected":b,"extensionVersion":"0.1.6"}
        ]}));
        assert_eq!(profiles.snapshot().len(), 1);
        assert_eq!(profiles.snapshot()[0].0, expected);
    }
    profiles.clear();
    assert_eq!(profiles.snapshot(), Vec::<(String, Option<String>)>::new());
    profiles.observe(&serde_json::json!({"profiles":[{"profileId":"unknown","connected":true}]}));
    assert_eq!(profiles.snapshot(), vec![("unknown".into(), None)]);
    assert_eq!(
        installed_version(
            &super::super::setup::ChromeControlConfig::default(),
            &[],
            "id",
            true,
            &profiles.snapshot()
        ),
        None
    );
    profiles.observe(&serde_json::json!({"profiles":[]}));
    assert_eq!(profiles.snapshot(), Vec::<(String, Option<String>)>::new());
}
#[test]
fn a_disabled_extension_overrides_a_stale_connecting_phase() {
    let facts = Facts {
        paired_once: true,
        connecting: true,
        extension_folder: true,
        ..ready()
    };
    assert_eq!(state(&facts, || Probe::Off), AddonState::TurnedOffInChrome);
    assert_eq!(
        state(
            &Facts {
                connected: true,
                ..facts
            },
            || panic!("live connection wins")
        ),
        AddonState::Connected
    );
}

#[test]
fn a_missing_browser_is_diagnosed_before_setup_and_never_offers_connect() {
    let facts = Facts {
        set_up: false,
        browser_issue: Some(BrowserIssue::Missing),
        ..ready()
    };
    let state = state(&facts, never_probed);
    assert_eq!(state, AddonState::ChromeMissing);
    for browser in [
        super::super::agent_browser::Browser::Chrome,
        super::super::agent_browser::Browser::Edge,
    ] {
        let view = view_for(state, "", browser);
        assert_eq!(view.action, None);
        assert!(view.label.contains("is not installed"));
        assert!(view.step.as_deref().unwrap().starts_with("Install "));
        assert!(
            !view
                .instruction
                .contains(vw_vocabulary::texts::CONNECTOR_CONNECT)
        );
        assert!(!super::super::availability::reason(&view, browser).contains("addon_suggest"));
    }
}

#[test]
fn a_lost_connection_retries_before_asking_for_a_browser_restart() {
    for browser in [
        super::super::agent_browser::Browser::Chrome,
        super::super::agent_browser::Browser::Edge,
    ] {
        let mut view = view_for(AddonState::Connecting, "", browser);
        reconnecting_for(&mut view, true, browser);
        assert_eq!(
            view.instruction,
            browser.words("{browser}'s Vibe Work connection was lost and is reconnecting automatically. Retry this step in a few seconds. Only if the connection stays down, ask the person to open {browser}'s Extensions page and turn on Vibe Work if it is off, or restart {browser} if it is already on.")
        );
    }
}

#[test]
fn first_disconnect_recovery_works_even_before_preferences_record_the_extension_is_off() {
    let facts = Facts {
        paired_once: true,
        connecting: true,
        chrome_running: true,
        ..ready()
    };
    for probe in [Probe::Unknown, Probe::NotOff, Probe::Off] {
        let state = state(&facts, || probe);
        for browser in [
            super::super::agent_browser::Browser::Chrome,
            super::super::agent_browser::Browser::Edge,
        ] {
            let mut view = view_for(state, "", browser);
            reconnecting_for(&mut view, true, browser);
            assert_eq!(view.label, browser.words("Turn on in {browser}"));
            assert!(view.instruction.contains("Extensions page"));
            assert!(!view.instruction.contains("icon"));
            assert!(!view.instruction.contains("Check connection"));
        }
    }
}
