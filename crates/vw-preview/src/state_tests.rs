//! Refusal bookkeeping of one preview entry: what the window shows and what action receipts read.
use super::Entry;

/// A refused third-party page belongs to the page that asked for it: once another page starts loading, the
/// preview no longer shows "Not opened: ..." for it. Receipts still count the refusal (their lifetime mark).
#[test]
fn a_new_page_load_drops_the_previous_pages_refusal_from_the_view() {
    let mut entry = Entry::new("a");
    entry.note_refused("https://third.example/login", "not allowed from this page");
    assert_eq!(
        entry.view(true).refused,
        vec!["https://third.example/login".to_owned()]
    );
    entry.reset_console();
    assert_eq!(entry.view(true).refused, Vec::<String>::new());
    assert_eq!(entry.refusals_seen, 1, "receipts keep their lifetime mark");
    // A refusal of the new page shows again.
    entry.note_refused("https://other.example/", "not allowed from this page");
    assert_eq!(
        entry.view(true).refused,
        vec!["https://other.example/".to_owned()]
    );
}
