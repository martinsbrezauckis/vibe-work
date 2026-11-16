//! Portable plan for the macOS press and release of a checked click: the `AppKit` button number each button sends.
//! Called by the `AppKit` adapter; Linux tests cover the mapping.
//!
//! `NSEvent`'s mouse-event constructor takes no button number and gives every press number 0, the primary button;
//! `WebKit` reads the number, so an "other" (middle) press made that way reaches the page as `button` 0. The middle
//! press and release therefore carry number 2 explicitly (`kCGMouseButtonCenter`); left and right keep their own
//! event types, which already say which button they are.

/// The `AppKit` / Quartz button number for click button `button` (0 left, 1 middle, 2 right), when it must be set on
/// the event explicitly; `None` when the event type alone says it.
pub(super) const fn explicit_button_number(button: u8) -> Option<i64> {
    match button {
        1 => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // PREDICT (before the fix): there was no explicit number, so a middle press went out with number 0 and the page
    // saw `button` 0 (measured on macOS 27: mousedown/mouseup/auxclick all with button 0, the receipt failed).
    #[test]
    fn a_middle_press_names_the_center_button_and_the_others_need_no_number() {
        assert_eq!(explicit_button_number(1), Some(2));
        assert_eq!(explicit_button_number(0), None);
        assert_eq!(explicit_button_number(2), None);
    }
}
