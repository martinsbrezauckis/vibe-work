//! Which of the person's input a browser view swallows while the agent controls it (the input block of the
//! agent control frame). The rule is the same on every platform; each platform's watch maps its native events to
//! [`Input`] and drops an event this rule swallows before the page sees it.
//!
//! - While blocked: pointer presses, pointer motion, wheel and touch, and key presses other than Tab / Shift+Tab are
//!   swallowed. Tab passes so the keyboard can still leave the page (the frame's Pause and Stop sit in the app,
//!   outside the page). System and app shortcuts pass too (Alt or the Windows key on Windows, Command on macOS): the
//!   person keeps switching windows and using the app's own shortcuts while the agent works.
//! - A release goes the way of its press: the release of a swallowed press is swallowed even after the block ended
//!   (the page never saw the press), and the release of a press the page saw passes even when the block started in
//!   between (the page must not keep a button or key held).
//! - The engine's own input (the agent's native clicks and keys) never reaches this rule: each platform leaves it out
//!   before mapping, the same way its pointer watch does.
//! - A swallowed press is not the person's input for the checked clicks: the page never received it.
//! - A swallowed press is reported ([`Refused`], [`refused_press`]): the press reaches no view, so the app window's
//!   own keyboard focus stays where it was (an answer of an ask card, say), and only this report tells the app that
//!   the person meant the page.
//!
//! Depends on: std only. Called by: the platform watches in `person` (Linux, macOS, Windows).

use std::{collections::HashMap, sync::Arc};

/// Answers whether the view is blocked right now. Called on the view's input thread for every event, so it must
/// be cheap and never block.
pub type Blocked = Arc<dyn Fn() -> bool + Send + Sync>;

/// A never-blocked view (internal viewers and checkers have no agent control frame).
#[must_use]
pub fn never() -> Blocked {
    Arc::new(|| false)
}

/// Told of each of the person's presses a blocked view swallowed. Called on the view's input thread (a hook thread on
/// Windows), so it must be cheap and never block.
pub type Refused = Arc<dyn Fn() + Send + Sync>;

/// Whether `input`, which the rule swallowed (`swallowed`), is a press of the person's to report through [`Refused`].
#[must_use]
pub fn refused_press(swallowed: bool, input: Input) -> bool {
    swallowed && input == Input::Press
}

/// One native event of the person's, as the block rule sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// A pointer button press or a touch beginning.
    Press,
    /// A pointer button release or a touch ending.
    Release,
    /// Pointer or touch motion over the view.
    Motion,
    /// A wheel, scroll or touchpad gesture step.
    Wheel,
    /// A key press or release.
    Key {
        /// The platform's key code, unchanged between a press and its release.
        code: u32,
        /// Tab or Shift+Tab.
        tab: bool,
        /// A system or app shortcut (Alt / Windows key on Windows, Command on macOS).
        shortcut: bool,
        /// A press (`false`: a release).
        press: bool,
    },
    /// Anything else (focus, crossing, configure): never swallowed.
    Other,
}

/// What one view remembers between events: whether the page saw the current pointer press and key presses.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Held {
    pointer_swallowed: bool,
    keys: HashMap<u32, bool>,
}

/// Whether to swallow `input` given whether the view is blocked now; updates `held` for the matching release.
#[must_use]
pub fn swallow(blocked: bool, input: Input, held: &mut Held) -> bool {
    match input {
        Input::Press => {
            held.pointer_swallowed = blocked;
            blocked
        }
        Input::Release => std::mem::take(&mut held.pointer_swallowed),
        Input::Motion => blocked || held.pointer_swallowed,
        Input::Wheel => blocked,
        Input::Key {
            code,
            tab,
            shortcut,
            press: true,
        } => {
            let swallowed = blocked && !tab && !shortcut;
            let original = held.keys.entry(code).or_insert(swallowed);
            swallowed || *original
        }
        Input::Key {
            code,
            tab,
            shortcut,
            press: false,
        } => held
            .keys
            .remove(&code)
            .unwrap_or(blocked && !tab && !shortcut),
        Input::Other => false,
    }
}

#[cfg(test)]
mod tests {
    //! PREDICT: before the rule exists every person's event reaches the page while the agent works (the watches only
    //! observe). These tests pin the contract every platform maps onto.
    use super::{Held, Input, refused_press, swallow};

    fn key(press: bool) -> Input {
        key_code(1, press)
    }

    fn key_code(code: u32, press: bool) -> Input {
        Input::Key {
            code,
            tab: false,
            shortcut: false,
            press,
        }
    }

    #[test]
    fn a_blocked_view_swallows_pointer_wheel_motion_and_keys() {
        let mut held = Held::default();
        for input in [
            Input::Press,
            Input::Motion,
            Input::Release,
            Input::Wheel,
            key(true),
            key(false),
        ] {
            assert!(
                swallow(true, input, &mut held),
                "{input:?} must be swallowed"
            );
        }
    }

    #[test]
    fn an_unblocked_view_passes_everything() {
        let mut held = Held::default();
        for input in [
            Input::Press,
            Input::Motion,
            Input::Release,
            Input::Wheel,
            key(true),
            key(false),
        ] {
            assert!(!swallow(false, input, &mut held), "{input:?} must pass");
        }
    }

    #[test]
    fn tab_shortcuts_and_other_events_always_pass() {
        let mut held = Held::default();
        for press in [true, false] {
            for (tab, shortcut) in [(true, false), (false, true)] {
                assert!(!swallow(
                    true,
                    Input::Key {
                        code: 1,
                        tab,
                        shortcut,
                        press
                    },
                    &mut held
                ));
            }
        }
        assert!(!swallow(true, Input::Other, &mut held));
    }

    #[test]
    fn a_release_goes_the_way_of_its_press() {
        // Press swallowed, block ends (Pause): the release is still swallowed, then the next one passes.
        let mut held = Held::default();
        assert!(swallow(true, Input::Press, &mut held));
        assert!(
            swallow(false, Input::Motion, &mut held),
            "a drag of a swallowed press stays swallowed"
        );
        assert!(swallow(false, Input::Release, &mut held));
        assert!(!swallow(false, Input::Release, &mut held));
        // Press seen by the page, block starts: its release passes so no button stays held in the page.
        let mut held = Held::default();
        assert!(!swallow(false, Input::Press, &mut held));
        assert!(!swallow(true, Input::Release, &mut held));
    }

    #[test]
    fn only_a_swallowed_press_is_reported() {
        // PREDICT: without the report the app never learns of a press on the blocked page (the press reaches no view
        // and the window's own focus does not move), so keys typed next still answer the app's focused ask card.
        let mut held = Held::default();
        let press = swallow(true, Input::Press, &mut held);
        assert!(refused_press(press, Input::Press));
        for input in [
            Input::Motion,
            Input::Release,
            Input::Wheel,
            key(true),
            key(false),
        ] {
            let swallowed = swallow(true, input, &mut held);
            assert!(!refused_press(swallowed, input), "{input:?} is no press");
        }
        let mut held = Held::default();
        let passed = swallow(false, Input::Press, &mut held);
        assert!(
            !refused_press(passed, Input::Press),
            "a press the page saw is not refused"
        );
    }

    #[test]
    fn a_swallowed_key_press_keeps_its_release_swallowed_after_the_block() {
        let mut held = Held::default();
        assert!(swallow(true, key(true), &mut held));
        assert!(swallow(false, key(false), &mut held));
        assert!(!swallow(false, key(false), &mut held));
    }

    #[test]
    fn a_key_pressed_before_the_block_keeps_its_release() {
        let mut held = Held::default();
        assert!(!swallow(false, key(true), &mut held));
        assert!(
            !swallow(true, key(false), &mut held),
            "the page must see the release of a press it saw before Working"
        );
    }

    #[test]
    fn simultaneous_keys_keep_their_own_release_dispositions() {
        let mut held = Held::default();
        assert!(!swallow(false, key(true), &mut held));
        assert!(swallow(true, key_code(2, true), &mut held));
        assert!(swallow(false, key_code(2, false), &mut held));
        assert!(!swallow(true, key(false), &mut held));
    }

    #[test]
    fn repeated_keydown_does_not_change_where_the_release_goes() {
        let mut held = Held::default();
        assert!(!swallow(false, key(true), &mut held));
        assert!(swallow(true, key(true), &mut held));
        assert!(!swallow(true, key(false), &mut held));

        assert!(swallow(true, key(true), &mut held));
        assert!(swallow(false, key(true), &mut held));
        assert!(swallow(false, key(false), &mut held));
    }

    #[test]
    fn a_release_without_a_recorded_press_uses_the_current_block() {
        let mut held = Held::default();
        assert!(!swallow(false, key(false), &mut held));
        assert!(swallow(true, key(false), &mut held));
    }

    #[test]
    fn two_views_keep_their_key_releases_independent() {
        let (mut first, mut second) = (Held::default(), Held::default());
        assert!(!swallow(false, key(true), &mut first));
        assert!(swallow(true, key(true), &mut second));
        assert!(swallow(false, key(false), &mut second));
        assert!(!swallow(true, key(false), &mut first));
    }
}
