//! Inline handbook input contracts exercised without a window. Depends on vw-pages; run by scoped Cargo tests.
//! Falsifier: the former 50-input limit truncates a 250-input board and loses its last stored answer.

use std::fmt::Write as _;
use vw_pages::{AnswerStore, AnswerValue, PagePath, page_answers, scan_html};

#[test]
fn a_large_board_keeps_every_input_and_its_last_answer() {
    let mut html = String::new();
    for i in 0..250 {
        write!(html, "<div class=\"hb-input\" id=\"note-{i}\" data-input=\"text\"><p class=\"hb-question\">Note {i}</p></div>").unwrap();
    }
    let scan = scan_html(&html);
    assert_eq!(scan.facts.inputs.len(), 250);
    assert_eq!(scan.detail.input_warnings, Vec::<String>::new());
    let dir = std::env::temp_dir().join(format!("vw-inline-inputs-{}", std::process::id()));
    let store = AnswerStore::new(dir.clone());
    let project = dir.join("project");
    let page = PagePath::parse("roadmap.html").unwrap();
    store
        .set(
            &project,
            &page,
            &scan.facts,
            "note-249",
            &AnswerValue::Text {
                text: "Keep the last card".into(),
            },
        )
        .unwrap();
    let answers = page_answers(&page, Some(&scan.facts), store.list(&project, Some(&page)));
    assert_eq!(answers.inputs.len(), 250);
    assert!(answers.inputs[249].answer.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn oversized_boards_still_have_a_bounded_input_count() {
    let mut html = String::new();
    for i in 0..501 {
        write!(html, "<div class=\"hb-input\" id=\"n-{i}\" data-input=\"text\"><p class=\"hb-question\">Note</p></div>").unwrap();
    }
    let scan = scan_html(&html);
    assert_eq!(scan.facts.inputs.len(), 500);
    assert!(
        scan.detail
            .input_warnings
            .iter()
            .any(|w| w.contains("more than 500 inputs"))
    );
}
