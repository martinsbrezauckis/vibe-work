//! Changes keep and revert actions preserve disk content and detect stale views.

mod changes;

use changes::{TempRepo, file, lines};
use vw_changes::{ChangesError, Target, keep, refresh, revert};

fn two_hunk_edit(repo: &TempRepo) -> String {
    let edited = lines(30)
        .replace("line 2\n", "line 2 changed\n")
        .replace("line 25\n", "line 25 changed\n");
    repo.write("a.txt", &edited);
    edited
}

#[test]
fn keep_one_hunk_leaves_the_other_and_the_file() {
    let repo = TempRepo::with_files(&[("a.txt", &lines(30))]);
    let mut baseline = repo.baseline("keep");
    let edited = two_hunk_edit(&repo);
    let diff = refresh(&baseline).unwrap();
    let a = file(&diff, "a.txt");
    let (first, second) = (a.hunks[0].id.clone(), a.hunks[1].clone());
    let after = keep(
        &mut baseline,
        &Target::Hunk {
            path: "a.txt".into(),
            blob: a.blob.clone(),
            hunk_id: first,
        },
    )
    .unwrap();
    let a = file(&after, "a.txt");
    assert_eq!(a.hunks.len(), 1);
    assert_eq!(
        a.hunks[0].lines, second.lines,
        "the remaining hunk is the second one"
    );
    assert_eq!(repo.read("a.txt"), edited, "keep changed the file");
}

#[test]
fn revert_one_hunk_restores_only_those_lines() {
    let repo = TempRepo::with_files(&[("a.txt", &lines(30))]);
    let baseline = repo.baseline("revert");
    two_hunk_edit(&repo);
    let diff = refresh(&baseline).unwrap();
    let a = file(&diff, "a.txt");
    let after = revert(
        &baseline,
        &Target::Hunk {
            path: "a.txt".into(),
            blob: a.blob.clone(),
            hunk_id: a.hunks[0].id.clone(),
        },
        &[],
    )
    .unwrap();
    assert_eq!(
        repo.read("a.txt"),
        lines(30).replace("line 25\n", "line 25 changed\n")
    );
    assert_eq!(file(&after, "a.txt").hunks.len(), 1);
}

#[test]
fn an_action_on_a_file_that_changed_since_the_diff_is_refused() {
    let repo = TempRepo::with_files(&[("a.txt", &lines(30))]);
    let mut baseline = repo.baseline("stale");
    two_hunk_edit(&repo);
    let diff = refresh(&baseline).unwrap();
    let a = file(&diff, "a.txt").clone();
    repo.write("a.txt", &format!("{}extra\n", repo.read("a.txt")));
    let on_disk = repo.read("a.txt");
    let hunk = Target::Hunk {
        path: "a.txt".into(),
        blob: a.blob.clone(),
        hunk_id: a.hunks[0].id.clone(),
    };
    for result in [
        revert(&baseline, &hunk, &[]),
        keep(&mut baseline, &hunk),
        revert(
            &baseline,
            &Target::File {
                path: "a.txt".into(),
                blob: a.blob.clone(),
            },
            &[],
        ),
        revert(
            &baseline,
            &Target::All {
                fingerprint: diff.fingerprint.clone(),
            },
            &[],
        ),
    ] {
        assert!(
            matches!(result, Err(ChangesError::Stale { .. })),
            "{result:?}"
        );
    }
    assert_eq!(
        repo.read("a.txt"),
        on_disk,
        "a refused action changed the file"
    );
}

#[test]
fn bulk_revert_names_new_files_and_keep_all_empties_the_diff() {
    let repo = TempRepo::with_files(&[("a.txt", &lines(30)), ("b.txt", "b\n")]);
    let baseline = repo.baseline("bulk");
    two_hunk_edit(&repo);
    repo.write("new/created.txt", "made by the agent\n");
    let diff = refresh(&baseline).unwrap();
    let all = Target::All {
        fingerprint: diff.fingerprint.clone(),
    };
    match revert(&baseline, &all, &[]) {
        Err(ChangesError::ConfirmDeletes { paths }) => {
            assert_eq!(paths, vec!["new/created.txt".to_owned()])
        }
        other => panic!("expected ConfirmDeletes, got {other:?}"),
    }
    assert!(
        repo.path("new/created.txt").exists(),
        "a refused revert deleted the file"
    );
    let file_target = Target::File {
        path: "new/created.txt".into(),
        blob: file(&diff, "new/created.txt").blob.clone(),
    };
    assert!(matches!(
        revert(&baseline, &file_target, &[]),
        Err(ChangesError::ConfirmDeletes { .. })
    ));
    let after = revert(&baseline, &all, &["new/created.txt".to_owned()]).unwrap();
    assert!(after.files.is_empty(), "{:?}", after.files);
    assert_eq!(repo.read("a.txt"), lines(30));
    assert!(!repo.path("new/created.txt").exists());

    let mut baseline = repo.baseline("bulk-keep");
    two_hunk_edit(&repo);
    let diff = refresh(&baseline).unwrap();
    let after = keep(
        &mut baseline,
        &Target::All {
            fingerprint: diff.fingerprint,
        },
    )
    .unwrap();
    assert!(after.files.is_empty());
}

#[test]
fn crlf_files_diff_by_line_and_keep_their_line_endings() {
    let crlf = |text: &str| text.replace('\n', "\r\n");
    let repo = TempRepo::with_files(&[(".gitattributes", "*.txt text eol=crlf\n")]);
    repo.write("w.txt", &crlf(&lines(20)));
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "crlf"]);
    let baseline = repo.baseline("crlf");
    repo.write(
        "w.txt",
        &crlf(
            &lines(20)
                .replace("line 4\n", "line four\n")
                .replace("line 17\n", "line seventeen\n"),
        ),
    );
    let diff = refresh(&baseline).unwrap();
    let w = file(&diff, "w.txt");
    assert_eq!(
        (w.added, w.removed),
        (2, 2),
        "line endings made a whole-file diff"
    );
    assert!(w.hunks[0].lines.iter().all(|l| !l.text.ends_with('\r')));
    revert(
        &baseline,
        &Target::Hunk {
            path: "w.txt".into(),
            blob: w.blob.clone(),
            hunk_id: w.hunks[0].id.clone(),
        },
        &[],
    )
    .unwrap();
    let bytes = repo.read_bytes("w.txt");
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(
        text,
        crlf(&lines(20).replace("line 17\n", "line seventeen\n"))
    );
}

#[test]
fn hunks_of_new_or_binary_files_are_file_level_only() {
    let repo = TempRepo::with_files(&[("img.bin", "\0old")]);
    let baseline = repo.baseline("refuse");
    repo.write_bytes("img.bin", b"\0new");
    repo.write("n.txt", "x\n");
    let diff = refresh(&baseline).unwrap();
    let n = file(&diff, "n.txt");
    let result = revert(
        &baseline,
        &Target::Hunk {
            path: "n.txt".into(),
            blob: n.blob.clone(),
            hunk_id: n.hunks[0].id.clone(),
        },
        &[],
    );
    assert!(
        matches!(result, Err(ChangesError::Refused(_))),
        "{result:?}"
    );
    let img = file(&diff, "img.bin");
    revert(
        &baseline,
        &Target::File {
            path: "img.bin".into(),
            blob: img.blob.clone(),
        },
        &[],
    )
    .unwrap();
    assert_eq!(repo.read_bytes("img.bin"), b"\0old");
}
