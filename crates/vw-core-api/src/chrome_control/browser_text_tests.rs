//! Fixed agent refusals are worded from the producer's typed id ([`Fixed`]); `render` no longer reads them.
//! PREDICT (red on the sentence-matching mapping): `render` recognised a fixed refusal only by its exact Chrome
//! sentence, so a producer whose sentence differs by one character (here the final period) reached a Firefox
//! agent in Chrome's words. With typed ids the producer names the refusal and the wording comes from the id.
use super::*;
use serde_json::json;

const BROWSERS: [Browser; 4] = [
    Browser::Chrome,
    Browser::Edge,
    Browser::Safari,
    Browser::Firefox,
];

/// The expected Chrome wording for each fixed refusal.
fn chrome_words(id: Fixed) -> String {
    let words = match id {
        Fixed::InvalidArguments => {
            "Not run: invalid Chrome tool arguments. Use chrome_status for the action's argument schema."
        }
        Fixed::OpenSessionFirst => "Not run: open a Chrome browser session in this chat first.",
        Fixed::SelectionFromUser => {
            "Not run: selection is sent by the user from Chrome. Ask the user to use Ask Vibe Work in Chrome's context menu."
        }
        Fixed::OtherChat => {
            "Not run: this Chrome browser session belongs to another chat. Open a Chrome browser session for this chat with chrome_session."
        }
        Fixed::LostConnection => {
            "Not run: this Chrome browser session lost its connection to Chrome. Check chrome_status, reconnect the browser if needed, then open a new browser session with chrome_session."
        }
        Fixed::Stopped => {
            "Not run: this Chrome browser session was stopped; open a new one with chrome_session."
        }
        Fixed::ModeLowered => {
            "Not run: this Chrome browser session ended because this chat's access mode was lowered; its tabs stay open. Open a new one with chrome_session if the new mode allows the work."
        }
        Fixed::UnknownAction => {
            "Not run: unknown Chrome action. Use chrome_status to find supported actions."
        }
        Fixed::Unavailable => {
            return format!(
                "{} is not available in this version of Vibe Work. Use an available browser, or ask the user to install a version that includes it.",
                vw_vocabulary::texts::CHROME_NAME
            );
        }
    };
    words.to_owned()
}

#[test]
fn a_changed_producer_sentence_still_renders_by_its_id() {
    // An edited sentence no longer selects any wording: it passes through as dynamic text, while the producer's
    // id words the refusal for the agent's browser.
    let changed = "Not run: invalid Chrome tool arguments";
    assert_eq!(render(Browser::Firefox, changed), changed);
    let reason =
        super::super::gateway::validate_public_args(&json!({"_internal": true}), Browser::Firefox)
            .unwrap_err();
    assert_eq!(
        reason,
        "Not run: invalid Firefox tool arguments. Use chrome_status for the action's argument schema."
    );
}

#[test]
fn chrome_agents_read_the_expected_refusals() {
    for id in Fixed::ALL {
        assert_eq!(id.text(Browser::Chrome), chrome_words(id), "{id:?}");
    }
    assert_eq!(
        Fixed::Unavailable.text(Browser::Chrome),
        Browser::Chrome.text(vw_vocabulary::texts::CHROME_UNAVAILABLE)
    );
}

#[test]
fn edge_other_chat_refusal_uses_an_edge_browser_session() {
    assert_eq!(
        Fixed::OtherChat.text(Browser::Edge),
        "Not run: this Edge browser session belongs to another chat. Open an Edge browser session for this chat with chrome_session."
    );
}

#[test]
fn every_id_names_the_agents_own_browser() {
    for id in Fixed::ALL {
        for browser in BROWSERS {
            let text = id.text(browser);
            assert!(
                text.contains(browser.short_name()),
                "{id:?} {browser:?}: {text}"
            );
            if browser != Browser::Chrome {
                assert!(!text.contains("Chrome"), "{id:?} {browser:?}: {text}");
            }
        }
    }
}

#[test]
fn render_matches_no_fixed_refusal_sentence() {
    // Already-worded refusals, in any browser's words, pass the boundary mapping unchanged.
    for id in Fixed::ALL
        .into_iter()
        .filter(|id| *id != Fixed::Unavailable)
    {
        for worded in BROWSERS {
            for agent in BROWSERS {
                let text = id.text(worded);
                assert_eq!(render(agent, &text), text, "{id:?} {worded:?} -> {agent:?}");
            }
        }
    }
}

#[test]
fn session_refusals_tell_each_browser_how_to_recover() {
    for browser in BROWSERS {
        let other = Fixed::OtherChat.text(browser);
        assert!(other.contains("chrome_session"), "{browser:?}: {other}");
        let lost = Fixed::LostConnection.text(browser);
        assert!(lost.contains("chrome_status"), "{browser:?}: {lost}");
        assert!(lost.contains("chrome_session"), "{browser:?}: {lost}");
    }
}

#[test]
fn every_fixed_refusal_has_a_next_step_for_each_browser() {
    for browser in BROWSERS {
        for id in Fixed::ALL {
            let message = id.text(browser);
            let step = match id {
                Fixed::InvalidArguments | Fixed::UnknownAction => "chrome_status",
                Fixed::OpenSessionFirst => "open a",
                Fixed::SelectionFromUser
                    if browser == Browser::Firefox
                        && !super::super::firefox_sharing::PAGE_SHARING_AVAILABLE =>
                {
                    "Use chrome_status"
                }
                Fixed::SelectionFromUser => "Ask the user",
                Fixed::OtherChat | Fixed::Stopped | Fixed::ModeLowered => "chrome_session",
                Fixed::LostConnection => "reconnect",
                Fixed::Unavailable => "Use an available browser",
            };
            assert!(message.contains(step), "{browser:?} {id:?}: {message}");
        }
    }
}
