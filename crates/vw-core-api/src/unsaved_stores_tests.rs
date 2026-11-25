use super::{Verdict, read, verdict, write};
use crate::error::{CoreError, ErrorCode};

fn server_error(code: &str) -> CoreError {
    let mut error = CoreError::new(ErrorCode::ServerError, "refused");
    error.server_code = Some(code.to_owned());
    error
}

#[test]
fn only_a_not_found_answer_marks_a_chat_unsaved() {
    assert_eq!(verdict(Ok(())), Verdict::Saved);
    assert_eq!(verdict(Err(server_error("not_found"))), Verdict::Unsaved);
    assert_eq!(verdict(Err(server_error("stale_turn"))), Verdict::Unknown);
    assert_eq!(
        verdict(Err(CoreError::new(ErrorCode::Internal, "no server"))),
        Verdict::Unknown
    );
}

#[test]
fn the_record_round_trips_and_an_empty_list_removes_the_file() {
    let dir = std::env::temp_dir().join(format!("vw-unsaved-stores-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("started-sessions.json");
    assert!(read(&file).is_empty(), "a missing record reads as empty");
    write(&file, &["a".to_owned(), "b".to_owned()]);
    assert_eq!(read(&file), ["a", "b"]);
    std::fs::write(&file, b"not json").unwrap();
    assert!(read(&file).is_empty(), "a damaged record reads as empty");
    write(&file, &[]);
    assert!(!file.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
