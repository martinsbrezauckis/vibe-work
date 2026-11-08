//! Visibility policy for a browser child parked outside the window while a chrome menu is open.
//! Called by platform parking and repeated engine layout; macOS must retain the page's selection.

pub(crate) fn cover<E>(macos: bool, hide: impl FnOnce() -> Result<(), E>) -> Result<(), E> {
    if macos { Ok(()) } else { hide() }
}

#[cfg(test)]
mod tests {
    use super::cover;
    use std::cell::Cell;

    #[test]
    fn a_parked_macos_page_keeps_its_selection_during_cover_and_repeated_layout() {
        let selection = Cell::new(Some("selected page text"));
        for _ in 0..3 {
            cover(true, || {
                // Hiding a WKWebView can clear its selection. Parking removes its pixels instead.
                selection.set(None);
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        assert_eq!(selection.get(), Some("selected page text"));
    }

    #[test]
    fn other_platforms_keep_their_existing_hide_and_error_behavior() {
        let hidden = Cell::new(false);
        cover(false, || {
            hidden.set(true);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(hidden.get());
        assert_eq!(
            cover(false, || Err::<(), _>("hide failed")),
            Err("hide failed")
        );
    }
}
