//! Unavailable browser outcomes keep UI labels separate from actionable agent instructions.
//! Exercises all browser presentations and fixed text translation without installing an extension.
use super::{AddonState, addon, agent_browser::Browser, browser_text};

#[tokio::test]
async fn unavailable_browser_instructions_are_identical_across_producers() {
    let core = crate::offline_core::start("browser-unavailable-copy").await;
    for browser in [
        Browser::Chrome,
        Browser::Edge,
        Browser::Firefox,
        Browser::Safari,
    ] {
        let expected = browser.words("{yourBrowser} is not available in this version of Vibe Work. Use an available browser, or ask the user to install a version that includes it.");
        let view = browser.addon(&core.shared);
        assert_eq!(view.state, AddonState::Unavailable);
        assert_eq!(view.instruction, expected);
        assert_eq!(
            addon::view_for(AddonState::Unavailable, "Unavailable", browser).instruction,
            expected
        );
        assert_eq!(
            browser_text::render(browser, vw_vocabulary::texts::CHROME_UNAVAILABLE),
            expected
        );
        assert!(!view.label.contains("Ask the user") && !view.label.contains("Use an available"));
    }
    core.shutdown().await;
}

#[test]
fn broken_browser_packages_keep_their_reason_and_user_repair_step() {
    for browser in [
        Browser::Chrome,
        Browser::Edge,
        Browser::Firefox,
        Browser::Safari,
    ] {
        for (label, expected) in [
            (
                vw_vocabulary::texts::CHROME_PACKAGE_MISSING,
                "This installation is missing a file {yourBrowser} needs. Ask the user to reinstall Vibe Work.",
            ),
            (
                vw_vocabulary::texts::CHROME_PACKAGE_CHANGED,
                "A file {yourBrowser} needs differs from the one shipped with Vibe Work. Ask the user to reinstall Vibe Work.",
            ),
        ] {
            let view = addon::view_for(AddonState::Unavailable, label, browser);
            assert_eq!(view.instruction, browser.words(expected));
            assert_eq!(browser_text::render(browser, label), view.instruction);
            assert!(!view.label.contains("Ask the user"));
        }
    }
}
