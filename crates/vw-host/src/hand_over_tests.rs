//! Hand-over arguments, admission checks, wire form and inbox.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{HandOver, HandOverInbox, HandOverText, MAX_ITEMS, MAX_PENDING, MAX_TEXT_BYTES};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn parse(values: &[&str], cwd: &Path) -> Option<HandOver> {
    HandOver::from_args(args(values), cwd, std::io::empty())
}

#[test]
fn a_plain_launch_is_no_hand_over() {
    let dir = scratch();
    assert_eq!(parse(&["vibe-work"], dir.path()), None);
    assert_eq!(parse(&["vibe-work", "--other", "x"], dir.path()), None);
}

#[test]
fn attach_takes_every_remaining_path_and_resolves_relative_ones() {
    let dir = scratch();
    std::fs::write(dir.path().join("one.txt"), "1").unwrap();
    std::fs::write(dir.path().join("two words.md"), "2").unwrap();
    std::fs::create_dir(dir.path().join("folder")).unwrap();
    std::fs::write(dir.path().join("--later"), "3").unwrap();
    std::fs::write(dir.path().join("last.txt"), "4").unwrap();
    let hand_over = parse(
        &[
            "vibe-work",
            "--attach",
            "one.txt",
            "two words.md",
            "folder",
            "--later",
            "last.txt",
        ],
        dir.path(),
    )
    .unwrap();
    assert_eq!(
        hand_over.paths,
        vec![
            dir.path().join("one.txt"),
            dir.path().join("two words.md"),
            dir.path().join("folder"),
            dir.path().join("--later"),
            dir.path().join("last.txt"),
        ]
    );
    assert_eq!(hand_over.refused, Vec::new());
    let (files, folders) = hand_over.split_paths();
    assert_eq!(files.len(), 4);
    assert_eq!(folders, vec![dir.path().join("folder")]);
}

/// Literal file names that look like options. Windows file names cannot end in a dot (NTFS drops trailing dots, so
/// `--attach=..` would be written as `--attach=`): there the dot-ending names do not exist as files and are left out.
/// The Nautilus relative-name path these cases come from (relative-name hand-over) is Linux-only.
const OPTION_LIKE_NAMES: &[&str] = if cfg!(windows) {
    &["--attach-text", "--foo", "--"]
} else {
    &["--attach=..", "--attach=.", "--attach-text", "--foo", "--"]
};

#[test]
fn attach_preserves_option_like_file_names() {
    let dir = scratch();
    for &name in OPTION_LIKE_NAMES {
        std::fs::write(dir.path().join(name), "selected file").unwrap();
        let hand_over = parse(&["vibe-work", "--attach", name], dir.path()).unwrap();
        assert_eq!(hand_over.paths, vec![dir.path().join(name)], "{name}");
        assert_eq!(hand_over.split_paths().1, Vec::<std::path::PathBuf>::new());
        assert!(hand_over.text.is_none());
        assert_eq!(hand_over.refused, Vec::new());
    }
}

#[test]
fn attach_equals_form_and_absolute_paths() {
    let dir = scratch();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "a").unwrap();
    let given = format!("--attach={}", file.display());
    let hand_over = parse(&["vibe-work", &given], Path::new("/elsewhere")).unwrap();
    assert_eq!(hand_over.paths, vec![file]);
}

#[test]
fn paths_that_do_not_exist_are_refused_by_name() {
    let dir = scratch();
    let hand_over = parse(&["vibe-work", "--attach", "missing.pdf"], dir.path()).unwrap();
    assert_eq!(hand_over.paths, Vec::<std::path::PathBuf>::new());
    assert_eq!(hand_over.refused.len(), 1);
    assert!(hand_over.refused[0].name.ends_with("missing.pdf"));
    assert_eq!(hand_over.refused[0].reason, "it does not exist");
}

#[test]
fn text_is_read_from_standard_input_only() {
    let dir = scratch();
    let from_stdin = HandOver::from_args(
        args(&["vibe-work", "--attach-text", "-"]),
        dir.path(),
        "a selected paragraph".as_bytes(),
    )
    .unwrap();
    assert_eq!(
        from_stdin.text,
        Some(HandOverText {
            text: "a selected paragraph".to_owned(),
            from: None,
        })
    );
    // Text given as an argument is never taken: arguments are visible to every program on the computer.
    let in_argv = parse(
        &["vibe-work", "--attach-text", "my secret paragraph"],
        dir.path(),
    )
    .unwrap();
    assert_eq!(in_argv.text, None);
    assert_eq!(in_argv.refused.len(), 1);
    assert!(!format!("{in_argv:?}").contains("my secret paragraph"));
}

#[test]
fn text_over_the_limit_is_refused_and_blank_text_is_dropped() {
    let dir = scratch();
    let long = vec![b'x'; MAX_TEXT_BYTES + 1];
    let hand_over = HandOver::from_args(
        args(&["vibe-work", "--attach-text", "-"]),
        dir.path(),
        long.as_slice(),
    )
    .unwrap();
    assert_eq!(hand_over.text, None);
    assert_eq!(hand_over.refused[0].reason, "it is longer than 1 MB");
    let blank = HandOver::from_args(
        args(&["vibe-work", "--attach-text", "-"]),
        dir.path(),
        "  \n".as_bytes(),
    )
    .unwrap();
    assert!(blank.is_empty());
}

#[test]
fn admission_rechecks_paths_dedupes_and_caps() {
    let dir = scratch();
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "f").unwrap();
    let hand_over = HandOver {
        paths: vec![
            file.clone(),
            file.clone(),
            "relative.txt".into(),
            dir.path().join("gone.txt"),
        ],
        ..HandOver::default()
    }
    .admit();
    assert_eq!(hand_over.paths, vec![file.clone()]);
    let reasons: Vec<&str> = hand_over
        .refused
        .iter()
        .map(|r| r.reason.as_str())
        .collect();
    assert_eq!(reasons, vec!["it is not a full path", "it does not exist"]);

    let many = HandOver {
        paths: (0..MAX_ITEMS + 3)
            .map(|n| {
                let path = dir.path().join(format!("{n}.txt"));
                std::fs::write(&path, "n").unwrap();
                path
            })
            .collect(),
        ..HandOver::default()
    }
    .admit();
    assert_eq!(many.paths.len(), MAX_ITEMS);
    assert_eq!(many.refused.len(), 1);
    assert_eq!(many.refused[0].name, "3 more");
}

#[test]
fn the_wire_form_refuses_anything_but_chips() {
    let hand_over = HandOver {
        paths: vec!["/tmp/a".into()],
        text: Some(HandOverText {
            text: "t".to_owned(),
            from: Some("Mail".to_owned()),
        }),
        refused: Vec::new(),
    };
    let bytes = hand_over.encode().unwrap();
    assert_eq!(HandOver::decode(&bytes), Ok(hand_over));
    // Falsifier: a hand-over that would start a turn (or type, or choose the chat) is not a hand-over.
    for extra in [
        r#"{"paths":[],"send":true}"#,
        r#"{"text":{"text":"x"},"prompt":"do it"}"#,
        r#"{"sessionId":"s1"}"#,
        r#"{"text":{"text":"x","autoSend":true}}"#,
    ] {
        assert!(HandOver::decode(extra.as_bytes()).is_err(), "{extra}");
    }
}

#[test]
fn the_inbox_keeps_hand_overs_until_the_window_takes_them() {
    let inbox = HandOverInbox::default();
    let file = HandOver {
        paths: vec!["/a".into()],
        ..HandOver::default()
    };
    inbox.push(HandOver::default());
    inbox.push(file.clone());
    // Subscribing after a launch hand-over tells the window at once.
    let told = Arc::new(AtomicUsize::new(0));
    let counter = told.clone();
    inbox.subscribe(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        true
    });
    assert_eq!(told.load(Ordering::SeqCst), 1);
    inbox.push(file.clone());
    assert_eq!(told.load(Ordering::SeqCst), 2);
    assert_eq!(inbox.take(), vec![file.clone(), file.clone()]);
    assert_eq!(inbox.take(), Vec::new());
    for _ in 0..MAX_PENDING + 5 {
        inbox.push(file.clone());
    }
    assert_eq!(inbox.take().len(), MAX_PENDING);
}

#[test]
fn a_gone_subscriber_is_dropped() {
    let inbox = HandOverInbox::default();
    let told = Arc::new(AtomicUsize::new(0));
    let counter = told.clone();
    inbox.subscribe(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        false
    });
    let file = HandOver {
        paths: vec!["/a".into()],
        ..HandOver::default()
    };
    inbox.push(file.clone());
    inbox.push(file);
    assert_eq!(told.load(Ordering::SeqCst), 1);
}
