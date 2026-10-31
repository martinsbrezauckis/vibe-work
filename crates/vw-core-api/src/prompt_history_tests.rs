//! Shared-history format, append and concurrent writer tests over isolated files.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct History(std::path::PathBuf);
impl History {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "vw-prompt-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p.join("vibehistory"))
    }
}
impl Drop for History {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
#[test]
fn reads_cli_json_legacy_multiline_and_latest_hundred() {
    let h = History::new();
    assert_eq!(read(&h.0).unwrap(), Vec::<String>::new());
    std::fs::write(&h.0, "Hello Vibe!\n\n\"two\\nlines ā\"\nnot json\n").unwrap();
    assert_eq!(
        read(&h.0).unwrap(),
        ["Hello Vibe!", "two\nlines ā", "not json"]
    );
    let mut body = String::new();
    for n in 0..150 {
        use std::fmt::Write as _;
        writeln!(body, "\"prompt {n}\"").unwrap();
    }
    std::fs::write(&h.0, body).unwrap();
    let entries = read(&h.0).unwrap();
    assert_eq!(entries.len(), 100);
    assert_eq!(entries[0], "prompt 50");
    assert_eq!(entries[99], "prompt 149");
}
#[test]
fn appends_cli_records_without_rewriting_or_truncating_user_bytes() {
    let h = History::new();
    std::fs::write(&h.0, "legacy without newline").unwrap();
    append(&h.0, "  two\nlines \"ā\"  ", &|_| true).unwrap();
    append(&h.0, "  two\nlines \"ā\"", &|_| true).unwrap();
    append(&h.0, "   ", &|_| true).unwrap();
    assert_eq!(
        std::fs::read_to_string(&h.0).unwrap(),
        "legacy without newline\n\"  two\\nlines \\\"ā\\\"\"\n"
    );
    let old = std::fs::read(&h.0).unwrap();
    for n in 0..110 {
        append(&h.0, &format!("next {n}"), &|_| true).unwrap();
    }
    assert!(std::fs::read(&h.0).unwrap().starts_with(&old));
    assert_eq!(read(&h.0).unwrap().len(), 100);
}
#[test]
fn vault_detection_skips_the_entire_prompt_before_creating_a_file() {
    let h = History::new();
    append(&h.0, "send synthetic-secret-value", &|s| {
        !s.contains("synthetic-secret-value")
    })
    .unwrap();
    assert!(!h.0.exists());
    append(&h.0, "safe", &|_| true).unwrap();
    let old = std::fs::read(&h.0).unwrap();
    append(&h.0, "send synthetic-secret-value", &|s| {
        !s.contains("synthetic-secret-value")
    })
    .unwrap();
    assert_eq!(std::fs::read(&h.0).unwrap(), old);
}
#[test]
fn concurrent_appends_are_complete_json_records() {
    let h = History::new();
    // The file exists first: on macOS concurrent first creations through a folder handle can fail (0.1.5 retries).
    std::fs::write(&h.0, "").unwrap();
    std::thread::scope(|scope| {
        for n in 0..20 {
            let p = &h.0;
            scope.spawn(move || append(p, &format!("entry {n}\nsecond line"), &|_| true).unwrap());
        }
    });
    let bytes = std::fs::read_to_string(&h.0).unwrap();
    assert_eq!(bytes.lines().count(), 20);
    for line in bytes.lines() {
        assert!(
            serde_json::from_str::<String>(line)
                .unwrap()
                .ends_with("\nsecond line")
        );
    }
}
#[cfg(unix)]
#[test]
fn history_links_are_refused_and_new_files_are_private() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let h = History::new();
    let outside = h.0.with_file_name("outside");
    std::fs::write(&outside, "keep").unwrap();
    symlink(&outside, &h.0).unwrap();
    assert!(append(&h.0, "new", &|_| true).is_err());
    assert!(read(&h.0).is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
    std::fs::remove_file(&h.0).unwrap();
    append(&h.0, "new", &|_| true).unwrap();
    assert_eq!(
        std::fs::metadata(&h.0).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn preserves_leading_spaces_that_make_cli_mode_characters_literal() {
    let h = History::new();
    append(&h.0, "  !echo example  ", &|_| true).unwrap();
    append(&h.0, "  !echo example", &|_| true).unwrap();
    assert_eq!(read(&h.0).unwrap(), ["  !echo example"]);
    assert_eq!(
        std::fs::read_to_string(&h.0).unwrap(),
        "\"  !echo example\"\n"
    );
}
