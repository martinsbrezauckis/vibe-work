//! Portable plan for the `WebView2` protocol mouse variants: option admission, the ordered steps of one sequence and
//! each step's `Input.dispatchMouseEvent` parameters. Called by the Windows sequence adapter; Linux tests cover it.

/// DOM button, click count, modifier bits and an optional wheel delta, as the checked frame input passes them.
pub(super) type Options = (u8, u8, u8, Option<(i32, i32)>);

/// One protocol call of a sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Pointer motion only (hover): no button.
    Move,
    /// One wheel event at the point.
    Wheel,
    /// Pointer motion onto the point before the first press of a button sequence. A person's pointer, and the
    /// left-button path, always arrive before they press: the page's motion handlers run first, so a target that
    /// moves under the pointer is caught by the checked receipt instead of the press landing on a page that never
    /// saw the pointer arrive.
    Approach,
    /// Button press with this click count.
    Press(u8),
    /// Button release with this click count.
    Release(u8),
}

impl Step {
    /// Steps that must pass the live admission checks (deadline, permission, foreground target) first. A release
    /// never does: a held button is always released.
    pub(super) fn admitted(self) -> bool {
        !matches!(self, Self::Release(_))
    }
    /// Steps after which the page may have received agent input.
    pub(super) fn sends(self) -> bool {
        !matches!(self, Self::Approach)
    }
}

/// True for options the adapter accepts: plain motion, a left/middle/right sequence of one to three clicks with
/// modifier bits, or a bounded nonzero wheel without buttons; the CSS point is finite and not negative.
pub(super) fn admissible(options: Options, css: (f64, f64)) -> bool {
    let buttons = (options.0, options.1, options.2) == (0, 0, 0)
        || (options.0 <= 2 && (1..=3).contains(&options.1) && options.2 <= 15);
    let wheel = options.3.is_none_or(|(x, y)| {
        (options.0, options.1, options.2) == (0, 0, 0)
            && (-20_000..=20_000).contains(&x)
            && (-20_000..=20_000).contains(&y)
            && (x, y) != (0, 0)
    });
    buttons
        && wheel
        && [css.0, css.1]
            .into_iter()
            .all(|n| n.is_finite() && n >= 0.0)
}

/// The ordered protocol calls of one sequence.
pub(super) fn steps(options: Options) -> Vec<Step> {
    if options.3.is_some() {
        return vec![Step::Wheel];
    }
    if options.1 == 0 {
        return vec![Step::Move];
    }
    let mut steps = vec![Step::Approach];
    for count in 1..=options.1 {
        steps.extend([Step::Press(count), Step::Release(count)]);
    }
    steps
}

/// `Input.dispatchMouseEvent` parameters for one step. `late`: the press callback outlived the input deadline, so a
/// release goes outside the viewport where a document born meanwhile cannot take a late release or activation.
pub(super) fn event(
    step: Step,
    css: (f64, f64),
    options: Options,
    timestamp: f64,
    late: bool,
) -> serde_json::Value {
    let button = match options.0 {
        1 => "middle",
        2 => "right",
        _ => "left",
    };
    match step {
        Step::Wheel => {
            let (x, y) = options.3.unwrap_or_default();
            serde_json::json!({"type":"mouseWheel","x":css.0,"y":css.1,"deltaX":x,"deltaY":y,"timestamp":timestamp})
        }
        Step::Move | Step::Approach => {
            serde_json::json!({"type":"mouseMoved","x":css.0,"y":css.1,"button":"none","buttons":0,"modifiers":options.2,"timestamp":timestamp})
        }
        Step::Press(count) => {
            serde_json::json!({"type":"mousePressed","x":css.0,"y":css.1,"button":button,"clickCount":count,"modifiers":options.2,"timestamp":timestamp})
        }
        Step::Release(count) => {
            let (x, y) = if late { (-1.0, -1.0) } else { css };
            serde_json::json!({"type":"mouseReleased","x":x,"y":y,"button":button,"clickCount":count,"modifiers":options.2,"timestamp":timestamp})
        }
    }
}

#[cfg(test)]
mod tests {
    //! PREDICT: before the approach step, a right, middle or double click went straight to `mousePressed` at the
    //! point. A page that moves its target when the pointer arrives (a cover on `mousemove`) never saw the pointer
    //! arrive, so the press reached the old target and the act was reported as done with "no visible change",
    //! while the same page under a left click (Win32 `WM_MOUSEMOVE` first) was refused as "the page moved".
    //! `every_button_sequence_moves_onto_its_point_before_the_first_press` fails on that plan.
    use super::*;

    #[test]
    fn every_button_sequence_moves_onto_its_point_before_the_first_press() {
        for (button, count) in [(0, 1), (0, 2), (0, 3), (1, 1), (2, 1)] {
            let options = (button, count, 0, None);
            let plan = steps(options);
            assert_eq!(plan.first(), Some(&Step::Approach), "{options:?}: {plan:?}");
            assert_eq!(plan.get(1), Some(&Step::Press(1)), "{options:?}: {plan:?}");
            assert_eq!(
                plan.iter().filter(|s| **s == Step::Approach).count(),
                1,
                "{plan:?}"
            );
            let moved = event(Step::Approach, (50.0, 40.0), options, 7.0, false);
            assert_eq!(moved["type"], "mouseMoved");
            assert_eq!(
                (moved["x"].as_f64(), moved["y"].as_f64()),
                (Some(50.0), Some(40.0))
            );
            assert_eq!(moved["button"], "none");
            assert_eq!(moved["timestamp"], 7.0);
        }
    }

    #[test]
    fn presses_and_releases_pair_up_with_rising_click_counts() {
        assert_eq!(
            steps((0, 2, 0, None)),
            [
                Step::Approach,
                Step::Press(1),
                Step::Release(1),
                Step::Press(2),
                Step::Release(2)
            ]
        );
        assert_eq!(
            steps((2, 1, 8, None)),
            [Step::Approach, Step::Press(1), Step::Release(1)]
        );
        let press = event(Step::Press(2), (3.0, 4.0), (0, 2, 8, None), 1.0, false);
        assert_eq!(
            (
                press["type"].as_str(),
                press["button"].as_str(),
                press["clickCount"].as_u64(),
                press["modifiers"].as_u64()
            ),
            (Some("mousePressed"), Some("left"), Some(2), Some(8))
        );
        let right = event(Step::Release(1), (3.0, 4.0), (2, 1, 0, None), 1.0, false);
        assert_eq!(right["button"], "right");
        let middle = event(Step::Press(1), (3.0, 4.0), (1, 1, 0, None), 1.0, false);
        assert_eq!(middle["button"], "middle");
    }

    #[test]
    fn motion_and_wheel_stay_single_calls_without_an_approach() {
        assert_eq!(steps((0, 0, 0, None)), [Step::Move]);
        assert_eq!(steps((0, 0, 0, Some((0, 120)))), [Step::Wheel]);
        let wheel = event(
            Step::Wheel,
            (1.0, 2.0),
            (0, 0, 0, Some((5, 120))),
            3.0,
            false,
        );
        assert_eq!(
            (
                wheel["type"].as_str(),
                wheel["deltaX"].as_i64(),
                wheel["deltaY"].as_i64()
            ),
            (Some("mouseWheel"), Some(5), Some(120))
        );
    }

    #[test]
    fn a_late_release_goes_outside_the_viewport() {
        let late = event(Step::Release(1), (50.0, 40.0), (0, 1, 0, None), 1.0, true);
        assert_eq!(
            (late["x"].as_f64(), late["y"].as_f64()),
            (Some(-1.0), Some(-1.0))
        );
        let on_time = event(Step::Release(1), (50.0, 40.0), (0, 1, 0, None), 1.0, false);
        assert_eq!(
            (on_time["x"].as_f64(), on_time["y"].as_f64()),
            (Some(50.0), Some(40.0))
        );
        assert!(
            !Step::Release(1).admitted() && Step::Press(1).admitted() && Step::Approach.admitted()
        );
        assert!(!Step::Approach.sends() && Step::Press(1).sends() && Step::Move.sends());
    }

    #[test]
    fn options_outside_the_adapter_are_not_admitted() {
        assert!(admissible((0, 0, 0, None), (0.0, 0.0)));
        assert!(admissible((2, 1, 15, None), (1.0, 1.0)));
        assert!(admissible((0, 0, 0, Some((0, -120))), (1.0, 1.0)));
        assert!(!admissible((3, 1, 0, None), (1.0, 1.0)));
        assert!(!admissible((0, 4, 0, None), (1.0, 1.0)));
        assert!(!admissible((0, 1, 16, None), (1.0, 1.0)));
        assert!(!admissible((0, 1, 0, Some((0, 1))), (1.0, 1.0)));
        assert!(!admissible((0, 0, 0, Some((0, 0))), (1.0, 1.0)));
        assert!(!admissible((0, 0, 0, Some((20_001, 0))), (1.0, 1.0)));
        assert!(!admissible((0, 1, 0, None), (f64::NAN, 1.0)));
        assert!(!admissible((0, 1, 0, None), (-1.0, 1.0)));
    }
}
